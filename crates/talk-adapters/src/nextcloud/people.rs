//! Account directory and direct-conversation admission. No room or raw UI user ID.
use super::projection::Project;
use super::*;
use nextcloud_talk_api as api;
use talk_core::policy::Moment;
use talk_core::protocol::conversation::{Recipient, RecipientId, RecipientSource};

pub(super) struct PersonBinding {
    id: RecipientId,
    actor_id: String,
    expires: Moment,
}
pub(super) struct DirectAttempt {
    operation: OperationId,
    recipient: RecipientId,
    room: Option<ConversationId>,
    uncertain: bool,
}
impl NextcloudClient {
    pub(super) async fn find_people(
        &mut self,
        c: &OperationContext,
        query: Text<256>,
    ) -> Result<ProtocolReply, PortError> {
        // A superseding search invalidates previous choices even if it is cancelled.
        // Room invitations use a different binding set and cannot start a direct chat.
        self.people.clear();
        let reply = self
            .transport
            .api_request(
                c,
                api::Request::FindPeople {
                    search: query.as_str().to_owned(),
                },
            )
            .await?;
        let api::Reply::Conversation(api::conversation::Response::Directory(entries)) = reply
        else {
            return Err(self.error(ErrorKind::Malformed));
        };
        let now = self.transport.now();
        let expires = Moment {
            epoch: now.epoch,
            millis: now.millis.saturating_add(120_000),
        };
        let mut people = Vec::new();
        let mut page = Page::empty();
        for entry in entries {
            if entry.source != "users" || entry.id == self.transport.user() {
                continue;
            }
            if entry.id.trim().is_empty()
                || entry.id.chars().any(char::is_control)
                || people
                    .iter()
                    .any(|person: &PersonBinding| person.actor_id == entry.id)
            {
                return Err(self.error(ErrorKind::Malformed));
            }
            let id = RecipientId {
                scope: c.scope,
                local: self.opaque()?,
            };
            page.try_push(Recipient {
                id,
                name: Text::new(&entry.label).map_err(|_| self.error(ErrorKind::TooLarge))?,
                label: Text::new(entry.details.as_deref().unwrap_or(&entry.label))
                    .map_err(|_| self.error(ErrorKind::TooLarge))?,
                source: RecipientSource::User,
            })
            .map_err(|_| self.error(ErrorKind::TooLarge))?;
            people.push(PersonBinding {
                id,
                actor_id: entry.id,
                expires,
            });
        }
        self.people = people;
        Ok(ProtocolReply::People(page))
    }

    pub(super) async fn create_direct(
        &mut self,
        c: &OperationContext,
        operation: OperationId,
        recipient: RecipientId,
        caps: &Capabilities,
    ) -> Result<ProtocolReply, PortError> {
        if recipient.scope != c.scope {
            return Err(self.error(ErrorKind::WrongScope));
        }
        if let Some(attempt) = self
            .direct_attempts
            .iter()
            .find(|a| a.operation == operation)
        {
            if attempt.recipient != recipient {
                return Err(self.error(ErrorKind::WrongScope));
            }
            if let Some(room) = attempt.room {
                return self
                    .room(c, room, caps)
                    .await
                    .map(ProtocolReply::Conversation);
            }
            return Err(self.error(if attempt.uncertain {
                ErrorKind::Uncertain
            } else {
                ErrorKind::Conflict
            }));
        }
        if self.creation_uncertain {
            return Err(self.error(ErrorKind::Uncertain));
        }
        let now = self.transport.now();
        let user = self
            .people
            .iter()
            .find(|person| {
                person.id == recipient
                    && person.expires.epoch == now.epoch
                    && now.millis < person.expires.millis
            })
            .ok_or_else(|| self.error(ErrorKind::WrongScope))?
            .actor_id
            .clone();
        // `can-create` governs group creation. The pinned server handles direct rooms
        // before that check; current user/target permissions remain server-authoritative.
        if self.direct_attempts.len() >= 100 {
            if let Some(index) = self.direct_attempts.iter().position(|a| !a.uncertain) {
                self.direct_attempts.remove(index);
            } else {
                return Err(self.error(ErrorKind::Busy));
            }
        }
        self.direct_attempts.push(DirectAttempt {
            operation,
            recipient,
            room: None,
            uncertain: true,
        });
        self.creation_uncertain = true;
        let result = self
            .transport
            .api_request(c, api::Request::CreateDirect { user })
            .await;
        let reply = match result {
            Ok(reply) => reply,
            Err(error) => {
                if matches!(
                    error.kind,
                    ErrorKind::PermissionDenied
                        | ErrorKind::NotFound
                        | ErrorKind::Conflict
                        | ErrorKind::RateLimited
                ) {
                    self.creation_uncertain = false;
                    if let Some(attempt) = self
                        .direct_attempts
                        .iter_mut()
                        .find(|a| a.operation == operation)
                    {
                        attempt.uncertain = false;
                    }
                }
                return Err(error);
            }
        };
        let api::Reply::Room(room) = reply else {
            return Err(self.error(ErrorKind::Uncertain));
        };
        if room.kind != Some(1) {
            return Err(self.error(ErrorKind::Uncertain));
        }
        let id = self
            .bind_room(&room.project())
            .map_err(|_| self.error(ErrorKind::Uncertain))?;
        if let Some(attempt) = self
            .direct_attempts
            .iter_mut()
            .find(|a| a.operation == operation)
        {
            attempt.room = Some(id);
            attempt.uncertain = false;
        }
        self.creation_uncertain = false;
        self.people.clear();
        // Use the common fresh projection so call scopes/tags share their normal owner.
        self.room(c, id, caps)
            .await
            .map(ProtocolReply::Conversation)
            .map_err(|_| self.error(ErrorKind::Uncertain))
    }
}
