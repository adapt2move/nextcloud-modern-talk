//! Versioned, bounded local intents. Runtime scopes and permission grants are
//! deliberately absent: every dispatch resolves room keys and rechecks policy.
use super::{DataError, OutboxKind, OutboxPhase, OutboxRecord, codec};
use serde::{Deserialize, Serialize};
use talk_core::{bounded::Text, ids::OperationId, preferences::navigation::NavigationRoomKey};

#[derive(Clone, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct OutboxOptions {
    pub reply_to: Option<u64>,
    pub thread: Option<u64>,
    pub silent: bool,
}
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(tag = "action", rename_all = "snake_case", deny_unknown_fields)]
pub enum OutboxIntent {
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
}
impl OutboxIntent {
    pub fn kind(&self) -> OutboxKind {
        match self {
            Self::Send { .. } => OutboxKind::Send,
            Self::Edit { .. } => OutboxKind::Edit,
            Self::Delete { .. } => OutboxKind::Delete,
            Self::Reaction { .. } => OutboxKind::Reaction,
            Self::Pin { .. } => OutboxKind::Pin,
            Self::Forward { .. } => OutboxKind::Forward,
            Self::Reminder { .. } => OutboxKind::Reminder,
            Self::Schedule { .. } => OutboxKind::Schedule,
            Self::CancelScheduled { .. } => OutboxKind::CancelScheduled,
        }
    }
    fn valid(&self) -> bool {
        let options = |o: &OutboxOptions| o.reply_to != Some(0) && o.thread != Some(0);
        match self {
            Self::Send { body, options: o } => !body.as_str().trim().is_empty() && options(o),
            Self::Schedule {
                body,
                timestamp,
                options: o,
            } => !body.as_str().trim().is_empty() && *timestamp > 0 && options(o),
            Self::Edit { message, body } => *message > 0 && !body.as_str().trim().is_empty(),
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
}
