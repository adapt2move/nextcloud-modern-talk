//! Scoped bridge from the signaling owner's ephemeral observations to core.
use super::*;
use nextcloud_talk_api::models::RoomToken;
use talk_core::protocol::{TypingObservation, TypingPerson};

impl NextcloudClient {
    pub(super) fn watch_typing(
        &mut self,
        context: &OperationContext,
        id: ConversationId,
    ) -> Result<ProtocolReply, PortError> {
        let token = RoomToken::parse(self.check_room(id)?)
            .map_err(|error| self.error(transport::kind(error)))?;
        if self.typing_watch.as_ref().is_some_and(|(current, watch)| {
            *current == id && self.transport.typing_matches(context, watch, &token)
        }) {
            return Ok(ProtocolReply::TypingAccepted);
        }
        self.typing_watch = None;
        let watch = self.transport.watch_typing(context, token)?;
        self.typing_watch = Some((id, watch));
        Ok(ProtocolReply::TypingAccepted)
    }
    pub(super) fn typing_activity(
        &self,
        context: &OperationContext,
        id: ConversationId,
        active: bool,
    ) -> Result<ProtocolReply, PortError> {
        let Some((_, watch)) = self
            .typing_watch
            .as_ref()
            .filter(|(current, _)| *current == id)
        else {
            return Err(self.error(ErrorKind::WrongScope));
        };
        self.transport.typing_activity(context, watch, active)?;
        Ok(ProtocolReply::TypingAccepted)
    }
    pub(super) fn poll_typing(
        &self,
        context: &OperationContext,
        id: ConversationId,
    ) -> Result<ProtocolReply, PortError> {
        let token = RoomToken::parse(self.check_room(id)?)
            .map_err(|error| self.error(transport::kind(error)))?;
        let mut people = Page::empty();
        let running = if let Some((_, watch)) = self
            .typing_watch
            .as_ref()
            .filter(|(current, _)| *current == id)
        {
            let running = self.transport.typing_matches(context, watch, &token);
            if running {
                for person in self
                    .transport
                    .typing_people(context, watch, &token)
                    .into_iter()
                    .take(16)
                {
                    let Some(author_key) =
                        super::messaging::actor_key(id, &person.actor_type, &person.actor_id)
                    else {
                        continue;
                    };
                    let Ok(name) = Text::new(&person.display_name) else {
                        continue;
                    };
                    let _ = people.try_push(TypingPerson { name, author_key });
                }
            }
            running
        } else {
            false
        };
        Ok(ProtocolReply::Typing(TypingObservation {
            conversation: id,
            running,
            people,
        }))
    }
}
