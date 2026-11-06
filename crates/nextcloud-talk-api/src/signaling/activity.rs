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
            Activity::RaisedHand {
                state: true,
                timestamp: 42,
            },
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

#[cfg(test)]
mod batch_tests {
    use super::*;
    use crate::signaling::{Event, parse_batch};
    use serde_json::json;

    #[test]
    fn force_mute_is_only_projected_for_the_local_session() {
        let row = |target: &str| {
            json!({"type":"message", "data": json!({
            "from":"peer", "to":"local", "roomType":"video", "type":"control",
            "payload":{"action":"forceMute", "peerId":target}
        }).to_string()})
        };
        let batch = parse_batch(&json!([row("local"), row("other")]), "local", 1).unwrap();
        assert_eq!(batch.events().len(), 1);
        assert!(matches!(&batch.events()[0], Event::ForceMuteBy { .. }));
    }

    #[test]
    fn activity_is_scoped_and_bounded_in_batch() {
        let row = |to: &str| {
            json!({"type":"message", "data": json!({
            "from":"peer", "to":to, "roomType":"video", "type":"raiseHand",
            "payload":{"state":true,"timestamp":42}
        }).to_string()})
        };
        let batch = parse_batch(&json!([row("local")]), "local", 1).unwrap();
        assert!(matches!(
            &batch.events()[0],
            Event::Activity {
                activity: Activity::RaisedHand {
                    state: true,
                    timestamp: 42
                },
                ..
            }
        ));
        assert!(parse_batch(&json!([row("other")]), "local", 1).is_err());
        assert_eq!(
            parse_batch(&json!(vec![row("local"); 17]), "local", 1)
                .unwrap()
                .events()
                .len(),
            16
        );
    }
}
