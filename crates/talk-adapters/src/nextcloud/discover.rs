//! Account-scoped listing tickets and conservative self-join reconciliation.
use super::projection::Project;
use super::*;
use nextcloud_talk_api as api;
use talk_core::policy::Moment;
use talk_core::protocol::{OpenConversation, OpenConversations};

pub(super) struct ListedRoom {
    id: ConversationId,
    expires: Moment,
    password_required: Option<bool>,
}

impl NextcloudClient {
    pub(super) async fn list_open_conversations(
        &mut self,
        context: &OperationContext,
        query: Text<256>,
        caps: &Capabilities,
    ) -> Result<ProtocolReply, PortError> {
        self.listed_rooms.clear();
        if !caps.has("listable-rooms") {
            return Err(self.error(ErrorKind::Unsupported));
        }
        let api::Reply::Rooms(rooms) = self
            .transport
            .api_request(
                context,
                api::Request::ListedRooms {
                    search: query.as_str().to_owned(),
                },
            )
            .await?
        else {
            return Err(self.error(ErrorKind::Malformed));
        };
        let now = self.transport.now();
        let expires = Moment {
            epoch: now.epoch,
            millis: now.millis.saturating_add(120_000),
        };
        let mut result = Page::empty();
        let mut tickets = Vec::new();
        for room in rooms {
            if !matches!(room.kind, Some(2 | 3)) || !matches!(room.listable, Some(1 | 2)) {
                continue;
            }
            let id = self.bind_room(&room.clone().project())?;
            if tickets.iter().any(|ticket: &ListedRoom| ticket.id == id) {
                return Err(self.error(ErrorKind::Malformed));
            }
            result
                .try_push(OpenConversation {
                    id,
                    name: Text::new(&room.name).map_err(|_| self.error(ErrorKind::TooLarge))?,
                    description: room
                        .description
                        .as_deref()
                        .map(Text::new)
                        .transpose()
                        .map_err(|_| self.error(ErrorKind::TooLarge))?,
                    // Talk v4 removed the public count; participants cannot be read
                    // before membership. Never manufacture a count from a partial list.
                    participants: None,
                    password_required: room.has_password,
                })
                .map_err(|_| self.error(ErrorKind::TooLarge))?;
            tickets.push(ListedRoom {
                id,
                expires,
                password_required: room.has_password,
            });
        }
        self.transport.validate(context)?;
        self.listed_rooms = tickets;
        Ok(ProtocolReply::OpenConversations(OpenConversations {
            scope: context.scope,
            rooms: result,
        }))
    }

    pub(super) async fn join_open_conversation(
        &mut self,
        context: &OperationContext,
        id: ConversationId,
        caps: &Capabilities,
    ) -> Result<ProtocolReply, PortError> {
        if !caps.has("listable-rooms") {
            return Err(self.error(ErrorKind::Unsupported));
        }
        if self.listed_join_uncertain.is_some() || self.active.is_some() {
            return Err(self.error(ErrorKind::Busy));
        }
        let now = self.transport.now();
        let ticket = self
            .listed_rooms
            .iter()
            .find(|ticket| {
                ticket.id == id
                    && ticket.expires.epoch == now.epoch
                    && now.millis < ticket.expires.millis
            })
            .ok_or_else(|| self.error(ErrorKind::WrongScope))?;
        if ticket.password_required != Some(false) {
            return Err(self.error(ErrorKind::Unsupported));
        }
        let token = api::models::RoomToken::parse(self.check_room(id)?)
            .map_err(|k| self.error(transport::kind(k)))?;
        // Retain uncertainty before dispatch: a dropped reply can still have joined.
        self.listed_join_uncertain = Some(id);
        let session = match self
            .transport
            .api_request(context, api::Request::ActivateRoom(token.clone()))
            .await
        {
            Ok(api::Reply::RoomSession(session)) => session.into_secret(),
            Ok(_) => return Err(self.error(ErrorKind::Uncertain)),
            Err(error) => {
                if matches!(
                    error.kind,
                    ErrorKind::PermissionDenied
                        | ErrorKind::NotFound
                        | ErrorKind::Conflict
                        | ErrorKind::RateLimited
                        | ErrorKind::Busy
                ) {
                    self.listed_join_uncertain = None;
                    return Err(error);
                }
                return Err(self.error(ErrorKind::Uncertain));
            }
        };
        self.active = Some(ActiveConversation { id, session });
        // Active is a session, not membership. Close the temporary session so it
        // cannot block a later call, while retaining uncertain cleanup on failure.
        if self
            .transport
            .api_request(context, api::Request::DeactivateRoom(token))
            .await
            .is_err()
        {
            return Err(self.error(ErrorKind::Uncertain));
        }
        self.active = None;
        let room = self
            .room(context, id, caps)
            .await
            .map_err(|_| self.error(ErrorKind::Uncertain))?;
        self.listed_join_uncertain = None;
        self.listed_rooms.retain(|ticket| ticket.id != id);
        Ok(ProtocolReply::Conversation(room))
    }

    pub(super) async fn cleanup_listed_join(&mut self, context: &OperationContext) {
        let Some(id) = self.listed_join_uncertain else {
            return;
        };
        let Ok(token) = self.check_room(id).and_then(|value| {
            api::models::RoomToken::parse(value).map_err(|k| self.error(transport::kind(k)))
        }) else {
            return;
        };
        if self
            .transport
            .api_request(context, api::Request::DeactivateRoom(token))
            .await
            .is_ok()
            && self.active.as_ref().is_some_and(|active| active.id == id)
        {
            self.active = None;
        }
    }
}
