//! Account-wide read-only message observation, owned by the admitted account.
//! Pins: Talk 24 0d736588dcfb42becf0e5688e0847e35d5b5efa2 and Talk 25
//! eed360d176edcb6888dad60965cb694e6bf62ae3, lib/Notification/Notifier.php.
//! Notifications f106a3fedaaca27c54a5b701ad54f8b688e628ec,
//! lib/Controller/EndpointController.php. No translated subject/action URL is trusted.
use super::{projection::Project, *};
use nextcloud_talk_api as api;
use std::time::{Duration, Instant};

const BATCH: usize = 4;
const FRESH: Duration = Duration::from_secs(60);

#[derive(Clone, Default)]
pub(super) struct Observation {
    cursor: Option<u64>,
    at: Option<Instant>,
    since: u64,
    // One high-water mark per typed room, capped like the account's 256 room handles.
    // Evicting individual notification IDs could otherwise replay older messages.
    messages: Vec<(api::models::RoomToken, u64)>,
}
impl Observation {
    fn begin(&mut self, high_water: u64, now: Instant, wall: u64) -> bool {
        let baseline = self.cursor.is_none()
            || self
                .at
                .is_none_or(|at| now.saturating_duration_since(at) >= FRESH);
        if baseline {
            self.cursor = Some(self.cursor.unwrap_or(0).max(high_water));
            self.since = wall;
        }
        self.at = Some(now);
        baseline
    }
    fn consume_message(&mut self, room: api::models::RoomToken, message: u64) -> bool {
        if let Some((_, last)) = self.messages.iter_mut().find(|(id, _)| *id == room) {
            if message <= *last {
                return false;
            }
            *last = message;
        } else {
            if self.messages.len() == 256 {
                return false;
            }
            self.messages.push((room, message));
        }
        true
    }
}

impl NextcloudClient {
    pub(super) async fn poll_message_notifications(
        &mut self,
        context: &OperationContext,
    ) -> Result<ProtocolReply, PortError> {
        self.transport.validate(context)?;
        let list = self.transport.message_notifications(context).await?;
        let mut result = talk_core::protocol::MessageNotifications {
            scope: context.scope,
            notices: Page::empty(),
            changed: false,
        };
        if self
            .message_notifications
            .begin(list.high_water, Instant::now(), unix_seconds())
        {
            // Existing message IDs remain silent even if the server later assigns
            // a different notification ID to the same message.
            for notice in &list.messages {
                if let Some(message) = notice.message {
                    self.message_notifications
                        .consume_message(notice.room.clone(), message);
                }
            }
            return Ok(ProtocolReply::MessageNotifications(result));
        }
        let after = self.message_notifications.cursor.unwrap_or(0);
        let candidates: Vec<_> = list
            .messages
            .into_iter()
            .filter(|n| n.id > after)
            .take(BATCH)
            .collect();
        result.changed = !candidates.is_empty();
        if candidates.is_empty() {
            self.message_notifications.cursor = Some(after.max(list.high_water));
            return Ok(ProtocolReply::MessageNotifications(result));
        }
        let caps = self.capabilities(context).await?;
        self.supported(&caps)?;
        let mut observed_state = self.message_notifications.clone();
        for notice in candidates {
            // Consume suppressed/invalid events too. A mute change must not replay
            // them later. A transport failure leaves this event for the next poll.
            let observed = self.message_notice(context, &caps, &notice).await;
            match observed {
                Ok(Some(value)) => {
                    if observed_state.consume_message(notice.room.clone(), value.message) {
                        result
                            .notices
                            .try_push(value)
                            .map_err(|_| self.error(ErrorKind::TooLarge))?;
                    }
                }
                Ok(None) => {}
                Err(e) if matches!(e.kind, ErrorKind::NotFound | ErrorKind::PermissionDenied) => {}
                Err(e) => return Err(e),
            }
            if let Some(message) = notice.message {
                observed_state.consume_message(notice.room.clone(), message);
            }
            observed_state.cursor = Some(notice.id);
        }
        self.transport.validate(context)?;
        self.message_notifications = observed_state;
        Ok(ProtocolReply::MessageNotifications(result))
    }

