//! Read-only current-user status from Nextcloud's user_status OCS API.
//! The server evaluates calendar status here; this never sets the user online.
use crate::{Client, Error, ErrorKind, models::string, scope::OperationContext};
use reqwest::Method;
use serde_json::Value;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum UserStatus {
    Online,
    Away,
    Offline,
    Invisible,
    DoNotDisturb,
    Unknown,
}

fn decode(value: &Value, user: &str) -> Result<UserStatus, ErrorKind> {
    if string(value.get("userId"), 256)?.as_deref() != Some(user) {
        return Err(ErrorKind::WrongScope);
    }
    Ok(match string(value.get("status"), 32)?.as_deref() {
        Some("online") => UserStatus::Online,
        Some("away") => UserStatus::Away,
        Some("offline") => UserStatus::Offline,
        Some("invisible") => UserStatus::Invisible,
        Some("dnd") => UserStatus::DoNotDisturb,
        _ => UserStatus::Unknown,
    })
}

impl Client {
    /// One bounded read using the ordinary account lease, cookie jar and deadline.
    /// Missing/disabled user_status is an error, never evidence that DND is off.
    pub async fn own_user_status(
        &mut self,
        context: &OperationContext,
    ) -> Result<UserStatus, Error> {
        self.validate(context)?;
        let response = self
            .transport
            .ocs(
                context,
                Method::GET,
                "ocs/v2.php/apps/user_status/api/v1/user_status",
                &[],
                &[],
                false,
            )
            .await?;
        decode(&response.data, self.transport.user()).map_err(|kind| self.error(kind))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn status_is_bound_to_current_user_and_unknown_is_not_available() {
        assert_eq!(
            decode(
                &serde_json::json!({"userId":"alice","status":"dnd"}),
                "alice"
            ),
            Ok(UserStatus::DoNotDisturb)
        );
        assert_eq!(
            decode(
                &serde_json::json!({"userId":"alice","status":"online"}),
                "alice"
            ),
            Ok(UserStatus::Online)
        );
        assert_eq!(
            decode(
                &serde_json::json!({"userId":"bob","status":"online"}),
                "alice"
            ),
            Err(ErrorKind::WrongScope)
        );
        assert_eq!(
            decode(
                &serde_json::json!({"userId":"alice","status":"future-status"}),
                "alice"
            ),
            Ok(UserStatus::Unknown)
        );
        assert_eq!(
            decode(&serde_json::json!({"userId":"alice"}), "alice"),
            Ok(UserStatus::Unknown)
        );
    }
}
