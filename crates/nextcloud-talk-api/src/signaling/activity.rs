//! Bounded call activity payloads. Routing and authority remain transport-owned.
use crate::ErrorKind;
use serde_json::{Value, json};

const REACTION_MAX: usize = 64;

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum Activity {
    RaisedHand { state: bool, timestamp: u64 },
    Reaction(String),
}

impl Activity {
    /// Decode a scoped Talk peer message's type and payload.
    pub fn decode(kind: &str, payload: &Value) -> Result<Self, ErrorKind> {
        match kind {
            "raiseHand" => Ok(Self::RaisedHand {
                state: payload["state"].as_bool().ok_or(ErrorKind::Malformed)?,
                timestamp: payload["timestamp"].as_u64().ok_or(ErrorKind::Malformed)?,
            }),
            "reaction" => {
                let reaction = payload["reaction"].as_str().ok_or(ErrorKind::Malformed)?;
                if reaction.is_empty()
                    || reaction.len() > REACTION_MAX
                    || reaction.chars().any(char::is_control)
                {
                    return Err(ErrorKind::Malformed);
                }
                Ok(Self::Reaction(reaction.to_owned()))
            }
            _ => Err(ErrorKind::Unsupported),
        }
    }

    /// Encode a peer message without selecting recipients.
    pub fn encode(&self) -> Result<(&'static str, Value), ErrorKind> {
        match self {
            Self::RaisedHand { state, timestamp } => {
                Ok(("raiseHand", json!({"state": state, "timestamp": timestamp})))
            }
            Self::Reaction(reaction) => {
                let payload = json!({"reaction": reaction});
                Self::decode("reaction", &payload)?;
                Ok(("reaction", payload))
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn activity_round_trips_and_rejects_invalid_payloads() {
        for activity in [
            Activity::RaisedHand { state: true, timestamp: 42 },
            Activity::Reaction("👍".into()),
        ] {
            let (kind, payload) = activity.encode().unwrap();
            assert_eq!(Activity::decode(kind, &payload).unwrap(), activity);
        }
        assert!(Activity::decode("raiseHand", &json!({"state": true})).is_err());
        assert!(Activity::decode("reaction", &json!({"reaction": ""})).is_err());
        assert!(Activity::decode("reaction", &json!({"reaction": "x".repeat(65)})).is_err());
        assert!(Activity::decode("reaction", &json!({"reaction": "\n"})).is_err());
    }
}
