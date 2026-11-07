//! Bounded local-server federation invitation fields. Remote credentials and
//! server URLs stay on the server and are never retained by presentation.
use crate::{ErrorKind, models::RoomToken};
use serde_json::Value;

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
