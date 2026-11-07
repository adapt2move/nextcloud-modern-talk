//! Account-wide upcoming reminders. Wire room tokens stay with the account owner.
use super::*;
use nextcloud_talk_api as api;
use talk_core::protocol::messaging::{AccountReminders, MessageReminder};

impl NextcloudClient {
    pub(super) async fn upcoming_reminders(
        &mut self,
        context: &OperationContext,
    ) -> Result<ProtocolReply, PortError> {
        let caps = self.capabilities(context).await?;
        self.supported(&caps)?;
        let mut result = AccountReminders {
            scope: context.scope,
            supported: caps.has("upcoming-reminders"),
            complete: true,
            reminders: Page::empty(),
        };
        if !result.supported {
            return Ok(ProtocolReply::UpcomingReminders(result));
        }
        let api::Reply::Chat(api::chat::Response::Reminders(values)) = self
            .transport
            .api_request(context, api::Request::UpcomingReminders)
            .await?
        else {
            return Err(self.error(ErrorKind::Malformed));
        };
        // Talk 24/25's Reminder::NUM_UPCOMING_REMINDERS is ten and the
        // endpoint has no pagination. A full page may omit later reminders.
        if values.len() >= 10 {
            result.complete = false;
        }
        for value in values {
            let Some(room) = self.rooms.iter().find(|room| room.token == value.token) else {
                // The room list can be partial. Never turn an unbound token into
                // a navigation handle or pretend this observation is complete.
                result.complete = false;
                continue;
            };
            if value.message == 0 {
                return Err(self.error(ErrorKind::Malformed));
            }
            result
                .reminders
                .try_push(MessageReminder {
                    message: MessageId {
                        conversation: room.id,
                        sequence: value.message,
                    },
                    timestamp: value.timestamp,
                })
                .map_err(|_| self.error(ErrorKind::TooLarge))?;
        }
        self.transport.validate(context)?;
        Ok(ProtocolReply::UpcomingReminders(result))
    }
}
