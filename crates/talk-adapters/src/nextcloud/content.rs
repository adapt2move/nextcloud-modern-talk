//! Fresh room/content authorization and bounded native projection. No URL from a
//! rich object is executed: file links and image routes are constructed by the API.
use super::*;
use nextcloud_talk_api as api;
use talk_core::protocol::content::*;
use talk_core::protocol::conversation::ActionAvailability;
use talk_core::protocol::{FeatureSupport, Role, RoomKind};

pub(super) fn is_mutation(command: &ContentCommand) -> bool {
    matches!(
        command,
        ContentCommand::DownloadCloudFile { .. }
            | ContentCommand::Poll {
                command: PollCommand::Create(_)
                    | PollCommand::SaveDraft(_)
                    | PollCommand::UpdateDraft { .. }
                    | PollCommand::DeleteDraft(_)
                    | PollCommand::Vote { .. }
                    | PollCommand::Close(_),
                ..
            }
    )
}
fn poll_availability(room: &Conversation, caps: &Capabilities, create: bool) -> ActionAvailability {
    ActionAvailability {
        implemented: true,
        server: caps.feature("talk-polls"),
        allowed: if room.unknown_restrictions {
            Decision::Unknown
        } else if create && !matches!(room.kind, RoomKind::Group | RoomKind::Public) {
            Decision::Denied
        } else if create {
            room.actions.send_message
        } else {
            room.actions.read_messages
        },
    }
}
impl NextcloudClient {
    async fn content_room(
        &mut self,
        c: &OperationContext,
        id: ConversationId,
        caps: &Capabilities,
    ) -> Result<Conversation, PortError> {
        let data = self.room_data(c, id).await?;
        let room = wire::room(&data, id, caps).map_err(|k| self.error(k))?;
        self.allowed(room.actions.read_messages)?;
        if room.unknown_restrictions {
            return Err(self.error(ErrorKind::Unsupported));
        }
        Ok(room)
    }
    async fn attachment(
        &mut self,
        c: &OperationContext,
        message: MessageId,
        placeholder: &Text<128>,
        caps: &Capabilities,
    ) -> Result<api::chat::RichObject, PortError> {
        Ok(self
            .attachment_with_kind(c, message, placeholder, caps)
            .await?
            .1)
    }
    /// The attachment with the message type that carries it, from one message read.
    async fn attachment_with_kind(
        &mut self,
        c: &OperationContext,
        message: MessageId,
        placeholder: &Text<128>,
        caps: &Capabilities,
    ) -> Result<(Option<String>, api::chat::RichObject), PortError> {
        let room = self.content_room(c, message.conversation, caps).await?;
        if room.classified || room.sensitive {
            return Err(self.error(ErrorKind::PermissionDenied));
        }
        let target = self.target(c, message).await?;
        let kind = target.kind.clone();
        let object = target
            .rich
            .into_iter()
            .find(|o| o.kind == "file" && o.placeholder == placeholder.as_str())
            .ok_or_else(|| self.error(ErrorKind::NotFound))?;
        Ok((kind, object))
    }
    pub(super) async fn content(
        &mut self,
        c: &OperationContext,
        operation: OperationId,
        command: ContentCommand,
        caps: &Capabilities,
    ) -> Result<ContentResult, PortError> {
        let revalidate = matches!(&command, ContentCommand::RevalidateAvatar(_));
        match command {
            ContentCommand::DownloadCloudFile {
                path,
                destination,
                progress,
            } => {
                let path = api::files::FilePath::parse(path.as_str())
                    .map_err(|kind| self.error(transport::kind(kind)))?;
                let bytes = self
                    .download_to_file(c, path, destination.as_str(), &progress)
                    .await?;
                Ok(ContentResult::CloudFileSaved(bytes))
            }
            ContentCommand::OpenCloudFile(file) => {
                let link = self.transport.file_link(file)?;
                Ok(ContentResult::CloudLink(
                    Text::new(&link).map_err(|_| self.error(ErrorKind::TooLarge))?,
                ))
            }
            ContentCommand::ListFiles { .. }
            | ContentCommand::ListFilesPage { .. }
            | ContentCommand::SearchFiles { .. }
            | ContentCommand::FavoriteFiles => {
                let (entries, pagination, folder_page) = match command {
                    ContentCommand::ListFiles { path } => {
                        let path = api::files::DirectoryPath::parse(path.as_str())
                            .map_err(|kind| self.error(transport::kind(kind)))?;
                        (self.transport.list_files(c, path).await?, None, false)
                    }
                    ContentCommand::ListFilesPage { path, offset } => {
                        let path = api::files::DirectoryPath::parse(path.as_str())
                            .map_err(|kind| self.error(transport::kind(kind)))?;
                        let page = self.transport.list_files_page(c, path, offset).await?;
                        (page.entries, page.pagination, true)
                    }
                    ContentCommand::SearchFiles { query } => (
                        self.transport
                            .search_files(c, query.as_ref().map(Text::as_str))
                            .await?,
                        None,
                        false,
                    ),
                    ContentCommand::FavoriteFiles => {
                        (self.transport.favorite_files(c).await?, None, false)
                    }
                    _ => unreachable!(),
                };
                let mut files = Page::empty();
                for entry in entries {
                    files
                        .try_push(CloudFile {
                            name: Text::new(&entry.name)
                                .map_err(|_| self.error(ErrorKind::TooLarge))?,
                            path: Text::new(&entry.path)
                                .map_err(|_| self.error(ErrorKind::TooLarge))?,
                            directory: entry.is_directory,
                            size: entry.size,
                            modified: entry
                                .modified
                                .as_deref()
                                .map(Text::new)
                                .transpose()
                                .map_err(|_| self.error(ErrorKind::TooLarge))?,
                            modified_unix: entry.modified_unix,
                            mime: entry
                                .content_type
                                .as_deref()
                                .map(Text::new)
                                .transpose()
                                .map_err(|_| self.error(ErrorKind::TooLarge))?,
                            file_id: entry
                                .file_id
                                .as_deref()
                                .map(Text::new)
                                .transpose()
                                .map_err(|_| self.error(ErrorKind::TooLarge))?,
                        })
                        .map_err(|_| self.error(ErrorKind::TooLarge))?;
                }
                if folder_page {
                    Ok(ContentResult::FilePage { files, pagination })
                } else {
                    Ok(ContentResult::Files(files))
                }
            }
            ContentCommand::VoiceAudio {
                message,
                placeholder,
            } => {
                let (kind, object) = self
                    .attachment_with_kind(c, message, &placeholder, caps)
                    .await?;
                let voice = kind.as_deref() == Some("voice-message");
                let mime = object.mime_type.as_deref().unwrap_or_default();
                // Android's AAC-in-MP4 recorder currently names files .mp3;
                // its DAV MIME may therefore be audio/mpeg. The native player
                // still identifies the container from bounded bytes.
                // https://github.com/nextcloud/talk-android/blob/master/app/src/main/java/com/nextcloud/talk/chat/data/io/MediaRecorderManager.kt
                let allowed = if voice {
                    matches!(
                        mime,
                        "audio/wav"
                            | "audio/x-wav"
                            | "audio/ogg"
                            | "application/ogg"
                            | "audio/opus"
                            | "audio/mpeg"
                            | "audio/webm"
                            | "audio/mp4"
                            | "audio/x-m4a"
                            | "video/mp4"
                    )
                } else {
                    // An ordinary audio attachment: whatever the server calls audio,
                    // within the same size bound as other media. The player decides
                    // from the bytes whether it can decode it.
                    mime.starts_with("audio/") || mime == "application/ogg"
                };
                if !allowed {
                    return Err(self.error(ErrorKind::Unsupported));
                }
                let limit = if voice {
                    api::files::MAX_FILE_BYTES
                } else {
                    api::files::MAX_FILE_BYTES.min(api::content::MAX_VIDEO_BYTES as u64)
                };
                if object.size.is_some_and(|size| size > limit) {
                    return Err(self.error(ErrorKind::TooLarge));
                }
                let raw = object
                    .path
                    .as_deref()
                    .ok_or_else(|| self.error(ErrorKind::Unsupported))?;
                let path =
                    api::files::FilePath::parse(&format!("/{}", raw.trim_start_matches('/')))
                        .map_err(|kind| self.error(transport::kind(kind)))?;
                let bytes = self.transport.voice_audio(c, path).await?;
                self.transport.validate(c)?;
                Ok(ContentResult::VoiceAudio(
                    VideoBytes::new(bytes).map_err(|_| self.error(ErrorKind::TooLarge))?,
                ))
            }
            ContentCommand::Video {
                message,
                placeholder,
            } => {
                let object = self.attachment(c, message, &placeholder, caps).await?;
                let format = match object.mime_type.as_deref() {
                    Some("video/mp4") => api::content::VideoFormat::Mp4,
                    Some("video/webm") => api::content::VideoFormat::WebM,
                    _ => return Err(self.error(ErrorKind::Unsupported)),
                };
                if object
                    .size
                    .is_some_and(|bytes| bytes > api::content::MAX_VIDEO_BYTES as u64)
                {
                    return Err(self.error(ErrorKind::TooLarge));
                }
                let raw = object
                    .path
                    .as_deref()
                    .ok_or_else(|| self.error(ErrorKind::Unsupported))?;
                let path =
                    api::files::FilePath::parse(&format!("/{}", raw.trim_start_matches('/')))
                        .map_err(|kind| self.error(transport::kind(kind)))?;
                // Reuse the authenticated, cancelable DAV owner. Original video
                // bytes never enter the raster decoder, server preview lane or
                // a URL/playlist-aware player source.
                let bytes = self.transport.video(c, path, format).await?;
                self.transport.validate(c)?;
                Ok(ContentResult::Video(
                    VideoBytes::new(bytes).map_err(|_| self.error(ErrorKind::TooLarge))?,
                ))
            }
            ContentCommand::SelfProfile | ContentCommand::RevalidateProfile => {
                let now = messaging::now_unix();
                if matches!(command, ContentCommand::SelfProfile)
                    && let Some(profile) = self.avatars.profile(now)
                {
                    self.transport.validate(c)?;
                    return Ok(ContentResult::SelfProfile {
                        name: profile.name,
                        organization: profile.organization,
                        saved_at: profile.saved_at,
                    });
                }
                let reply = self.transport.api_request(c, api::Request::Account).await?;
                let api::Reply::Account(profile) = reply else {
                    return Err(self.error(ErrorKind::Malformed));
                };
                let profile = avatars::Profile {
                    name: profile
                        .display_name
                        .as_deref()
                        .map(Text::new)
                        .transpose()
                        .map_err(|_| self.error(ErrorKind::TooLarge))?,
                    organization: profile
                        .organization
                        .as_deref()
                        .map(Text::new)
                        .transpose()
                        .map_err(|_| self.error(ErrorKind::TooLarge))?,
                    saved_at: now,
                };
                self.transport.validate(c)?;
                self.avatars.put_profile(profile.clone());
                Ok(ContentResult::SelfProfile {
                    name: profile.name,
                    organization: profile.organization,
                    saved_at: profile.saved_at,
                })
            }
            ContentCommand::OpenCloud {
                message,
                placeholder,
            } => {
                let object = self.attachment(c, message, &placeholder, caps).await?;
                let file = object
                    .id
                    .as_deref()
                    .and_then(|s| s.parse::<u64>().ok())
                    .filter(|n| *n > 0)
                    .ok_or_else(|| self.error(ErrorKind::Unsupported))?;
                let link = self.transport.file_link(file)?;
                Ok(ContentResult::CloudLink(
                    Text::new(&link).map_err(|_| self.error(ErrorKind::TooLarge))?,
                ))
            }
            ContentCommand::Asset(request) | ContentCommand::RevalidateAvatar(request) => {
                if revalidate && matches!(request, AssetRequest::Attachment { .. }) {
                    return Err(self.error(ErrorKind::Unsupported));
                }
                use api::content::ImageRequest as I;
                let mut durable = true;
                let mut room_key = None;
                let mut room_identity = None;
                let image = match request {
                    AssetRequest::CloudFile(file) => {
                        if file == 0 {
                            return Err(self.error(ErrorKind::Malformed));
                        }
                        let bytes = self
                            .transport
                            .image(c, I::FilePreview { file, large: false })
                            .await?;
                        self.transport.validate(c)?;
                        return Ok(ContentResult::Image(
                            ImageBytes::new(bytes).map_err(|_| self.error(ErrorKind::TooLarge))?,
                        ));
                    }
                    AssetRequest::SelfAvatar => I::UserAvatar {
                        user: self.transport.user().to_owned(),
                    },
                    AssetRequest::ConversationAvatar(id) => {
                        let room = self.content_room(c, id, caps).await?;
                        room_key = Some(avatars::key("room", self.check_room(id)?));
                        room_identity =
                            Some(avatars::canonical_identity("room", self.check_room(id)?));
                        durable = !room.classified && !room.sensitive;
                        if !caps.has("avatar") {
                            return Err(self.error(ErrorKind::Unsupported));
                        }
                        I::ConversationAvatar(
                            api::models::RoomToken::parse(self.check_room(id)?)
                                .map_err(|k| self.error(transport::kind(k)))?,
                        )
                    }
                    AssetRequest::MessageAvatar(message) => {
                        let room = self.content_room(c, message.conversation, caps).await?;
                        durable = !room.classified && !room.sensitive;
                        let message = self.target(c, message).await?;
                        if message.actor_type.as_deref() != Some("users") {
                            return Err(self.error(ErrorKind::Unsupported));
                        }
                        I::UserAvatar {
                            user: message
                                .actor_id
                                .ok_or_else(|| self.error(ErrorKind::NotFound))?,
                        }
                    }
                    AssetRequest::ParticipantAvatar(member) => {
                        let room = self.content_room(c, member.conversation, caps).await?;
                        durable = !room.classified && !room.sensitive;
                        let actor = self
                            .members
                            .iter()
                            .find(|m| m.id == member && m.actor_type == "users")
                            .map(|m| m.actor_id.clone())
                            .ok_or_else(|| self.error(ErrorKind::WrongScope))?;
                        let token =
                            api::models::RoomToken::parse(self.check_room(member.conversation)?)
                                .map_err(|k| self.error(transport::kind(k)))?;
                        let reply = self
                            .transport
                            .api_request(c, api::Request::Participants(token))
                            .await?;
                        if !matches!(reply,api::Reply::Participants(ref rows) if rows.iter().any(|p|p.actor_type.as_deref()==Some("users")&&p.actor_id.as_deref()==Some(&actor)))
                        {
                            return Err(self.error(ErrorKind::PermissionDenied));
                        }
                        I::UserAvatar { user: actor }
                    }
                    AssetRequest::PersonAvatar(person) => {
                        if person.scope != c.scope {
                            return Err(self.error(ErrorKind::WrongScope));
                        }
                        let now = self.transport.now();
                        let user = self
                            .people
                            .iter()
                            .find(|p| {
                                p.id == person
                                    && p.expires.epoch == now.epoch
                                    && p.expires.millis > now.millis
                            })
                            .map(|p| p.actor_id.clone())
                            .ok_or_else(|| self.error(ErrorKind::WrongScope))?;
                        I::UserAvatar { user }
                    }
                    AssetRequest::Attachment {
                        message,
                        placeholder,
                        large,
                    } => {
                        return self
                            .attachment_image(c, message, &placeholder, large, caps)
                            .await;
                    }
                };
                let key = match &image {
                    I::UserAvatar { user } => avatars::key("user", user),
                    I::ConversationAvatar(_) => {
                        room_key.ok_or_else(|| self.error(ErrorKind::WrongScope))?
                    }
                    _ => return Err(self.error(ErrorKind::Unsupported)),
                };
                let identity = match &image {
                    I::UserAvatar { user } => avatars::canonical_identity("user", user),
                    I::ConversationAvatar(_) => {
                        room_identity.ok_or_else(|| self.error(ErrorKind::WrongScope))?
                    }
                    _ => return Err(self.error(ErrorKind::Unsupported)),
                };
                self.transport.validate(c)?;
                let now = messaging::now_unix();
                let (bytes, saved_at) = if let Some(cached) =
                    self.avatars.get(&key, now, durable).filter(|_| !revalidate)
                {
                    cached
                } else {
                    let bytes = self.transport.image(c, image).await?;
                    self.transport.validate(c)?;
                    self.avatars.put(&key, &bytes, now, durable);
                    (bytes, now)
                };
                self.transport.validate(c)?;
                let source = AvatarSource {
                    identity,
                    saved_at,
                    restricted: !durable,
                };
                if let Some((emoji, color)) = server_emoji(&bytes) {
                    return Ok(ContentResult::Emoji {
                        text: Text::new(emoji).map_err(|_| self.error(ErrorKind::TooLarge))?,
                        color: color
                            .map(Text::new)
                            .transpose()
                            .map_err(|_| self.error(ErrorKind::TooLarge))?,
                        source,
                    });
                }
                Ok(ContentResult::Image(
                    ImageBytes::avatar(bytes, source)
                        .map_err(|_| self.error(ErrorKind::TooLarge))?,
                ))
            }
            ContentCommand::Poll {
                conversation,
                command,
            } => {
                self.poll_content(c, operation, conversation, command, caps)
                    .await
            }
        }
    }
    async fn attachment_image(
        &mut self,
        c: &OperationContext,
        message: MessageId,
        placeholder: &Text<128>,
        large: bool,
        caps: &Capabilities,
    ) -> Result<ContentResult, PortError> {
        let object = self.attachment(c, message, placeholder, caps).await?;
        let file = object
            .id
            .as_deref()
            .and_then(|id| id.parse::<u64>().ok())
            .filter(|id| *id > 0)
            .ok_or_else(|| self.error(ErrorKind::Unsupported))?;
        // The message and its room were just read fresh (`attachment` also refuses
        // classified, sensitive and unknown-policy rooms), so a cached preview is
        // served only to someone who may see it now.
        let cache_key = self
            .previews
            .is_some()
            .then(|| super::previews::key(file, large, object.size, object.mime_type.as_deref()));
        if let Some(key) = &cache_key
            && let Some(store) = self.previews.as_mut()
        {
            match store.get(key, super::messaging::now_unix()) {
                Ok(Some((bytes, pages))) => {
                    self.transport.validate(c)?;
                    let image = match pages {
                        Some(pages) => ImageBytes::first_pdf_page(bytes, pages),
                        None => ImageBytes::new(bytes),
                    };
                    if let Ok(image) = image {
                        return Ok(ContentResult::Image(image));
                    }
                }
                Ok(None) => {}
                // A broken store only disables caching; the network path stays.
                Err(()) => self.previews = None,
            }
        }
        let result = self.fetch_attachment_image(c, &object, file, large).await?;
        if let (Some(key), ContentResult::Image(image), Some(store)) =
            (&cache_key, &result, self.previews.as_mut())
            && store
                .put(
                    key,
                    &(image.as_slice().to_vec(), image.document_pages()),
                    super::messaging::now_unix(),
                )
                .is_err()
        {
            self.previews = None;
        }
        Ok(result)
    }
    async fn fetch_attachment_image(
        &mut self,
        c: &OperationContext,
        object: &api::chat::RichObject,
        file: u64,
        large: bool,
    ) -> Result<ContentResult, PortError> {
        let object = object.clone();
        if large
            && matches!(
                object.mime_type.as_deref(),
                Some("image/png" | "image/jpeg" | "image/gif" | "image/webp" | "application/pdf")
            )
        {
            // Opening the gallery requests the original, not a server-generated
            // 1600px thumbnail. Its failure must stay visible: falling back here
            // would silently present reduced content as the requested original.
            return self.attachment_original(c, &object, large).await;
        }
        let result = self
            .transport
            .image(c, api::content::ImageRequest::FilePreview { file, large })
            .await;
        let bytes = match result {
            Ok(bytes) => bytes,
            Err(error)
                if matches!(error.kind, ErrorKind::NotFound | ErrorKind::Unsupported)
                    && matches!(
                        object.mime_type.as_deref(),
                        Some("image/png" | "image/jpeg" | "image/gif" | "image/webp")
                    )
                    && object
                        .size
                        .is_none_or(|bytes| bytes <= api::content::MAX_IMAGE_BYTES as u64) =>
            {
                // A disabled/missing preview provider must not hide an otherwise
                // supported image. DAV still enforces the fresh account's file
                // permissions; denial, redirects and cancellation never fallback.
                if object.path.is_none() {
                    return Err(error);
                }
                return self.attachment_original(c, &object, large).await;
            }
            #[cfg(target_os = "macos")]
            Err(error)
                if matches!(error.kind, ErrorKind::NotFound | ErrorKind::Unsupported)
                    && object.mime_type.as_deref() == Some("application/pdf")
                    && object
                        .size
                        .is_none_or(|bytes| bytes <= api::content::MAX_IMAGE_BYTES as u64) =>
            {
                if object.path.is_none() {
                    return Err(error);
                }
                return self.attachment_original(c, &object, large).await;
            }
            Err(error) => return Err(error),
        };
        Ok(ContentResult::Image(
            ImageBytes::new(bytes).map_err(|_| self.error(ErrorKind::TooLarge))?,
        ))
    }
    async fn attachment_original(
        &mut self,
        c: &OperationContext,
        object: &api::chat::RichObject,
        large: bool,
    ) -> Result<ContentResult, PortError> {
        self.transport.validate(c)?;
        if object
            .size
            .is_some_and(|bytes| bytes > api::content::MAX_IMAGE_BYTES as u64)
        {
            return Err(self.error(ErrorKind::TooLarge));
        }
        let raw = object
            .path
            .as_deref()
            .ok_or_else(|| self.error(ErrorKind::Unsupported))?;
        // Only the path on the freshly read attachment is accepted. The API
        // scopes DAV to this authenticated account and rejects traversal; rich
        // object links and guessed filename/file-id URLs never enter this lane.
        let path = api::files::FilePath::parse(&format!("/{}", raw.trim_start_matches('/')))
            .map_err(|kind| self.error(transport::kind(kind)))?;
        match object.mime_type.as_deref() {
            Some("image/png" | "image/jpeg" | "image/gif" | "image/webp") => {
                // Both a declared Content-Length and streamed chunks enforce
                // the same 4 MiB cap before bytes reach the native decoder.
                let bytes = self
                    .transport
                    .image(c, api::content::ImageRequest::FileImage(path))
                    .await?;
                self.transport.validate(c)?;
                Ok(ContentResult::Image(
                    ImageBytes::new(bytes).map_err(|_| self.error(ErrorKind::TooLarge))?,
                ))
            }
            #[cfg(target_os = "macos")]
            Some("application/pdf") => {
                let bytes = self.transport.preview_pdf(c, path).await?;
                self.transport.validate(c)?;
                let timeout = std::time::Duration::from_millis(
                    c.deadline
                        .millis
                        .saturating_sub(self.transport.now().millis),
                )
                .min(std::time::Duration::from_secs(5));
                let image = super::pdf_preview::render(
                    bytes,
                    large,
                    c.cancellation.clone(),
                    self.transport.cancellation(),
                    timeout,
                )
                .await
                .map_err(|kind| self.error(kind))?;
                self.transport.validate(c)?;
                Ok(ContentResult::Image(image))
            }
            _ => {
                // No document/player implementation is inferred from a server
                // thumbnail or a MIME label. Other formats keep their explicit
                // preview/download/external-open paths.
                let _ = (path, large);
                Err(self.error(ErrorKind::Unsupported))
            }
        }
    }
    async fn poll_content(
        &mut self,
        c: &OperationContext,
        operation: OperationId,
        id: ConversationId,
        command: PollCommand,
        caps: &Capabilities,
    ) -> Result<ContentResult, PortError> {
        use api::poll::Command as W;
        let room = self.content_room(c, id, caps).await?;
        let create = poll_availability(&room, caps, true);
        let manage = ActionAvailability {
            implemented: true,
            server: caps.feature("talk-polls-drafts"),
            allowed: if matches!(
                room.role,
                Role::Owner | Role::Moderator | Role::GuestModerator
            ) {
                create.allowed
            } else {
                Decision::Denied
            },
        };
        let edit = ActionAvailability {
            server: caps.feature("edit-draft-poll"),
            ..manage
        };
        if command == PollCommand::Capabilities {
            return Ok(ContentResult::PollCapabilities {
                create,
                drafts: manage,
                edit_draft: edit,
            });
        }
        if !caps.has("talk-polls") {
            return Err(self.error(ErrorKind::Unsupported));
        }
        let token = api::models::RoomToken::parse(self.check_room(id)?)
            .map_err(|k| self.error(transport::kind(k)))?;
        if command == PollCommand::ListDrafts {
            if !caps.has("talk-polls-drafts") {
                return Err(self.error(ErrorKind::Unsupported));
            }
            self.allowed(manage.allowed)?;
            let drafts = self.transport.poll_drafts(c, &token).await?;
            let mut result = Page::empty();
            for draft in drafts {
                result
                    .try_push(self.project_poll_draft(draft)?)
                    .map_err(|_| self.error(ErrorKind::TooLarge))?;
            }
            return Ok(ContentResult::PollDrafts(result));
        }
        if matches!(
            command,
            PollCommand::SaveDraft(_)
                | PollCommand::UpdateDraft { .. }
                | PollCommand::DeleteDraft(_)
        ) {
            if !caps.has("talk-polls-drafts") {
                return Err(self.error(ErrorKind::Unsupported));
            }
            self.allowed(manage.allowed)?;
            if matches!(command, PollCommand::UpdateDraft { .. }) && !caps.has("edit-draft-poll") {
                return Err(self.error(ErrorKind::Unsupported));
            }
            self.admit_action(id, operation)?;
            let result = match &command {
                PollCommand::DeleteDraft(poll) => self
                    .transport
                    .delete_poll_draft(c, &token, *poll)
                    .await
                    .map(|()| ContentResult::PollDraftDeleted(*poll)),
                PollCommand::SaveDraft(draft) | PollCommand::UpdateDraft { draft, .. } => {
                    let question = draft.question.as_str().to_owned();
                    let options = draft
                        .options
                        .as_slice()
                        .iter()
                        .map(|s| s.as_str().to_owned())
                        .collect();
                    let wire = match &command {
                        PollCommand::UpdateDraft { poll, .. } => W::UpdateDraft {
                            poll: *poll,
                            question,
                            options,
                            hidden_results: draft.hidden_results,
                            max_votes: draft.max_votes,
                        },
                        _ => W::CreateDraft {
                            question,
                            options,
                            hidden_results: draft.hidden_results,
                            max_votes: draft.max_votes,
                        },
                    };
                    self.transport
                        .poll_draft_mutation(c, &token, wire)
                        .await
                        .and_then(|draft| self.project_poll_draft(draft))
                        .map(|draft| ContentResult::PollDraftSaved(Box::new(draft)))
                }
                _ => Err(self.error(ErrorKind::Malformed)),
            };
            self.settle_action(operation, result.as_ref().err());
            return result;
        }
        let wire = match &command {
            PollCommand::Capabilities
            | PollCommand::ListDrafts
            | PollCommand::SaveDraft(_)
            | PollCommand::UpdateDraft { .. }
            | PollCommand::DeleteDraft(_) => return Err(self.error(ErrorKind::Malformed)),
            PollCommand::Read(n) => W::Read(*n),
            PollCommand::Create(draft) => {
                self.allowed(create.allowed)?;
                W::Create {
                    question: draft.question.as_str().into(),
                    options: draft
                        .options
                        .as_slice()
                        .iter()
                        .map(|s| s.as_str().into())
                        .collect(),
                    hidden_results: draft.hidden_results,
                    max_votes: draft.max_votes,
                }
            }
            PollCommand::Vote { poll, options } => {
                let current = self.transport.poll(c, &token, W::Read(*poll)).await?;
                if current.status != 0 {
                    return Err(self.error(ErrorKind::Conflict));
                }
                if options
                    .as_slice()
                    .iter()
                    .any(|n| usize::from(*n) >= current.options.len())
                    || current.max_votes > 0
                        && options.as_slice().len() > usize::from(current.max_votes)
                {
                    return Err(self.error(ErrorKind::Malformed));
                }
                W::Vote {
                    poll: *poll,
                    options: options.as_slice().to_vec(),
                }
            }
            PollCommand::Close(poll) => {
                let current = self.transport.poll(c, &token, W::Read(*poll)).await?;
                if current.status == 1 {
                    return self.project_poll(current, &room, caps);
                }
                if current.status != 0 {
                    return Err(self.error(ErrorKind::Unsupported));
                }
                self.allowed(self.poll_close(&current, &room))?;
                W::Close(*poll)
            }
        };
        let mutation = !matches!(command, PollCommand::Read(_));
        if mutation {
            self.admit_action(id, operation)?;
        }
        let result = self
            .transport
            .poll(c, &token, wire)
            .await
            .and_then(|poll| self.project_poll(poll, &room, caps));
        if mutation {
            self.settle_action(operation, result.as_ref().err());
        }
        result
    }
    fn project_poll_draft(&self, draft: api::poll::Draft) -> Result<PollDraftEntry, PortError> {
        let mut options = Page::empty();
        for option in draft.options {
            options
                .try_push(Text::new(&option).map_err(|_| self.error(ErrorKind::TooLarge))?)
                .map_err(|_| self.error(ErrorKind::TooLarge))?;
        }
        Ok(PollDraftEntry {
            id: draft.id,
            draft: PollDraft {
                question: Text::new(&draft.question)
                    .map_err(|_| self.error(ErrorKind::TooLarge))?,
                options,
                max_votes: draft.max_votes,
                hidden_results: draft.hidden_results,
            },
            max_votes: draft.max_votes,
        })
    }
    fn poll_close(&self, poll: &api::poll::Poll, room: &Conversation) -> Decision {
        if poll.status == 0
            && (matches!(
                room.role,
                Role::Owner | Role::Moderator | Role::GuestModerator
            ) || poll.actor_type == "users" && poll.actor_id == self.transport.user())
        {
            room.actions.read_messages
        } else {
            Decision::Denied
        }
    }
    fn project_poll(
        &self,
        poll: api::poll::Poll,
        room: &Conversation,
        caps: &Capabilities,
    ) -> Result<ContentResult, PortError> {
        if poll.status == 2 {
            return Err(self.error(ErrorKind::Unsupported));
        }
        let mut vote = poll_availability(room, caps, false);
        if poll.status != 0 {
            vote.allowed = Decision::Denied;
        }
        let close = ActionAvailability {
            implemented: true,
            server: FeatureSupport::Supported,
            allowed: self.poll_close(&poll, room),
        };
        let mut options = Page::empty();
        for (index, label) in poll.options.iter().enumerate() {
            options
                .try_push(PollOption {
                    id: index as u16,
                    label: Text::new(label).map_err(|_| self.error(ErrorKind::TooLarge))?,
                    votes: poll.votes.as_ref().and_then(|v| v.get(index).copied()),
                    own: poll.own.contains(&(index as u16)),
                })
                .map_err(|_| self.error(ErrorKind::TooLarge))?;
        }
        let mut details = Page::empty();
        for voter in poll.details {
            details
                .try_push(PollVoter {
                    name: Text::new(&voter.name).map_err(|_| self.error(ErrorKind::TooLarge))?,
                    option: voter.option,
                })
                .map_err(|_| self.error(ErrorKind::TooLarge))?;
        }
        // The server sends zero to viewers not permitted to see an open count. Preserve
        // absence instead of presenting that privacy placeholder as zero participants.
        let can_count = poll.status == 1
            || poll.votes.is_some()
            || matches!(
                room.role,
                Role::Owner | Role::Moderator | Role::GuestModerator
            )
            || poll.actor_type == "users" && poll.actor_id == self.transport.user();
        Ok(ContentResult::Poll(Box::new(Poll {
            id: poll.id,
            question: Text::new(&poll.question).map_err(|_| self.error(ErrorKind::TooLarge))?,
            options,
            closed: poll.status == 1,
            hidden_results: poll.hidden_results,
            max_votes: poll.max_votes,
            voters: if can_count { poll.voters } else { None },
            details,
            details_complete: poll.details_complete,
            vote,
            close,
        })))
    }
}
/// Read only the plain text from Talk's tiny emoji avatar response. No SVG is
/// rendered, no entity is expanded, and no URI/font/image is followed.
fn server_emoji(bytes: &[u8]) -> Option<(&str, Option<&str>)> {
    if bytes.len() > 4096 {
        return None;
    }
    let value = std::str::from_utf8(bytes).ok()?;
    if !value.contains("<svg ")
        || value.matches("<text ").count() != 1
        || value.contains("<!ENTITY")
    {
        return None;
    }
    let text = value
        .split_once("<text ")?
        .1
        .split_once('>')?
        .1
        .split_once("</text>")?
        .0;
    if text.is_empty()
        || text.len() > 64
        || text.contains(['<', '&'])
        || text.chars().any(char::is_control)
    {
        return None;
    }
    let color = value
        .split_once("fill=\"#")
        .and_then(|(_, v)| v.split_once('"'))
        .map(|(v, _)| v)
        .filter(|s| s.len() == 6 && s.bytes().all(|b| b.is_ascii_hexdigit()));
    Some((text, color))
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn emoji_only_extracts_bounded_plain_text() {
        assert_eq!(
            server_emoji(
                br##"<svg width="512"><rect fill="#123abc"/><text x="50%">hello</text></svg>"##
            ),
            Some(("hello", Some("123abc")))
        );
        assert!(server_emoji(br#"<svg width="512"><text x="1">&entity;</text></svg>"#).is_none());
        assert!(server_emoji(br#"<svg><image href="https://example.test/a"/></svg>"#).is_none());
    }
}
