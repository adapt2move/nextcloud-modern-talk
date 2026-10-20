//! Message command owner. Every mutation uses fresh room policy and scoped identities.
use super::projection::Project;
use super::*;
use api::chat::{Command as WireCommand, Response as WireResponse};
use nextcloud_talk_api as api;
use talk_core::protocol::conversation::ActionAvailability;
use talk_core::protocol::messaging::*;
use talk_core::protocol::{FeatureSupport, Role, RoomKind};

pub(super) struct UploadName {
    conversation: ConversationId,
    path: String,
    name: String,
}
pub(super) fn is_mutation(c: &MessagingCommand) -> bool {
    !matches!(
        c,
        MessagingCommand::Capabilities
            | MessagingCommand::History { .. }
            | MessagingCommand::Poll { .. }
            | MessagingCommand::Context(_)
            | MessagingCommand::ReactionAuthors { .. }
            | MessagingCommand::Mentions(_)
            | MessagingCommand::Search { .. }
            | MessagingCommand::Pins
            | MessagingCommand::ReadReminder(_)
            | MessagingCommand::Reminders
            | MessagingCommand::Scheduled
            | MessagingCommand::Threads
            | MessagingCommand::Thread(_)
            | MessagingCommand::SharedFiles { .. }
    )
}
fn feature(c: &MessagingCommand) -> MessagingFeature {
    use MessagingCommand as C;
    use MessagingFeature as F;
    match c {
        C::History { .. } | C::Poll { .. } | C::Context(_) => F::Read,
        C::Send { .. } => F::Send,
        C::Edit { .. } => F::Edit,
        C::Delete(_) => F::Delete,
        C::React { .. } | C::ReactionAuthors { .. } => F::React,
        C::MarkUnread => F::Read,
        C::MarkRead(_) => F::ReadReceipts,
        C::Typing(_) => F::Typing,
        C::Mentions(_) => F::Mentions,
        C::Search { .. } => F::Search,
        C::Pin { .. } | C::Unpin(_) | C::HidePin(_) | C::Pins => F::Pins,
        C::Reminder { .. } | C::ReadReminder(_) | C::Reminders => F::Reminders,
        C::Scheduled | C::Schedule { .. } | C::EditScheduled { .. } | C::CancelScheduled(_) => {
            F::Schedule
        }
        C::Threads | C::Thread(_) | C::RenameThread { .. } | C::ThreadNotifications { .. } => {
            F::Threads
        }
        C::Forward { .. } => F::Forward,
        C::SharedFiles { .. }
        | C::ShareFile { .. }
        | C::UploadFile { .. }
        | C::DownloadFile { .. } => F::Files,
        C::Translate { .. } => F::Translation,
        C::Summarize { .. } => F::Summary,
        C::Capabilities => F::Read,
    }
}
fn support(f: MessagingFeature) -> &'static str {
    use MessagingFeature as F;
    match f {
        F::Read | F::Send | F::Mentions => "chat-v2",
        F::Reply => "chat-replies",
        F::Edit => "edit-messages",
        F::Delete => "delete-messages",
        F::React => "react-permission",
        F::Silent => "silent-send",
        F::Search => "unified-search",
        F::Pins => "pinned-messages",
        F::Reminders => "remind-me-later",
        F::Schedule => "scheduled-messages",
        F::Threads => "threads",
        F::Forward => "chat-reference-id",
        F::Files => "rich-object-sharing",
        F::Typing => "typing-privacy",
        F::ReadReceipts => "chat-read-marker",
        F::Translation => "translation",
        F::Summary => "chat-summary",
    }
}
fn availability(
    f: MessagingFeature,
    room: &Conversation,
    data: &Value,
    caps: &Capabilities,
) -> ActionAvailability {
    use MessagingFeature as F;
    let allowed = if room.unknown_restrictions {
        Decision::Unknown
    } else if matches!(f, F::Forward | F::Translation | F::Summary) && room.classified {
        Decision::Denied
    } else if f == F::React {
        if room.actions.read_messages == Decision::Allowed
            && wire::integer(data.get("permissions")).is_some_and(|p| p & 256 != 0)
        {
            Decision::Allowed
        } else {
            Decision::Denied
        }
    } else if matches!(
        f,
        F::Send | F::Reply | F::Edit | F::Delete | F::Silent | F::Schedule | F::Forward
    ) {
        room.actions.send_message
    } else if f == F::ReadReceipts && caps.read_receipts != Some(true)
        || f == F::Typing && caps.typing_status != Some(true)
    {
        Decision::Denied
    } else {
        room.actions.read_messages
    };
    ActionAvailability {
        implemented: !matches!(f, F::Typing | F::Translation | F::Summary),
        server: caps.feature(support(f)),
        allowed,
    }
}
fn now_unix() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(u64::MAX)
}
// A deliberately finite subset of Talk's server notification policy. Never infer
// a mention from display text. Group defaults can be changed by the administrator;
// thread subscriptions and group/team mentions need additional authoritative data.
fn notification_policy(
    message: &api::models::Message,
    room: &Conversation,
    data: &Value,
    user: &str,
    after: u64,
) -> Decision {
    if message.id <= after
        || message.update.is_some()
        || message.kind.as_deref() != Some("comment")
        || message.system.as_deref().is_some_and(|s| !s.is_empty())
        || message.edited.is_some_and(|t| t > 0)
        || message.expires.is_some_and(|t| t > 0 && t <= now_unix())
        || message.silent == Some(true)
        || room.classified
        || data.get("notificationLevel").and_then(Value::as_i64) == Some(3)
        || message.actor_type.as_deref() == Some("users")
            && message.actor_id.as_deref() == Some(user)
    {
        return Decision::Denied;
    }
    if room.unknown_restrictions
        || room.actions.read_messages != Decision::Allowed
        || message.system.as_deref() != Some("")
        || message.silent != Some(false)
        || message.actor_type.as_deref() != Some("users")
        || message.actor_id.as_deref().is_none_or(str::is_empty)
        || message.thread_id.is_some_and(|thread| thread != message.id)
    {
        return Decision::Unknown;
    }
    let level = data.get("notificationLevel").and_then(Value::as_i64);
    match level {
        Some(1) => Decision::Allowed,
        Some(0) if room.kind == RoomKind::Direct => Decision::Allowed,
        Some(2)
            if message.rich.iter().any(|object| {
                object.kind == "user"
                    && object.id.as_deref() == Some(user)
                    && message
                        .message
                        .as_ref()
                        .is_some_and(|text| text.contains(&format!("{{{}}}", object.placeholder)))
            }) =>
        {
            Decision::Allowed
        }
        _ => Decision::Unknown,
    }
}
fn notification_presence(status: Option<api::presence::UserStatus>) -> Decision {
    use api::presence::UserStatus as S;
    match status {
        Some(S::Online | S::Away | S::Offline | S::Invisible) => Decision::Allowed,
        Some(S::DoNotDisturb) => Decision::Denied,
        Some(S::Unknown) | None => Decision::Unknown,
    }
}
fn snippet<const N: usize>(s: &str) -> Text<N> {
    let end = s
        .char_indices()
        .map(|(i, _)| i)
        .take_while(|i| *i <= N)
        .last()
        .unwrap_or(0);
    Text::new(if s.len() <= N { s } else { &s[..end] }).expect("bounded UTF-8 prefix")
}
// A random create-only staging file is never the visible destination. Drop also runs
// when the requesting task is canceled, so no unfinished destination is published.
struct PartialDownload(std::path::PathBuf);
impl Drop for PartialDownload {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.0);
    }
}
impl NextcloudClient {
    fn message_scope(&self, id: MessageId, conversation: ConversationId) -> Result<u64, PortError> {
        if id.conversation != conversation || id.sequence == 0 {
            return Err(self.error(ErrorKind::WrongScope));
        }
        Ok(id.sequence)
    }
    fn message_capabilities(
        &self,
        room: &Conversation,
        data: &Value,
        caps: &Capabilities,
    ) -> Result<MessagingCapabilities, PortError> {
        let mut actions = Page::empty();
        use MessagingFeature as F;
        for f in [
            F::Read,
            F::Send,
            F::Reply,
            F::Edit,
            F::Delete,
            F::React,
            F::Mentions,
            F::Silent,
            F::Search,
            F::Pins,
            F::Reminders,
            F::Schedule,
            F::Threads,
            F::Forward,
            F::Files,
            F::Typing,
            F::ReadReceipts,
            F::Translation,
            F::Summary,
        ] {
            actions
                .try_push((f, availability(f, room, data, caps)))
                .map_err(|_| self.error(ErrorKind::TooLarge))?;
        }
        Ok(MessagingCapabilities {
            actions,
            message_limit: caps.message_limit,
            file_limit: api::files::MAX_FILE_BYTES,
            read_receipts: caps.read_receipts,
            typing_status: caps.typing_status,
        })
    }
    fn message_action(
        &self,
        f: MessagingFeature,
        room: &Conversation,
        data: &Value,
        caps: &Capabilities,
    ) -> Result<(), PortError> {
        let a = availability(f, room, data, caps);
        if !a.implemented {
            return Err(self.error(ErrorKind::Unconfigured));
        }
        if a.server != FeatureSupport::Supported {
            return Err(self.error(ErrorKind::Unsupported));
        }
        if a.allowed != Decision::Allowed {
            return Err(self.error(ErrorKind::PermissionDenied));
        }
        Ok(())
    }
    fn details(
        &self,
        mut message: api::models::Message,
        id: ConversationId,
        room: &Conversation,
        caps: &Capabilities,
    ) -> Result<MessageDetails, PortError> {
        if let Some(update) = message.update.take() {
            message = *update;
        }
        if message.token != self.check_room(id)? {
            return Err(self.error(ErrorKind::WrongScope));
        }
        let own = message.actor_type.as_deref() == Some("users")
            && message.actor_id.as_deref() == Some(self.transport.user());
        let moderator = matches!(
            room.role,
            Role::Owner | Role::Moderator | Role::GuestModerator
        );
        let live = message.kind.as_deref() == Some("comment")
            && message.expires.is_none_or(|t| t == 0 || t > now_unix());
        let decision =
            if live && (own || moderator) && room.actions.send_message == Decision::Allowed {
                Decision::Allowed
            } else {
                Decision::Denied
            };
        let mut reactions = Page::empty();
        for r in &message.reactions {
            reactions
                .try_push(Reaction {
                    emoji: Text::new(&r.emoji).map_err(|_| self.error(ErrorKind::TooLarge))?,
                    count: r.count,
                    own: r.own,
                })
                .map_err(|_| self.error(ErrorKind::TooLarge))?;
        }
        let mut rich = Page::empty();
        for r in &message.rich {
            rich.try_push(RichObject {
                placeholder: Text::new(&r.placeholder)
                    .map_err(|_| self.error(ErrorKind::TooLarge))?,
                kind: Text::new(&r.kind).map_err(|_| self.error(ErrorKind::TooLarge))?,
                name: Text::new(&r.name).map_err(|_| self.error(ErrorKind::TooLarge))?,
                link: r
                    .link
                    .as_ref()
                    .map(|s| Text::new(s))
                    .transpose()
                    .map_err(|_| self.error(ErrorKind::TooLarge))?,
            })
            .map_err(|_| self.error(ErrorKind::TooLarge))?;
        }
        let parent = message.parent.as_ref().map(|p| {
            let unavailable = p.unavailable || p.expires.is_some_and(|t| t > 0 && t <= now_unix());
            ReplyContext {
                id: MessageId {
                    conversation: id,
                    sequence: p.id,
                },
                author: if unavailable {
                    snippet("")
                } else {
                    snippet(&p.author)
                },
                text: if unavailable {
                    snippet("")
                } else {
                    snippet(&p.message)
                },
                unavailable,
            }
        });
        let thread = message.thread_id.map(|sequence| MessageId {
            conversation: id,
            sequence,
        });
        let replyable = message.replyable;
        let system = message
            .system
            .as_ref()
            .map(|s| Text::new(s))
            .transpose()
            .map_err(|_| self.error(ErrorKind::TooLarge))?;
        let message = self.message(&message.project(), id)?;
        Ok(MessageDetails {
            message,
            notify: Decision::Denied,
            own,
            replyable,
            parent,
            thread,
            reactions,
            rich,
            system,
            edit: if caps.has("edit-messages") {
                decision
            } else {
                Decision::Denied
            },
            delete: if caps.has("delete-messages") {
                decision
            } else {
                Decision::Denied
            },
        })
    }
    async fn target(
        &mut self,
        c: &OperationContext,
        id: MessageId,
    ) -> Result<api::models::Message, PortError> {
        let room = api::models::RoomToken::parse(self.check_room(id.conversation)?)
            .map_err(|k| self.error(transport::kind(k)))?;
        let reply = self
            .transport
            .api_request(
                c,
                api::Request::Chat {
                    room,
                    command: WireCommand::Context(id.sequence),
                },
            )
            .await?;
        let api::Reply::History { messages, .. } = reply else {
            return Err(self.error(ErrorKind::Malformed));
        };
        let message = messages
            .into_iter()
            .find(|m| m.id == id.sequence)
            .ok_or_else(|| self.error(ErrorKind::NotFound))?;
        if message.kind.as_deref() == Some("comment_deleted")
            || message.expires.is_some_and(|t| t > 0 && t <= now_unix())
        {
            return Err(self.error(ErrorKind::NotFound));
        }
        Ok(message)
    }
    async fn compose_options(
        &mut self,
        c: &OperationContext,
        id: ConversationId,
        o: &ComposeOptions,
        room: &Conversation,
        data: &Value,
        caps: &Capabilities,
    ) -> Result<api::chat::ComposeOptions, PortError> {
        if o.silent {
            self.message_action(MessagingFeature::Silent, room, data, caps)?
        }
        if o.thread.is_some() || o.thread_title.is_some() {
            self.message_action(MessagingFeature::Threads, room, data, caps)?
        }
        if let Some(parent) = o.reply_to {
            self.message_scope(parent, id)?;
            self.message_action(MessagingFeature::Reply, room, data, caps)?;
            if self.target(c, parent).await?.replyable != Some(true) {
                return Err(self.error(ErrorKind::PermissionDenied));
            }
        }
        if let Some(thread) = o.thread {
            self.message_scope(thread, id)?;
            let token = api::models::RoomToken::parse(self.check_room(id)?)
                .map_err(|k| self.error(transport::kind(k)))?;
            self.transport
                .api_request(
                    c,
                    api::Request::Chat {
                        room: token,
                        command: WireCommand::Thread(thread.sequence),
                    },
                )
                .await?;
        }
        Ok(api::chat::ComposeOptions {
            reply_to: o.reply_to.map(|m| m.sequence),
            thread: o.thread.map(|m| m.sequence),
            thread_title: o.thread_title.as_ref().map(|s| s.as_str().into()),
            silent: o.silent,
        })
    }
    fn message_text(
        &self,
        text: &str,
        data: &Value,
        room: &Conversation,
        caps: &Capabilities,
    ) -> Result<(), PortError> {
        let limit = caps
            .message_limit
            .ok_or_else(|| self.error(ErrorKind::Unsupported))?;
        if text.trim().is_empty() || text.chars().count() > limit as usize {
            return Err(self.error(ErrorKind::TooLarge));
        }
        if caps.has("mention-permissions")
            && wire::integer(data.get("mentionPermissions")) != Some(0)
            && !matches!(
                room.role,
                Role::Owner | Role::Moderator | Role::GuestModerator
            )
            && text
                .split(|c: char| c.is_whitespace() || c == ',' || c == '.' || c == ';')
                .any(|part| part == "@all")
        {
            return Err(self.error(ErrorKind::PermissionDenied));
        }
        Ok(())
    }
    async fn detailed_send(
        &mut self,
        c: &OperationContext,
        id: ConversationId,
        operation: OperationId,
        text: String,
        options: api::chat::ComposeOptions,
    ) -> Result<MessagingResult, PortError> {
        if let Some(s) = self.sends.iter().find(|s| s.operation == operation) {
            if s.conversation != id {
                return Err(self.error(ErrorKind::WrongScope));
            }
            return Ok(MessagingResult::Send(SendReceipt {
                operation,
                state: s.state,
            }));
        }
        if self.sends.len() >= 100 {
            if let Some(index) = self
                .sends
                .iter()
                .position(|s| s.state != SendState::Uncertain)
            {
                self.sends.remove(index);
            } else {
                return Err(self.error(ErrorKind::Busy));
            }
        }
        let reference = format!("{:032x}{:032x}", self.opaque()?, self.opaque()?);
        let token = api::models::RoomToken::parse(self.check_room(id)?)
            .map_err(|k| self.error(transport::kind(k)))?;
        self.sends.push(PendingSend {
            operation,
            conversation: id,
            reference: reference.clone(),
            state: SendState::Uncertain,
        });
        let response = self
            .transport
            .api_request(
                c,
                api::Request::Chat {
                    room: token,
                    command: WireCommand::Send {
                        message: text,
                        reference,
                        options,
                    },
                },
            )
            .await;
        match response {
            Ok(api::Reply::Message(message)) => {
                let message = self
                    .message(&message.project(), id)
                    .map_err(|_| self.error(ErrorKind::Uncertain))?;
                if message.own_operation != Some(operation) {
                    return Err(self.error(ErrorKind::Uncertain));
                }
                let state = SendState::ServerAccepted(message.id);
                if let Some(send) = self.sends.iter_mut().find(|s| s.operation == operation) {
                    send.state = state
                }
                Ok(MessagingResult::Send(SendReceipt { operation, state }))
            }
            Ok(_) => Err(self.error(ErrorKind::Uncertain)),
            Err(e) => {
                if e.kind != ErrorKind::Uncertain
                    && let Some(send) = self.sends.iter_mut().find(|s| s.operation == operation)
                {
                    send.state = SendState::Rejected
                }
                Err(e)
            }
        }
    }
    pub(super) async fn messaging(
        &mut self,
        c: &OperationContext,
        id: ConversationId,
        operation: OperationId,
        command: MessagingCommand,
        caps: &Capabilities,
    ) -> Result<MessagingResult, PortError> {
        use MessagingCommand as C;
        use MessagingResult as R;
        let data = self.room_data(c, id).await?;
        let room = wire::room(&data, id, caps).map_err(|k| self.error(k))?;
        if command == C::Capabilities {
            return Ok(R::Capabilities(
                self.message_capabilities(&room, &data, caps)?,
            ));
        }
        self.message_action(feature(&command), &room, &data, caps)?;
        if matches!(command, C::UploadFile { .. } | C::ShareFile { .. }) {
            self.message_action(MessagingFeature::Send, &room, &data, caps)?;
            if caps.conversation_subfolders != Some(true) {
                return Err(self.error(ErrorKind::Unsupported));
            }
        }
        if let C::UploadFile { local_path, name } = &command {
            if caps.attachments_allowed != Some(true) || room.classified {
                return Err(self.error(ErrorKind::PermissionDenied));
            }
            if self.uploads.len() >= 100 {
                return Err(self.error(ErrorKind::Busy));
            }
            let file = tokio::fs::File::open(local_path.as_str())
                .await
                .map_err(|_| self.error(ErrorKind::NotFound))?;
            let metadata = file
                .metadata()
                .await
                .map_err(|_| self.error(ErrorKind::Transport))?;
            if !metadata.is_file() {
                return Err(self.error(ErrorKind::Unsupported));
            }
            let size = metadata.len();
            if size > api::files::MAX_FILE_BYTES {
                return Err(self.error(ErrorKind::TooLarge));
            }
            self.transport.validate(c)?;
            let token = api::models::RoomToken::parse(self.check_room(id)?)
                .map_err(|k| self.error(transport::kind(k)))?;
            let folder = self
                .transport
                .api_request(
                    c,
                    api::Request::Chat {
                        room: token,
                        command: WireCommand::AttachmentFolder {
                            file_names: vec![name.as_str().into()],
                        },
                    },
                )
                .await?;
            let api::Reply::Chat(WireResponse::AttachmentFolder(folder)) = folder else {
                return Err(self.error(ErrorKind::Malformed));
            };
            // The probe returns a user-root-relative Draft folder. Upload under an
            // unpredictable temporary name; final naming happens only on explicit share.
            let path = format!(
                "/{}/nmt-{:032x}",
                folder.folder.trim_matches('/'),
                self.opaque()?
            );
            let remote =
                api::files::FilePath::parse(&path).map_err(|k| self.error(transport::kind(k)))?;
            let length = size as u32;
            self.admit_action(id, operation)?;
            let result = self.transport.upload_file(c, remote, file, size).await;
            self.settle_action(operation, result.as_ref().err());
            result?;
            self.uploads.push(UploadName {
                conversation: id,
                path: path.clone(),
                name: name.as_str().into(),
            });
            return Ok(R::Uploaded {
                path: Text::new(&path).map_err(|_| self.error(ErrorKind::TooLarge))?,
                bytes: length,
            });
        }
        if let C::DownloadFile {
            message,
            placeholder,
            destination,
        } = &command
        {
            self.message_scope(*message, id)?;
            if room.classified {
                return Err(self.error(ErrorKind::PermissionDenied));
            }
            let target = self.target(c, *message).await?;
            let object = target
                .rich
                .iter()
                .find(|o| o.placeholder == placeholder.as_str() && o.kind == "file")
                .ok_or_else(|| self.error(ErrorKind::NotFound))?;
            let raw = object
                .path
                .as_deref()
                .ok_or_else(|| self.error(ErrorKind::Unsupported))?;
            let path = api::files::FilePath::parse(&format!("/{}", raw.trim_start_matches('/')))
                .map_err(|k| self.error(transport::kind(k)))?;
            let destination = std::path::PathBuf::from(destination.as_str());
            let parent = destination
                .parent()
                .filter(|p| !p.as_os_str().is_empty())
                .unwrap_or(std::path::Path::new("."));
            let temporary = parent.join(format!(".nmt-download-{:032x}.part", self.opaque()?));
            let mut options = tokio::fs::OpenOptions::new();
            options.write(true).create_new(true);
            #[cfg(unix)]
            options.mode(0o600);
            let mut file = options
                .open(&temporary)
                .await
                .map_err(|_| self.error(ErrorKind::StorageUnavailable))?;
            let partial = PartialDownload(temporary);
            self.transport.validate(c)?;
            self.admit_action(id, operation)?;
            let result = async {
                let length = self.transport.download_file(c, path, &mut file).await?;
                file.sync_all()
                    .await
                    .map_err(|_| self.error(ErrorKind::StorageUnavailable))?;
                drop(file);
                self.transport.validate(c)?;
                tokio::fs::hard_link(&partial.0, &destination)
                    .await
                    .map_err(|e| {
                        self.error(if e.kind() == std::io::ErrorKind::AlreadyExists {
                            ErrorKind::Conflict
                        } else {
                            ErrorKind::StorageUnavailable
                        })
                    })?;
                Ok(R::FileSaved {
                    bytes: length as u32,
                })
            }
            .await;
            self.settle_action(operation, result.as_ref().err());
            return result;
        }

        if let C::Forward {
            source,
            destination,
        } = &command
        {
            self.message_scope(*source, id)?;
            self.check_room(*destination)?;
            if source.conversation.account != destination.account || room.classified {
                return Err(self.error(ErrorKind::PermissionDenied));
            }
            let original = self.target(c, *source).await?;
            if !original.rich.is_empty() || original.kind.as_deref() != Some("comment") {
                return Err(self.error(ErrorKind::Unsupported));
            }
            let destination_data = self.room_data(c, *destination).await?;
            let destination_room =
                wire::room(&destination_data, *destination, caps).map_err(|k| self.error(k))?;
            self.message_action(
                MessagingFeature::Send,
                &destination_room,
                &destination_data,
                caps,
            )?;
            let text = format!(
                "Forwarded message from {}:\n{}",
                original.author,
                original.message.unwrap_or_default()
            );
            self.message_text(&text, &destination_data, &destination_room, caps)?;
            return self
                .detailed_send(
                    c,
                    *destination,
                    operation,
                    text,
                    api::chat::ComposeOptions::default(),
                )
                .await;
        }
        if let C::Send { text, options } = &command {
            self.message_text(text.as_str(), &data, &room, caps)?;
            if !caps.has("chat-reference-id") {
                return Err(self.error(ErrorKind::Unsupported));
            }
            let options = self
                .compose_options(c, id, options, &room, &data, caps)
                .await?;
            return self
                .detailed_send(c, id, operation, text.as_str().into(), options)
                .await;
        }
        if let C::MarkRead(message) = command {
            self.message_scope(message, id)?;
            let token = api::models::RoomToken::parse(self.check_room(id)?)
                .map_err(|k| self.error(transport::kind(k)))?;
            self.transport
                .api_request(
                    c,
                    api::Request::MarkRead {
                        room: token,
                        message: message.sequence,
                    },
                )
                .await?;
            return Ok(R::Changed);
        }
        if matches!(command, C::Pin { .. } | C::Unpin(_))
            && !matches!(
                room.role,
                Role::Owner | Role::Moderator | Role::GuestModerator
            )
        {
            return Err(self.error(ErrorKind::PermissionDenied));
        }
        let mutation = is_mutation(&command);
        let wire = match &command {
            C::History { before, thread } => WireCommand::History {
                before: before.map(|m| self.message_scope(m, id)).transpose()?,
                thread: thread.map(|m| self.message_scope(m, id)).transpose()?,
            },
            C::Poll { after, thread } => WireCommand::Poll {
                after: self.message_scope(*after, id)?,
                thread: thread.map(|m| self.message_scope(m, id)).transpose()?,
            },
            C::Context(m) => WireCommand::Context(self.message_scope(*m, id)?),
            C::Edit { message, text } => {
                self.message_scope(*message, id)?;
                self.message_text(text.as_str(), &data, &room, caps)?;
                let m = self.target(c, *message).await?;
                if self.details(m, id, &room, caps)?.edit != Decision::Allowed {
                    return Err(self.error(ErrorKind::PermissionDenied));
                }
                WireCommand::Edit {
                    message: message.sequence,
                    text: text.as_str().into(),
                }
            }
            C::Delete(m) => {
                self.message_scope(*m, id)?;
                let target = self.target(c, *m).await?;
                if self.details(target, id, &room, caps)?.delete != Decision::Allowed {
                    return Err(self.error(ErrorKind::PermissionDenied));
                }
                WireCommand::Delete(m.sequence)
            }
            C::React {
                message,
                emoji,
                add,
            } => {
                self.message_scope(*message, id)?;
                self.target(c, *message).await?;
                WireCommand::React {
                    message: message.sequence,
                    reaction: emoji.as_str().into(),
                    add: *add,
                }
            }
            C::ReactionAuthors { message, emoji } => WireCommand::ReactionAuthors {
                message: self.message_scope(*message, id)?,
                reaction: emoji.as_str().into(),
            },
            C::MarkUnread => {
                if !caps.has("chat-unread") {
                    return Err(self.error(ErrorKind::Unsupported));
                }
                WireCommand::MarkUnread
            }
            C::Mentions(search) => WireCommand::Mentions(search.as_str().into()),
            C::Search { query, cursor } => WireCommand::Search {
                query: query.as_str().into(),
                cursor: cursor.as_ref().map(|s| s.as_str().into()),
            },
            C::Pin { message, until } => WireCommand::Pin {
                message: self.message_scope(*message, id)?,
                until: *until,
            },
            C::Unpin(m) => WireCommand::Unpin(self.message_scope(*m, id)?),
            C::HidePin(m) => WireCommand::HidePin(self.message_scope(*m, id)?),
            C::Pins => WireCommand::SharedObjects {
                object_type: "pinned".into(),
                before: None,
            },
            C::Reminder { message, timestamp } => WireCommand::Reminder {
                message: self.message_scope(*message, id)?,
                timestamp: *timestamp,
            },
            C::ReadReminder(m) => WireCommand::ReadReminder(self.message_scope(*m, id)?),
            C::Reminders => {
                if !caps.has("upcoming-reminders") {
                    return Err(self.error(ErrorKind::Unsupported));
                }
                WireCommand::Reminders
            }
            C::Scheduled => WireCommand::Scheduled,
            C::Schedule {
                text,
                send_at,
                options,
            } => {
                self.message_text(text.as_str(), &data, &room, caps)?;
                WireCommand::Schedule {
                    message: text.as_str().into(),
                    send_at: *send_at,
                    options: self
                        .compose_options(c, id, options, &room, &data, caps)
                        .await?,
                }
            }
            C::EditScheduled {
                id: scheduled,
                text,
                send_at,
                silent,
            } => {
                self.message_text(text.as_str(), &data, &room, caps)?;
                WireCommand::EditScheduled {
                    id: self.message_scope(*scheduled, id)?,
                    message: text.as_str().into(),
                    send_at: *send_at,
                    silent: *silent,
                }
            }
            C::CancelScheduled(m) => WireCommand::CancelScheduled(self.message_scope(*m, id)?),
            C::Threads => WireCommand::Threads,
            C::Thread(m) => WireCommand::Thread(self.message_scope(*m, id)?),
            C::RenameThread { root, title } => WireCommand::RenameThread {
                id: self.message_scope(*root, id)?,
                title: title.as_str().into(),
            },
            C::ThreadNotifications { root, level } => WireCommand::ThreadNotifications {
                id: self.message_scope(*root, id)?,
                level: u8::try_from(*level).map_err(|_| self.error(ErrorKind::Malformed))?,
            },
            C::SharedFiles { before } => WireCommand::SharedObjects {
                object_type: "file".into(),
                before: before.map(|m| self.message_scope(m, id)).transpose()?,
            },
            C::ShareFile { path, caption } => {
                if caps.attachments_allowed != Some(true) {
                    return Err(self.error(ErrorKind::PermissionDenied));
                }
                WireCommand::ShareAttachment {
                    path: path.as_str().into(),
                    file_name: self
                        .uploads
                        .iter()
                        .find(|u| u.conversation == id && u.path == path.as_str())
                        .ok_or_else(|| self.error(ErrorKind::WrongScope))?
                        .name
                        .clone(),
                    caption: caption.as_str().into(),
                    reference: format!("{:032x}{:032x}", self.opaque()?, self.opaque()?),
                }
            }
            _ => return Err(self.error(ErrorKind::Unconfigured)),
        };
        let token = api::models::RoomToken::parse(self.check_room(id)?)
            .map_err(|k| self.error(transport::kind(k)))?;
        if mutation {
            self.admit_action(id, operation)?
        }
        let response = self
            .transport
            .api_request(
                c,
                api::Request::Chat {
                    room: token,
                    command: wire,
                },
            )
            .await;
        if mutation && response.is_err() {
            self.settle_action(operation, response.as_ref().err())
        }
        let response = match response {
            Err(e) if matches!(command, C::ReadReminder(_)) && e.kind == ErrorKind::NotFound => {
                return Ok(R::Reminder(None));
            }
            r => r?,
        };
        // Observe presence only for a nonempty eligible future poll. Status errors
        // suppress notifications without losing an otherwise valid message page.
        // The same caller deadline/cancellation bounds this additional read.
        let notify_presence = if let (C::Poll { after, .. }, api::Reply::History { messages, .. }) =
            (&command, &response)
            && messages.iter().any(|message| {
                notification_policy(message, &room, &data, self.transport.user(), after.sequence)
                    == Decision::Allowed
            }) {
            notification_presence(self.transport.own_user_status(c).await.ok())
        } else {
            Decision::Unknown
        };
        let mid = |sequence| MessageId {
            conversation: id,
            sequence,
        };
        let result = (|| match response {
            api::Reply::History {
                messages,
                last_given,
                common_read,
            } => {
                let max_event = messages.iter().map(|m| m.id).max();
                let mut canonical: Vec<(u64, MessageDetails)> = Vec::new();
                for message in messages {
                    let notify = if let C::Poll { after, .. } = &command {
                        match notification_policy(
                            &message,
                            &room,
                            &data,
                            self.transport.user(),
                            after.sequence,
                        ) {
                            Decision::Allowed => notify_presence,
                            decision => decision,
                        }
                    } else {
                        Decision::Denied
                    };
                    let event = message.id;
                    let hidden = matches!(
                        message.system.as_deref(),
                        Some(
                            "reaction_deleted"
                                | "message_edited"
                                | "message_deleted"
                                | "reaction"
                                | "reaction_revoked"
                        )
                    );
                    if hidden && message.update.is_none() {
                        continue;
                    }
                    let mut detail = self.details(message, id, &room, caps)?;
                    detail.notify = notify;
                    if detail.message.expires_at.is_some_and(|t| t <= now_unix()) {
                        continue;
                    }
                    if let Some(old) = canonical
                        .iter_mut()
                        .find(|(_, m)| m.message.id == detail.message.id)
                    {
                        if event > old.0 {
                            *old = (event, detail)
                        }
                    } else {
                        canonical.push((event, detail))
                    }
                }
                canonical.sort_by_key(|(_, m)| m.message.id.sequence);
                let mut out = Page::empty();
                for (_, detail) in canonical {
                    if let Some(operation) = detail.message.own_operation
                        && let Some(send) = self
                            .sends
                            .iter_mut()
                            .find(|s| s.operation == operation && s.conversation == id)
                    {
                        send.state = SendState::ServerAccepted(detail.message.id)
                    }
                    out.try_push(detail)
                        .map_err(|_| self.error(ErrorKind::TooLarge))?;
                }
                let cursor = last_given
                    .or(if matches!(command, C::Poll { .. }) {
                        max_event
                    } else {
                        None
                    })
                    .filter(|n| *n > 0)
                    .map(mid);
                Ok(R::Messages {
                    messages: out,
                    cursor,
                    common_read: common_read
                        .filter(|n| *n > 0 && caps.read_receipts == Some(true))
                        .map(mid),
                })
            }
            api::Reply::Message(message) => Ok(R::Message(Box::new(
                self.details(message, id, &room, caps)?,
            ))),
            api::Reply::Chat(WireResponse::Mentions(values)) => {
                let mut out = Page::empty();
                for v in values {
                    if v.id == "all"
                        && wire::integer(data.get("mentionPermissions")) != Some(0)
                        && !matches!(
                            room.role,
                            Role::Owner | Role::Moderator | Role::GuestModerator
                        )
                    {
                        continue;
                    }
                    out.try_push(Mention {
                        id: Text::new(&v.id).map_err(|_| self.error(ErrorKind::TooLarge))?,
                        label: Text::new(&v.label).map_err(|_| self.error(ErrorKind::TooLarge))?,
                        source: Text::new(&v.source)
                            .map_err(|_| self.error(ErrorKind::TooLarge))?,
                    })
                    .map_err(|_| self.error(ErrorKind::TooLarge))?;
                }
                Ok(R::Mentions(out))
            }
            api::Reply::Chat(WireResponse::ReactionAuthors(values)) => {
                let mut out = Page::empty();
                for v in values {
                    out.try_push(Text::new(&v.name).map_err(|_| self.error(ErrorKind::TooLarge))?)
                        .map_err(|_| self.error(ErrorKind::TooLarge))?
                }
                Ok(R::ReactionAuthors(out))
            }
            api::Reply::Chat(WireResponse::Reminder(v)) => Ok(R::Reminder(Some(MessageReminder {
                message: mid(v.message),
                timestamp: v.timestamp,
            }))),
            api::Reply::Chat(WireResponse::Reminders(values)) => {
                let mut out = Page::empty();
                for v in values {
                    if v.token == self.check_room(id)? {
                        out.try_push(MessageReminder {
                            message: mid(v.message),
                            timestamp: v.timestamp,
                        })
                        .map_err(|_| self.error(ErrorKind::TooLarge))?
                    }
                }
                Ok(R::Reminders(out))
            }
            api::Reply::Chat(WireResponse::Scheduled(values)) => {
                let mut out = Page::empty();
                for v in values {
                    out.try_push(self.scheduled(v, id)?)
                        .map_err(|_| self.error(ErrorKind::TooLarge))?
                }
                Ok(R::Scheduled(out))
            }
            api::Reply::Chat(WireResponse::Schedule(v)) => {
                Ok(R::Schedule(Box::new(self.scheduled(v, id)?)))
            }
            api::Reply::Chat(WireResponse::Threads(values)) => {
                let mut out = Page::empty();
                for v in values {
                    out.try_push(self.thread(v, id)?)
                        .map_err(|_| self.error(ErrorKind::TooLarge))?
                }
                Ok(R::Threads(out))
            }
            api::Reply::Chat(WireResponse::Thread(v)) => {
                Ok(R::Thread(Box::new(self.thread(v, id)?)))
            }
            api::Reply::Chat(WireResponse::Search { hits, cursor }) => {
                let mut out = Page::empty();
                for v in hits {
                    out.try_push(SearchHit {
                        message: mid(v.message),
                        title: Text::new(&v.title).map_err(|_| self.error(ErrorKind::TooLarge))?,
                        excerpt: Text::new(&v.excerpt)
                            .map_err(|_| self.error(ErrorKind::TooLarge))?,
                    })
                    .map_err(|_| self.error(ErrorKind::TooLarge))?;
                }
                Ok(R::Search {
                    hits: out,
                    cursor: cursor
                        .map(|s| Text::new(&s))
                        .transpose()
                        .map_err(|_| self.error(ErrorKind::TooLarge))?,
                })
            }
            api::Reply::Acknowledged | api::Reply::Chat(WireResponse::AttachmentShared(_)) => {
                Ok(R::Changed)
            }
            _ => Err(self.error(ErrorKind::Malformed)),
        })();
        let result = if mutation {
            result.map_err(|_| self.error(ErrorKind::Uncertain))
        } else {
            result
        };
        if result.is_ok()
            && let C::ShareFile { path, .. } = &command
        {
            self.uploads
                .retain(|u| u.conversation != id || u.path != path.as_str())
        }
        if mutation {
            self.settle_action(operation, result.as_ref().err())
        }
        result
    }
    fn scheduled(
        &self,
        s: api::chat::ScheduledMessage,
        id: ConversationId,
    ) -> Result<ScheduledMessage, PortError> {
        let mid = |sequence| MessageId {
            conversation: id,
            sequence,
        };
        Ok(ScheduledMessage {
            id: mid(s.id),
            text: Text::new(&s.message).map_err(|_| self.error(ErrorKind::TooLarge))?,
            send_at: s.send_at,
            options: ComposeOptions {
                reply_to: s.options.reply_to.map(mid),
                thread: s.options.thread.map(mid),
                thread_title: s
                    .options
                    .thread_title
                    .map(|s| Text::new(&s))
                    .transpose()
                    .map_err(|_| self.error(ErrorKind::TooLarge))?,
                silent: s.options.silent,
            },
        })
    }
    fn thread(&self, t: api::chat::Thread, id: ConversationId) -> Result<Thread, PortError> {
        Ok(Thread {
            root: MessageId {
                conversation: id,
                sequence: t.id,
            },
            title: Text::new(&t.title).map_err(|_| self.error(ErrorKind::TooLarge))?,
            replies: t.replies,
            unread: None,
            notification_level: t.notification_level.and_then(|n| i8::try_from(n).ok()),
        })
    }
}
