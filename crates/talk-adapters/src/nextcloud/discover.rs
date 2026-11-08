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
        if query.as_str().starts_with("https://") || query.as_str().starts_with("http://") {
            let token = self.transport.conversation_link_token(query.as_str())?;
            let api::Reply::Room(room) = self
                .transport
                .api_request(context, api::Request::Room(token))
                .await?
            else {
                return Err(self.error(ErrorKind::Malformed));
            };
            if room.kind != Some(3) {
                return Err(self.error(ErrorKind::Unsupported));
            }
            let id = self.bind_room(&room.clone().project())?;
            let mut rooms = Page::empty();
            rooms
                .try_push(OpenConversation {
                    id,
                    name: Text::new(&room.name).map_err(|_| self.error(ErrorKind::TooLarge))?,
                    description: None,
                    participants: None,
                    password_required: room.has_password,
                })
                .map_err(|_| self.error(ErrorKind::TooLarge))?;
            let now = self.transport.now();
            self.listed_rooms.push(ListedRoom {
                id,
                expires: Moment {
                    epoch: now.epoch,
                    millis: now.millis.saturating_add(120_000),
                },
                password_required: room.has_password,
            });
            return Ok(ProtocolReply::OpenConversations(OpenConversations {
                scope: context.scope,
                rooms,
            }));
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
        if self.chat_join_uncertain || self.active.is_some() {
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
        if ticket.password_required.is_none() {
            return Err(self.error(ErrorKind::Unsupported));
        }
        let data = self.room_data(context, id).await?;
        let reply = self.enter_chat(context, id, data, None, caps, true).await?;
        self.listed_rooms.retain(|ticket| ticket.id != id);
        Ok(reply)
    }
}
