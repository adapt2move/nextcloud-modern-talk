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
        ContentCommand::Poll {
            command: PollCommand::Create(_) | PollCommand::Vote { .. } | PollCommand::Close(_),
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
        let room = self.content_room(c, message.conversation, caps).await?;
        if room.classified || room.sensitive {
            return Err(self.error(ErrorKind::PermissionDenied));
        }
        self.target(c, message)
            .await?
            .rich
            .into_iter()
            .find(|o| o.kind == "file" && o.placeholder == placeholder.as_str())
            .ok_or_else(|| self.error(ErrorKind::NotFound))
    }
    pub(super) async fn content(
        &mut self,
        c: &OperationContext,
        operation: OperationId,
        command: ContentCommand,
        caps: &Capabilities,
    ) -> Result<ContentResult, PortError> {
        match command {
            ContentCommand::SelfProfile => {
                let reply = self.transport.api_request(c, api::Request::Account).await?;
                let api::Reply::Account(profile) = reply else {
                    return Err(self.error(ErrorKind::Malformed));
                };
                Ok(ContentResult::SelfProfile {
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
            ContentCommand::Asset(request) => {
                use api::content::ImageRequest as I;
                let image = match request {
                    AssetRequest::SelfAvatar => I::UserAvatar {
                        user: self.transport.user().to_owned(),
                    },
                    AssetRequest::ConversationAvatar(id) => {
                        self.content_room(c, id, caps).await?;
                        if !caps.has("avatar") {
                            return Err(self.error(ErrorKind::Unsupported));
                        }
                        I::ConversationAvatar(
                            api::models::RoomToken::parse(self.check_room(id)?)
                                .map_err(|k| self.error(transport::kind(k)))?,
                        )
                    }
                    AssetRequest::MessageAvatar(message) => {
                        self.content_room(c, message.conversation, caps).await?;
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
                        self.content_room(c, member.conversation, caps).await?;
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
                        let object = self.attachment(c, message, &placeholder, caps).await?;
                        let file = object
                            .id
                            .as_deref()
                            .and_then(|s| s.parse::<u64>().ok())
                            .filter(|n| *n > 0)
                            .ok_or_else(|| self.error(ErrorKind::Unsupported))?;
                        I::FilePreview { file, large }
                    }
                };
                let bytes = self.transport.image(c, image).await?;
                if let Some((emoji, color)) = server_emoji(&bytes) {
                    return Ok(ContentResult::Emoji {
                        text: Text::new(emoji).map_err(|_| self.error(ErrorKind::TooLarge))?,
                        color: color
                            .map(Text::new)
                            .transpose()
                            .map_err(|_| self.error(ErrorKind::TooLarge))?,
                    });
                }
                Ok(ContentResult::Image(
                    ImageBytes::new(bytes).map_err(|_| self.error(ErrorKind::TooLarge))?,
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
        if command == PollCommand::Capabilities {
            return Ok(ContentResult::PollCapabilities(create));
        }
        if !caps.has("talk-polls") {
            return Err(self.error(ErrorKind::Unsupported));
        }
        let token = api::models::RoomToken::parse(self.check_room(id)?)
            .map_err(|k| self.error(transport::kind(k)))?;
        let wire = match &command {
            PollCommand::Capabilities => return Err(self.error(ErrorKind::Malformed)),
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
                    max_votes: if draft.multiple { 0 } else { 1 },
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
