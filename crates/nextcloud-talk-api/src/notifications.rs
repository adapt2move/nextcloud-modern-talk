//! Read-only account notification observation. No action links or translated subjects
//! become routes or event types. Notifications v2 exposes a bounded recent window,
//! not a delivery log; periodic room reconciliation remains authoritative.
use crate::{Client, Error, ErrorKind, Reply, Request, models::RoomToken, scope::OperationContext};
use serde_json::Value;

#[derive(Clone, Debug)]
pub struct MessageNotification {
    pub id: u64,
    /// The server's typed object, never inferred from translated subject text.
    pub reminder: bool,
    pub room: RoomToken,
    /// Sensitive notifications deliberately omit the message identifier.
    pub message: Option<u64>,
    pub timestamp: u64,
    pub eligible: bool,
}

#[derive(Clone, Debug)]
pub struct ServerTestNotification {
    pub id: u64,
    pub timestamp: u64,
    pub eligible: bool,
}

pub struct MessageNotifications {
    /// Includes unrelated notification IDs, so ignored/replayed rows cannot move
    /// the account's cursor backwards.
    pub high_water: u64,
    pub messages: Vec<MessageNotification>,
    pub server_tests: Vec<ServerTestNotification>,
}

impl Client {
    pub async fn message_notifications(
        &mut self,
        context: &OperationContext,
    ) -> Result<MessageNotifications, Error> {
        let Reply::Account(account) = self.request(context, Request::Account).await? else {
            return Err(self.error(ErrorKind::Malformed));
        };
        let Reply::Capabilities(caps) = self.request(context, Request::Capabilities).await? else {
            return Err(self.error(ErrorKind::Malformed));
        };
        if !caps.supported_protocol() || !caps.notification_endpoints.iter().any(|s| s == "list") {
            return Err(self.error(ErrorKind::Unsupported));
        }
        let response = self.transport.notification_list(context).await?;
        if response.status == 204 {
            return Err(self.error(ErrorKind::Unsupported));
        }
        if response.status != 200 {
            return Err(self.error(ErrorKind::Malformed));
        }
        decode(
            &response.data,
            &account.id,
            response.notification_allowed == Some(true)
                && caps
                    .notification_endpoints
                    .iter()
                    .any(|s| s == "user-status"),
        )
        .map_err(|kind| self.error(kind))
    }
}

fn decode(data: &Value, user: &str, allowed: bool) -> Result<MessageNotifications, ErrorKind> {
    let rows = data.as_array().ok_or(ErrorKind::Malformed)?;
    if rows.len() > 100 {
        return Err(ErrorKind::TooLarge);
    }
    let mut ids = Vec::with_capacity(rows.len());
    let mut messages = Vec::new();
    let mut server_tests = Vec::new();
    for row in rows {
        let id = row["notification_id"]
            .as_u64()
            .filter(|id| *id > 0)
            .ok_or(ErrorKind::Malformed)?;
        if ids.contains(&id) {
            return Err(ErrorKind::Malformed);
        }
        ids.push(id);
        // TestPush persists an admin_notification_talk object before attempting
        // mobile push. It has no room/message authority; never interpret its copy
        // or action links as a Talk target. Respect server eligibility and DND.
        if row["app"].as_str() == Some("admin_notification_talk")
            && row["object_type"].as_str() == Some("admin_notifications")
        {
            if row["user"].as_str() != Some(user) {
                return Err(ErrorKind::WrongScope);
            }
            server_tests.push(ServerTestNotification {
                id,
                timestamp: timestamp(row["datetime"].as_str().ok_or(ErrorKind::Malformed)?)?,
                eligible: allowed && row["shouldNotify"].as_bool() == Some(true),
            });
            continue;
        }
        let reminder = match row["object_type"].as_str() {
            Some("chat") => false,
            Some("reminder") => true,
            _ => continue,
        };
        if row["app"].as_str() != Some("spreed") {
            continue;
        }
        if row["user"].as_str() != Some(user) {
            return Err(ErrorKind::WrongScope);
        }
        // Talk 24/25 also use `chat` for reactions. Their structured parameter
        // distinguishes them; the localized subject does not.
        if !reminder && row["subjectRichParameters"].get("reaction").is_some() {
            continue;
        }
        let object = row["object_id"].as_str().ok_or(ErrorKind::Malformed)?;
        let mut parts = object.split('/');
        let room = RoomToken::parse(parts.next().ok_or(ErrorKind::Malformed)?)?;
        let positive = |s: &str| {
            s.parse::<u64>()
                .ok()
                .filter(|id| *id > 0)
                .ok_or(ErrorKind::Malformed)
        };
        // Talk 24/25's Notifier appends the message and optional thread to
        // non-sensitive notification object IDs for both chat and reminders.
        // Push preparation may instead expose the ID in the rich subject.
        let object_message = parts.next().map(positive).transpose()?;
        if let Some(thread) = parts.next() {
            positive(thread)?;
        }
        if parts.next().is_some() {
            return Err(ErrorKind::Malformed);
        }
        let message = if reminder && object_message.is_none() {
            // Sensitive notifications omit the structured target. Never infer
            // it from translated text, an action URL, or another message.
            let Some(id) = row["subjectRichParameters"]["message"]["id"].as_str() else {
                continue;
            };
            Some(positive(id)?)
        } else {
            object_message
        };
        let timestamp = timestamp(row["datetime"].as_str().ok_or(ErrorKind::Malformed)?)?;
        messages.push(MessageNotification {
            id,
            reminder,
            room,
            message,
            timestamp,
            eligible: allowed && row["shouldNotify"].as_bool() == Some(true),
        });
    }
    messages.sort_unstable_by_key(|n| n.id);
    Ok(MessageNotifications {
        high_water: ids.into_iter().max().unwrap_or(0),
        messages,
        server_tests,
    })
}