    async fn message_notice(
        &mut self,
        context: &OperationContext,
        caps: &Capabilities,
        notice: &api::notifications::MessageNotification,
    ) -> Result<Option<talk_core::protocol::MessageNotice>, PortError> {
        if !notice.eligible
            || notice.timestamp < self.message_notifications.since
            || notice.timestamp.saturating_add(FRESH.as_secs()) < unix_seconds()
        {
            return Ok(None);
        }
        let api::Reply::Room(wire_room) = self
            .transport
            .api_request(context, api::Request::Room(notice.room.clone()))
            .await?
        else {
            return Err(self.error(ErrorKind::Malformed));
        };
        if api::models::RoomToken::parse(&wire_room.token).as_ref() != Ok(&notice.room)
            || wire_room.remote
        {
            return Err(self.error(ErrorKind::WrongScope));
        }
        let data = wire_room.project();
        let id = self.bind_room(&data)?;
        let room = wire::room(&data, id, caps).map_err(|k| self.error(k))?;
        if room.unknown_restrictions
            || room.actions.read_messages != Decision::Allowed
            || !matches!(data["notificationLevel"].as_i64(), Some(0..=2))
        {
            return Ok(None);
        }
        // A context/history read cannot advance read markers or clear notifications.
        // Only a new notification triggers this bounded read, never one poll per room.
        let request = if let Some(message) = notice.message {
            api::Request::Chat {
                room: notice.room.clone(),
                command: api::chat::Command::Context(message),
            }
        } else if room.sensitive || room.classified {
            api::Request::History {
                room: notice.room.clone(),
                before: None,
            }
        } else {
            return Ok(None);
        };
        let api::Reply::History { messages, .. } =
            self.transport.api_request(context, request).await?
        else {
            return Err(self.error(ErrorKind::Malformed));
        };
        let message = messages.iter().filter(|m| {
            api::models::RoomToken::parse(&m.token).as_ref() == Ok(&notice.room)
                && notice.message.is_none_or(|id| m.id == id)
                // Sensitive rows hide their message ID. Only an actual new comment
                // at the notification's time can qualify; an old context cannot.
                && m.timestamp <= notice.timestamp
                && notice.timestamp.saturating_sub(m.timestamp) <= if notice.message.is_some() { 30 } else { 0 }
                && eligible_message(m, &room, &data, self.transport.user())
        }).max_by_key(|m| m.id);
        let Some(message) = message else {
            return Ok(None);
        };
        let redacted = room.sensitive || room.classified;
        Ok(Some(talk_core::protocol::MessageNotice {
            conversation: id,
            message: message.id,
            redacted,
            // Protected strings never enter the UI queue, even when previews are enabled.
            title: if redacted {
                Text::new("").expect("empty")
            } else {
                snippet(room.name.as_str())
            },
            body: if redacted {
                Text::new("").expect("empty")
            } else {
                snippet(&format!(
                    "{}: {}",
                    message.author,
                    message.message.as_deref().unwrap_or("")
                ))
            },
        }))
    }
}

fn eligible_message(
    message: &api::models::Message,
    room: &Conversation,
    data: &Value,
    user: &str,
) -> bool {
    let now = unix_seconds();
    if message.kind.as_deref() != Some("comment")
        || message.system.as_deref() != Some("")
        || message.update.is_some()
        || message.edited.is_some_and(|at| at > 0)
        || message.silent != Some(false)
        || message.actor_type.as_deref() != Some("users")
        || message
            .actor_id
            .as_deref()
            .is_none_or(|id| id.is_empty() || id == user)
        || message.expires.is_some_and(|at| at > 0 && at <= now)
        || room.unknown_restrictions
        || room.actions.read_messages != Decision::Allowed
    {
        return false;
    }
    match data["notificationLevel"].as_i64() {
        // Defaults are resolved by the fresh authenticated Notifications source.
        Some(0 | 1) => true,
        Some(2) => message.rich.iter().any(|object| {
            object.kind == "user"
                && object.id.as_deref() == Some(user)
                && message
                    .message
                    .as_ref()
                    .is_some_and(|text| text.contains(&format!("{{{}}}", object.placeholder)))
        }),
        _ => false,
    }
}
fn unix_seconds() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(u64::MAX, |d| d.as_secs())
}
fn snippet<const N: usize>(s: &str) -> Text<N> {
    let mut result = String::new();
    for c in s.chars().filter(|c| !c.is_control()) {
        if result.len() + c.len_utf8() > N {
            break;
        }
        result.push(c);
    }
    Text::new(&result).expect("bounded UTF-8 projection")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn room(local: u128) -> api::models::RoomToken {
        api::models::RoomToken::parse(&format!("{local:04x}")).unwrap()
    }
    #[test]
    fn background_baseline_replay_and_suspend_do_not_emit_historical_notifications() {
        let now = Instant::now();
        let mut state = Observation::default();
        assert!(state.begin(40, now, 100));
        assert_eq!(state.cursor, Some(40));
        assert!(!state.begin(45, now + Duration::from_secs(5), 105));
        assert_eq!(
            state.cursor,
            Some(40),
            "new events must be consumed, not skipped"
        );
        assert!(state.consume_message(room(1), 80));
        assert!(!state.consume_message(room(1), 80));
        assert!(!state.consume_message(room(1), 79));
        assert!(
            state.consume_message(room(2), 80),
            "room scopes are independent"
        );
        assert!(state.begin(100, now + Duration::from_secs(70), 170));
        assert_eq!(state.cursor, Some(100));
        assert_eq!(state.since, 170);
        assert!(state.begin(2, now + Duration::from_secs(140), 240));
        assert_eq!(
            state.cursor,
            Some(100),
            "a replay cannot move the cursor backwards"
        );
    }
    #[test]
    fn background_message_dedup_is_finite_without_evicting_replay_protection() {
        let mut state = Observation::default();
        for local in 0..256 {
            assert!(state.consume_message(room(local), 1));
        }
        assert!(!state.consume_message(room(256), 1));
        assert_eq!(state.messages.len(), 256);
        assert!(state.consume_message(room(0), 2));
        assert!(!state.consume_message(room(0), 1));
    }
}
