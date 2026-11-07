//! Bounded local-server federation invitation fields. Remote credentials and
//! server URLs stay on the server and are never retained by presentation.
use crate::{ErrorKind, models::RoomToken};
use serde_json::Value;

/// Only capability data needed to qualify the proxied basic chat route.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RemoteCapabilities {
    pub features: Vec<String>,
    pub message_limit: Option<u64>,
}
impl RemoteCapabilities {
    pub(crate) fn decode(value: &Value) -> Result<Self, ErrorKind> {
        let features = value
            .get("features")
            .and_then(Value::as_array)
            .ok_or(ErrorKind::Malformed)?;
        if features.len() > 256 {
            return Err(ErrorKind::TooLarge);
        }
        let mut result = Vec::with_capacity(features.len());
        for feature in features {
            let feature = feature.as_str().ok_or(ErrorKind::Malformed)?;
            if feature.len() > 128 {
                return Err(ErrorKind::TooLarge);
            }
            result.push(feature.to_owned());
        }
        let message_limit = value["config"]["chat"]
            .get("max-length")
            .and_then(Value::as_u64)
            .filter(|n| *n > 0);
        Ok(Self {
            features: result,
            message_limit,
        })
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Invitation {
    pub id: u64,
    pub local_token: RoomToken,
    pub user: String,
    pub room_name: String,
    pub inviter: String,
}
impl Invitation {
    pub(crate) fn decode(value: &Value) -> Result<Self, ErrorKind> {
        let id = value
            .get("id")
            .and_then(Value::as_u64)
            .filter(|id| *id > 0)
            .ok_or(ErrorKind::Malformed)?;
        let field = |key: &str, max: usize| -> Result<String, ErrorKind> {
            let value = value
                .get(key)
                .and_then(Value::as_str)
                .ok_or(ErrorKind::Malformed)?;
            if value.len() > max {
                return Err(ErrorKind::TooLarge);
            }
            Ok(value.to_owned())
        };
        let token = field("localToken", 30)?;
        Ok(Self {
            id,
            local_token: RoomToken::parse(&token)?,
            user: field("userId", 512)?,
            room_name: field("roomName", 256)?,
            inviter: field("inviterDisplayName", 256)?,
        })
    }
}
