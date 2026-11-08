//! Explicit chat admission, separate from call/session publication authority.
use super::projection::Project;
use super::*;
use nextcloud_talk_api as api;

impl NextcloudClient {
    pub(super) async fn cleanup_chat_join(&mut self, context: &OperationContext) {
        if !self.chat_join_uncertain {
            return;
        }
        let Some(id) = self.chat_session else {
            return;
        };
        let Ok(token) = self.check_room(id).and_then(|token| {
            api::models::RoomToken::parse(token).map_err(|kind| self.error(transport::kind(kind)))
        }) else {
            return;
        };
        if self
            .transport
            .api_request(context, api::Request::LeaveChat(token))
            .await
            .is_ok()
        {
            self.chat_session = None;
            self.chat_join_uncertain = false;
        }
    }

    pub(super) async fn open_chat(
        &mut self,
        context: &OperationContext,
        id: ConversationId,
        password: Option<Text<512>>,
        caps: &Capabilities,
    ) -> Result<ProtocolReply, PortError> {
        let data = self.room_data(context, id).await?;
        self.enter_chat(context, id, data, password, caps, false)
            .await
    }

    pub(super) async fn enter_chat(
        &mut self,
        context: &OperationContext,
        id: ConversationId,
        mut data: Value,
        password: Option<Text<512>>,
        caps: &Capabilities,
        listed: bool,
    ) -> Result<ProtocolReply, PortError> {
        // A raw chat session has no call generation, signaling or publication
        // grant. Keep it alive: leaving the last session removes a self-joined
        // attendee on both pinned Talk releases (ParticipantService.php).
        if let Some(previous) = self.chat_session.filter(|previous| {
            *previous != id
                || self.chat_join_uncertain
                || matches!(wire::integer(data.get("participantType")), Some(0 | 4))
        }) {
            if !self
                .active
                .as_ref()
                .is_some_and(|active| active.id == previous)
            {
                let token = api::models::RoomToken::parse(self.check_room(previous)?)
                    .map_err(|k| self.error(transport::kind(k)))?;
                self.transport
                    .api_request(context, api::Request::LeaveChat(token))
                    .await?;
            }
            self.chat_session = None;
            self.chat_join_uncertain = false;
        }
        let public = wire::integer(data.get("type")) == Some(3);
        if (public || listed) && !self.active.as_ref().is_some_and(|active| active.id == id) {
            // Pre-join participantType can be zero. Never turn it into a role
            // grant; let the server admit membership, then evaluate its reply.
            // Unsupported privacy/federation policies remain closed.
            if wire::is_federated(&data)
                || wire::integer(data.get("attributes")).is_none_or(|a| a & !31 != 0 || a & 4 != 0)
            {
                return Err(self.error(ErrorKind::Unsupported));
            }
            if self.chat_session != Some(id) {
                let token = api::models::RoomToken::parse(self.check_room(id)?)
                    .map_err(|k| self.error(transport::kind(k)))?;
                let password = password.map_or_else(String::new, |p| p.as_str().to_owned());
                // Keep a cleanup tombstone before dispatch. Cancellation or a
                // lost response cannot justify a second overlapping join.
                self.chat_session = Some(id);
                self.chat_join_uncertain = true;
                let result = self
                    .transport
                    .api_request(
                        context,
                        api::Request::JoinChat {
                            room: token,
                            password,
                        },
                    )
                    .await;
                match result {
                    Ok(api::Reply::Room(room)) => {
                        self.chat_join_uncertain = false;
                        data = (*room).project();
                    }
                    Ok(_) => return Err(self.error(ErrorKind::Uncertain)),
                    Err(error) => {
                        if matches!(
                            error.kind,
                            ErrorKind::PermissionDenied
                                | ErrorKind::PasswordRequired
                                | ErrorKind::NotFound
                                | ErrorKind::Conflict
                                | ErrorKind::RateLimited
                                | ErrorKind::Busy
                        ) {
                            self.chat_session = None;
                            self.chat_join_uncertain = false;
                        }
                        return Err(error);
                    }
                }
            }
        }
        self.transport.validate(context)?;
        let mut room = wire::room(&data, id, caps).map_err(|k| self.error(k))?;
        self.project_call(&mut room, &data, caps)?;
        self.project_room_tags(&mut room, &data, caps)?;
        if wire::integer(data.get("lobbyState")) == Some(1)
            && wire::integer(data.get("permissions")).is_none_or(|p| p & 8 == 0)
        {
            return Err(self.error(ErrorKind::Lobby));
        }
        self.allowed(room.actions.read_messages)?;
        Ok(ProtocolReply::Conversation(room))
    }
}
