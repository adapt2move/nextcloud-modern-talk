//! Current-user status from Nextcloud's user_status OCS API.
//! Reads evaluate calendar status and never set the user online. Explicit writes
//! are account-scoped and a lost acknowledgement remains uncertain.
//! Protocol: https://docs.nextcloud.com/server/stable/developer_manual/client_apis/OCS/ocs-status-api.html
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

impl UserStatus {
    /// Unknown or absent wire values never imply that a user is available.
    pub fn from_wire(value: Option<&str>) -> Self {
        match value {
            Some("online") => Self::Online,
            Some("away") => Self::Away,
            Some("offline") => Self::Offline,
            Some("invisible") => Self::Invisible,
            Some("dnd") => Self::DoNotDisturb,
            _ => Self::Unknown,
        }
    }
}

/// Explicit choices supported by the account picker.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum StatusChoice {
    Online,
    Away,
    DoNotDisturb,
    Invisible,
}

impl StatusChoice {
    fn wire(self) -> &'static str {
        match self {
            Self::Online => "online",
            Self::Away => "away",
            Self::DoNotDisturb => "dnd",
            Self::Invisible => "invisible",
        }
    }
}

fn decode(value: &Value, user: &str) -> Result<UserStatus, ErrorKind> {
    if string(value.get("userId"), 256)?.as_deref() != Some(user) {
        return Err(ErrorKind::WrongScope);
    }
    Ok(UserStatus::from_wire(
        string(value.get("status"), 32)?.as_deref(),
    ))
}

impl Client {
    /// Explicit status mutation. The caller checks user_status.enabled.
    /// Never replay after an uncertain dispatch; read the current status instead.
    pub async fn set_own_user_status(
        &mut self,
        context: &OperationContext,
        choice: StatusChoice,
    ) -> Result<UserStatus, Error> {
        self.validate(context)?;
        let response = self
            .transport
            .ocs(
                context,
                Method::PUT,
                "ocs/v2.php/apps/user_status/api/v1/user_status/status",
                &[],
                &[("statusType", choice.wire().to_owned())],
                false,
            )
            .await
            .map_err(|error| self.mutation_error(error))?;
        let status = decode(&response.data, self.transport.user())
            .map_err(|_| self.error(ErrorKind::Uncertain))?;
        self.validate(context)
            .map_err(|error| self.mutation_error(error))?;
        Ok(status)
    }

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
        assert_eq!(UserStatus::from_wire(None), UserStatus::Unknown);
        assert_eq!(
            UserStatus::from_wire(Some("invisible")),
            UserStatus::Invisible
        );
        assert_eq!(UserStatus::from_wire(Some("future")), UserStatus::Unknown);
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
