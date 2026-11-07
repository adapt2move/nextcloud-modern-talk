//! Versioned, bounded local intents. Runtime scopes and permission grants are
//! deliberately absent: every dispatch resolves room keys and rechecks policy.
use super::{DataError, OutboxKind, OutboxPhase, OutboxRecord, codec};
use serde::{Deserialize, Serialize};
use talk_core::{bounded::Text, ids::OperationId, preferences::navigation::NavigationRoomKey};

#[derive(Clone, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct OutboxOptions {
    pub reply_to: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub private_reply: Option<OutboxPrivateReply>,
    pub thread: Option<u64>,
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        with = "codec::optional_text"
    )]
    pub thread_title: Option<Text<256>>,
    pub silent: bool,
}
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct OutboxPrivateReply {
    #[serde(with = "codec::room_key")]
    pub source: NavigationRoomKey,
    pub message: u64,
    #[serde(with = "codec::text")]
    pub author: Text<128>,
    #[serde(with = "codec::text")]
    pub body: Text<512>,
}
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(tag = "action", rename_all = "snake_case", deny_unknown_fields)]
pub enum OutboxIntent {
    MarkRead {
        message: u64,
    },
    ShareFile {
        #[serde(with = "codec::text")]
        path: Text<4096>,
        #[serde(with = "codec::text")]
        name: Text<255>,
        #[serde(with = "codec::text")]
        caption: Text<4096>,
        options: OutboxOptions,
        voice: bool,
    },
    Favorite {
        value: bool,
    },
    Archived {
        value: bool,
    },
    Notifications {
        level: u8,
    },
    PollVote {
        poll: u64,
        #[serde(with = "codec::page")]
        options: talk_core::bounded::Page<u16, 100>,
    },
    Send {
        #[serde(with = "codec::text")]
        body: Text<4096>,
        options: OutboxOptions,
    },
    Edit {
        message: u64,
        #[serde(with = "codec::text")]
        body: Text<4096>,
    },
    Delete {
        message: u64,
    },
    Reaction {
        message: u64,
        #[serde(with = "codec::text")]
        emoji: Text<64>,
        add: bool,
    },
    Pin {
        message: u64,
        pinned: bool,
    },
    Forward {
        message: u64,
        #[serde(with = "codec::room_key")]
        destination: NavigationRoomKey,
        #[serde(with = "codec::text")]
        author: Text<256>,
        #[serde(with = "codec::text")]
        body: Text<4096>,
    },
    Reminder {
        message: u64,
        timestamp: Option<u64>,
    },
    Schedule {
        #[serde(with = "codec::text")]
        body: Text<4096>,
        timestamp: u64,
        options: OutboxOptions,
    },
    CancelScheduled {
        message: u64,
    },
    MarkUnread,
    RenameThread {
        root: u64,
        #[serde(with = "codec::text")]
        title: Text<256>,
    },
    ThreadNotifications {
        root: u64,
        level: i8,
    },
    EditScheduled {
        id: u64,
        #[serde(with = "codec::text")]
        body: Text<4096>,
        timestamp: u64,
        silent: bool,
    },
}
impl OutboxIntent {
    pub fn kind(&self) -> OutboxKind {
        match self {
            Self::MarkRead { .. } => OutboxKind::MarkRead,
            Self::ShareFile { .. } => OutboxKind::ShareFile,
            Self::Favorite { .. } => OutboxKind::Favorite,
            Self::Archived { .. } => OutboxKind::Archived,
            Self::Notifications { .. } => OutboxKind::Notifications,
            Self::PollVote { .. } => OutboxKind::PollVote,
            Self::Send { .. } => OutboxKind::Send,
            Self::Edit { .. } => OutboxKind::Edit,
            Self::Delete { .. } => OutboxKind::Delete,
            Self::Reaction { .. } => OutboxKind::Reaction,
            Self::Pin { .. } => OutboxKind::Pin,
            Self::Forward { .. } => OutboxKind::Forward,
            Self::Reminder { .. } => OutboxKind::Reminder,
            Self::Schedule { .. } => OutboxKind::Schedule,
            Self::CancelScheduled { .. } => OutboxKind::CancelScheduled,
            Self::MarkUnread => OutboxKind::MarkUnread,
            Self::RenameThread { .. } => OutboxKind::RenameThread,
            Self::ThreadNotifications { .. } => OutboxKind::ThreadNotifications,
            Self::EditScheduled { .. } => OutboxKind::EditScheduled,
        }
    }
    fn valid(&self) -> bool {
        let options = |o: &OutboxOptions| {
            o.reply_to != Some(0)
                && o.thread != Some(0)
                && o.thread_title.as_ref().is_none_or(|title| {
                    !title.as_str().trim().is_empty()
                        && o.reply_to.is_none()
                        && o.private_reply.is_none()
                        && o.thread.is_none()
                })
                && o.private_reply.as_ref().is_none_or(|private| {
                    private.message > 0 && o.reply_to.is_none() && o.thread.is_none()
                })
        };
        match self {
            Self::MarkRead { message } => *message > 0,
            Self::MarkUnread | Self::Favorite { .. } | Self::Archived { .. } => true,
            Self::Notifications { level } => *level <= 3,
            Self::PollVote { poll, options } => {
                *poll > 0
                    && options.as_slice().iter().all(|id| *id < 100)
                    && !options
                        .as_slice()
                        .iter()
                        .enumerate()
                        .any(|(i, id)| options.as_slice()[..i].contains(id))
            }
            Self::RenameThread { root, title } => *root > 0 && !title.as_str().trim().is_empty(),
            Self::ThreadNotifications { root, level } => *root > 0 && (0..=3).contains(level),
            Self::ShareFile {
                path,
                name,
                options: o,
                ..
            } => {
                path.as_str().starts_with('/')
                    && !path.as_str().contains("://")
                    && !path.as_str().split('/').any(|part| part == "..")
                    && !path.as_str().chars().any(char::is_control)
                    && !name.as_str().is_empty()
                    && options(o)
                    && o.private_reply.is_none()
                    && o.thread_title.is_none()
            }
            Self::Send { body, options: o } => !body.as_str().trim().is_empty() && options(o),
            Self::Schedule {
                body,
                timestamp,
                options: o,
            } => {
                !body.as_str().trim().is_empty()
                    && *timestamp > 0
                    && o.private_reply.is_none()
                    && o.thread_title.is_none()
                    && options(o)
            }
            Self::Edit { message, body } => *message > 0 && !body.as_str().trim().is_empty(),
            Self::EditScheduled {
                id,
                body,
                timestamp,
                ..
            } => *id > 0 && *timestamp > 0 && !body.as_str().trim().is_empty(),
            Self::Reaction { message, emoji, .. } => {
                *message > 0 && !emoji.as_str().trim().is_empty()
            }
            Self::Reminder { message, timestamp } => *message > 0 && *timestamp != Some(0),
            Self::Delete { message }
            | Self::Pin { message, .. }
            | Self::Forward { message, .. }
            | Self::CancelScheduled { message } => *message > 0,
        }
    }
}
impl OutboxRecord {
    pub fn from_intent(
        operation: OperationId,
        room: NavigationRoomKey,
        intent: &OutboxIntent,
    ) -> Result<Self, DataError> {
        if operation.as_local() == 0 || !intent.valid() {
            return Err(DataError::InvalidInput);
        }
        // Input fields are bounded before serialization; worst-case escaped body
        // fits the fixed payload limit, including the schema tag and metadata.
        let value = serde_json::to_string(&(1u8, intent)).map_err(|_| DataError::InvalidInput)?;
        Ok(Self {
            operation,
            room,
            kind: intent.kind(),
            phase: OutboxPhase::Queued,
            payload: Text::new(&value).map_err(|_| DataError::Capacity)?,
        })
    }
    pub fn intent(&self) -> Result<OutboxIntent, DataError> {
        let (version, intent): (u8, OutboxIntent) =
            serde_json::from_str(self.payload.as_str()).map_err(|_| DataError::Corrupt)?;
        if version != 1 || !intent.valid() || intent.kind() != self.kind {
            return Err(DataError::Corrupt);
        }
        Ok(intent)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mark_unread_intent_roundtrips_without_conversation_authority() {
        let room = NavigationRoomKey::new(&format!("r1:{:032x}", 7)).unwrap();
        let record =
            OutboxRecord::from_intent(OperationId::from_local(4), room, &OutboxIntent::MarkUnread)
                .unwrap();
        assert_eq!(record.kind, OutboxKind::MarkUnread);
        assert_eq!(record.intent(), Ok(OutboxIntent::MarkUnread));
    }

    #[test]
    fn thread_changes_roundtrip_without_persisting_permission() {
        let room = NavigationRoomKey::new(&format!("r1:{:032x}", 7)).unwrap();
        for intent in [
            OutboxIntent::RenameThread {
                root: 42,
                title: Text::new("Planning").unwrap(),
            },
            OutboxIntent::ThreadNotifications { root: 42, level: 3 },
        ] {
            let record =
                OutboxRecord::from_intent(OperationId::from_local(5), room.clone(), &intent)
                    .unwrap();
            assert_eq!(record.intent(), Ok(intent));
        }
        for invalid in [
            OutboxIntent::RenameThread {
                root: 0,
                title: Text::new("Planning").unwrap(),
            },
            OutboxIntent::RenameThread {
                root: 42,
                title: Text::new(" ").unwrap(),
            },
            OutboxIntent::ThreadNotifications { root: 42, level: 4 },
        ] {
            assert!(
                OutboxRecord::from_intent(OperationId::from_local(5), room.clone(), &invalid,)
                    .is_err()
            );
        }
    }

    #[test]
    fn thread_root_intent_is_bounded_and_cannot_be_combined_with_a_reply() {
        let room = NavigationRoomKey::new(&format!("r1:{:032x}", 7)).unwrap();
        let options = OutboxOptions {
            thread_title: Some(Text::new("Planning").unwrap()),
            ..Default::default()
        };
        let root = OutboxIntent::Send {
            body: Text::new("Planning details").unwrap(),
            options: options.clone(),
        };
        let record =
            OutboxRecord::from_intent(OperationId::from_local(5), room.clone(), &root).unwrap();
        assert_eq!(record.intent(), Ok(root));
        let invalid = OutboxIntent::Send {
            body: Text::new("Reply").unwrap(),
            options: OutboxOptions {
                reply_to: Some(3),
                ..options
            },
        };
        assert_eq!(
            OutboxRecord::from_intent(OperationId::from_local(6), room, &invalid),
            Err(DataError::InvalidInput)
        );
    }

    #[test]
    fn private_reply_survives_restart_and_cannot_be_combined_or_scheduled() {
        let room = |id| NavigationRoomKey::new(&format!("r1:{id:032x}")).unwrap();
        let private = OutboxPrivateReply {
            source: room(2),
            message: 17,
            author: Text::new("Mara").unwrap(),
            body: Text::new("A quoted message").unwrap(),
        };
        let options = OutboxOptions {
            private_reply: Some(private.clone()),
            ..Default::default()
        };
        let send = OutboxIntent::Send {
            body: Text::new("Thanks!").unwrap(),
            options: options.clone(),
        };
        let record = OutboxRecord::from_intent(OperationId::from_local(7), room(1), &send).unwrap();
        assert_eq!(record.intent(), Ok(send));
        let mixed = OutboxIntent::Send {
            body: Text::new("Thanks!").unwrap(),
            options: OutboxOptions {
                reply_to: Some(3),
                ..options.clone()
            },
        };
        assert_eq!(
            OutboxRecord::from_intent(OperationId::from_local(8), room(1), &mixed),
            Err(DataError::InvalidInput)
        );
        let scheduled = OutboxIntent::Schedule {
            body: Text::new("Thanks!").unwrap(),
            timestamp: 123,
            options,
        };
        assert_eq!(
            OutboxRecord::from_intent(OperationId::from_local(9), room(1), &scheduled),
            Err(DataError::InvalidInput)
        );
        let legacy: OutboxOptions =
            serde_json::from_str(r#"{"reply_to":null,"thread":null,"silent":false}"#).unwrap();
        assert!(legacy.private_reply.is_none());
    }
    #[test]
    fn restart_roundtrip_and_worst_case_body_remain_bounded() {
        let intent = OutboxIntent::Send {
            body: Text::new(&"\u{0001}".repeat(4096)).unwrap(),
            options: OutboxOptions::default(),
        };
        let record = OutboxRecord::from_intent(
            OperationId::from_local(1),
            NavigationRoomKey::new(&format!("r1:{:032x}", 1)).unwrap(),
            &intent,
        )
        .unwrap();
        assert_eq!(record.intent(), Ok(intent));
        let mut corrupt = record;
        corrupt.kind = OutboxKind::Delete;
        assert_eq!(corrupt.intent(), Err(DataError::Corrupt));
    }
    #[test]
    fn queued_settings_votes_and_cloud_publication_roundtrip_with_no_runtime_or_local_file_authority()
     {
        let room = NavigationRoomKey::new(&format!("r1:{:032x}", 1)).unwrap();
        let mut options = talk_core::bounded::Page::empty();
        options.try_push(2).unwrap();
        for intent in [
            OutboxIntent::Favorite { value: true },
            OutboxIntent::Archived { value: false },
            OutboxIntent::Notifications { level: 2 },
            OutboxIntent::PollVote { poll: 7, options },
            OutboxIntent::ShareFile {
                path: Text::new("/Documents/plan.pdf").unwrap(),
                name: Text::new("plan.pdf").unwrap(),
                caption: Text::new("caption").unwrap(),
                options: OutboxOptions::default(),
                voice: false,
            },
        ] {
            let record =
                OutboxRecord::from_intent(OperationId::from_local(1), room.clone(), &intent)
                    .unwrap();
            assert_eq!(record.intent(), Ok(intent));
        }
        for path in [
            "file:///tmp/plan.pdf",
            "/Documents/../secret",
            "https://cloud.invalid/f",
        ] {
            assert!(
                OutboxRecord::from_intent(
                    OperationId::from_local(1),
                    room.clone(),
                    &OutboxIntent::ShareFile {
                        path: Text::new(path).unwrap(),
                        name: Text::new("plan.pdf").unwrap(),
                        caption: Text::new("").unwrap(),
                        options: OutboxOptions::default(),
                        voice: false
                    }
                )
                .is_err()
            );
        }
    }
}