/// PHP DateTime::format('c') at the pinned Notifications endpoint. Use the
/// existing cookie time dependency for calendar validation, without a new parser dependency.
fn timestamp(value: &str) -> Result<u64, ErrorKind> {
    use cookie::time::{Date, Month, Time, UtcOffset};
    let b = value.as_bytes();
    if b.len() != 25
        || !value.is_ascii()
        || b[4] != b'-'
        || b[7] != b'-'
        || b[10] != b'T'
        || b[13] != b':'
        || b[16] != b':'
        || !matches!(b[19], b'+' | b'-')
        || b[22] != b':'
    {
        return Err(ErrorKind::Malformed);
    }
    let number = |a, z| {
        value
            .get(a..z)
            .and_then(|s| s.parse::<u16>().ok())
            .ok_or(ErrorKind::Malformed)
    };
    let date = Date::from_calendar_date(
        i32::from(number(0, 4)?),
        Month::try_from(u8::try_from(number(5, 7)?).map_err(|_| ErrorKind::Malformed)?)
            .map_err(|_| ErrorKind::Malformed)?,
        u8::try_from(number(8, 10)?).map_err(|_| ErrorKind::Malformed)?,
    )
    .map_err(|_| ErrorKind::Malformed)?;
    let byte = |a, z| u8::try_from(number(a, z)?).map_err(|_| ErrorKind::Malformed);
    let time = Time::from_hms(byte(11, 13)?, byte(14, 16)?, byte(17, 19)?)
        .map_err(|_| ErrorKind::Malformed)?;
    let sign = if b[19] == b'-' { -1 } else { 1 };
    let hours = i8::try_from(number(20, 22)?).map_err(|_| ErrorKind::Malformed)?;
    let minutes = i8::try_from(number(23, 25)?).map_err(|_| ErrorKind::Malformed)?;
    let offset =
        UtcOffset::from_hms(sign * hours, sign * minutes, 0).map_err(|_| ErrorKind::Malformed)?;
    u64::try_from(date.with_time(time).assume_offset(offset).unix_timestamp())
        .map_err(|_| ErrorKind::Malformed)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn row(id: u64, object: &str) -> Value {
        json!({"notification_id":id,"app":"spreed","object_type":"chat",
            "object_id":object,"user":"alice","datetime":"2026-10-20T10:00:00+02:00",
            "shouldNotify":true,"subject":"Untrusted translated text",
            "link":"https://unrelated.invalid/","actions":[{"link":"https://unrelated.invalid/"}]})
    }
    #[test]
    fn talk_server_diagnostics_have_no_talk_target_and_keep_server_policy() {
        let mut test = row(15, "untrusted/not/a/room/target");
        test["app"] = json!("admin_notification_talk");
        test["object_type"] = json!("admin_notifications");
        let data = json!([test.clone()]);
        let list = decode(&data, "alice", true).unwrap();
        assert_eq!(list.high_water, 15);
        assert!(list.messages.is_empty());
        assert_eq!(list.server_tests.len(), 1);
        assert!(list.server_tests[0].eligible);
        assert!(!decode(&data, "alice", false).unwrap().server_tests[0].eligible);
        assert!(matches!(
            decode(&data, "bob", true),
            Err(ErrorKind::WrongScope)
        ));
        test["shouldNotify"] = Value::Null;
        assert!(
            !decode(&json!([test.clone()]), "alice", true)
                .unwrap()
                .server_tests[0]
                .eligible
        );
        test["app"] = json!("admin_notifications");
        assert!(
            decode(&json!([test]), "alice", true)
                .unwrap()
                .server_tests
                .is_empty()
        );
    }

    #[test]
    fn background_notifications_use_only_typed_targets_and_fresh_server_eligibility() {
        let data = json!([row(8, "testroom/42/41"), row(9, "privateroom")]);
        let list = decode(&data, "alice", true).unwrap();
        assert_eq!(list.high_water, 9);
        assert_eq!(list.messages[0].message, Some(42));
        assert_eq!(list.messages[1].message, None);
        assert!(list.messages[1].eligible);
        assert!(!decode(&data, "alice", false).unwrap().messages[0].eligible);
        assert!(matches!(
            decode(&data, "bob", true),
            Err(ErrorKind::WrongScope)
        ));
        let mut reaction = row(10, "testroom/42");
        reaction["subjectRichParameters"] = json!({"reaction":{"type":"highlight","id":"x"}});
        assert!(
            decode(&json!([reaction]), "alice", true)
                .unwrap()
                .messages
                .is_empty()
        );
    }
    #[test]
    fn reminders_require_a_structured_message_id_and_ignore_translated_text_and_urls() {
        let mut reminder = row(11, "testroom");
        reminder["object_type"] = json!("reminder");
        assert!(
            decode(&json!([reminder.clone()]), "alice", true)
                .unwrap()
                .messages
                .is_empty()
        );
        reminder["subjectRichParameters"] =
            json!({"message":{"type":"highlight", "id":"42", "name":"Private text"}});
        let parsed = decode(&json!([reminder]), "alice", true).unwrap();
        assert!(parsed.messages[0].reminder);
        assert_eq!(parsed.messages[0].message, Some(42));

        let mut endpoint = row(12, "testroom/43/40");
        endpoint["object_type"] = json!("reminder");
        let parsed = decode(&json!([endpoint.clone()]), "alice", true).unwrap();
        assert!(parsed.messages[0].reminder);
        assert_eq!(parsed.messages[0].message, Some(43));
        endpoint["object_id"] = json!("testroom/0");
        assert!(matches!(
            decode(&json!([endpoint]), "alice", true),
            Err(ErrorKind::Malformed)
        ));
    }

    #[test]
    fn notification_lists_reject_partial_oversized_duplicate_and_invalid_targets() {
        assert!(matches!(
            decode(&json!(vec![row(1, "testroom"); 101]), "alice", true),
            Err(ErrorKind::TooLarge)
        ));
        assert!(
            decode(
                &json!([row(1, "testroom"), row(1, "testroom")]),
                "alice",
                true
            )
            .is_err()
        );
        for object in [
            "testroom/0",
            "testroom/42/1/2",
            "../other",
            "testroom/not-an-id",
        ] {
            assert!(decode(&json!([row(1, object)]), "alice", true).is_err());
        }
        assert_eq!(
            timestamp("2026-10-20T10:00:00+02:00"),
            timestamp("2026-10-20T08:00:00+00:00")
        );
        assert!(timestamp("2026-02-30T08:00:00+00:00").is_err());
    }
}
