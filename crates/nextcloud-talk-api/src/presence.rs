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

/// Bounded own-status fields returned by the server.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct StatusDetails {
    pub presence: UserStatus,
    pub icon: Option<String>,
    pub message: Option<String>,
    pub clear_at: Option<u64>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PredefinedStatus {
    pub id: String,
    pub icon: String,
    pub message: String,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum MessageChange {
    Custom {
        icon: Option<String>,
        message: String,
        clear_at: Option<u64>,
    },
    Predefined {
        id: String,
        clear_at: Option<u64>,
    },
    Clear,
}

fn details(value: &Value, user: &str) -> Result<StatusDetails, ErrorKind> {
    Ok(StatusDetails {
        presence: decode(value, user)?,
        icon: string(value.get("icon"), 64)?,
        message: string(value.get("message"), 320)?,
        clear_at: value.get("clearAt").and_then(Value::as_u64),
    })
}

fn predefined(value: &Value) -> Result<Vec<PredefinedStatus>, ErrorKind> {
    let rows = value.as_array().ok_or(ErrorKind::Malformed)?;
    if rows.len() > 32 {
        return Err(ErrorKind::TooLarge);
    }
    rows.iter()
        .map(|row| {
            let id = string(row.get("id"), 128)?
                .filter(|v| !v.is_empty() && !v.chars().any(char::is_control))
                .ok_or(ErrorKind::Malformed)?;
            Ok(PredefinedStatus {
                id,
                icon: string(row.get("icon"), 64)?.ok_or(ErrorKind::Malformed)?,
                message: string(row.get("message"), 320)?.ok_or(ErrorKind::Malformed)?,
            })
        })
        .collect()
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

    /// Read the current status without changing online activity.
    pub async fn own_user_status(
        &mut self,
        context: &OperationContext,
    ) -> Result<UserStatus, Error> {
        Ok(self.own_status_details(context).await?.presence)
    }

    pub async fn own_status_details(
        &mut self,
        context: &OperationContext,
    ) -> Result<StatusDetails, Error> {
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
        let result =
            details(&response.data, self.transport.user()).map_err(|kind| self.error(kind))?;
        self.validate(context)?;
        Ok(result)
    }

    /// Read at most 32 server-defined choices; unavailable lists do not imply an empty list.
    pub async fn predefined_statuses(
        &mut self,
        context: &OperationContext,
    ) -> Result<Vec<PredefinedStatus>, Error> {
        self.validate(context)?;
        let response = self
            .transport
            .ocs(
                context,
                Method::GET,
                "ocs/v2.php/apps/user_status/api/v1/predefined_statuses/",
                &[],
                &[],
                false,
            )
            .await?;
        let result = predefined(&response.data).map_err(|kind| self.error(kind))?;
        self.validate(context)?;
        Ok(result)
    }

    /// The caller checks capability and reconciles the result; never replay uncertainty.
    pub async fn set_status_message(
        &mut self,
        context: &OperationContext,
        change: &MessageChange,
    ) -> Result<(), Error> {
        self.validate(context)?;
        let mut form = Vec::new();
        let (method, route) = match change {
            MessageChange::Custom {
                icon,
                message,
                clear_at,
            } => {
                if message.chars().count() > 80
                    || message.len() > 320
                    || message.chars().any(char::is_control)
                    || icon
                        .as_ref()
                        .is_some_and(|s| s.len() > 64 || s.chars().any(char::is_control))
                {
                    return Err(self.error(ErrorKind::Malformed));
                }
                form.push(("message", message.clone()));
                if let Some(icon) = icon {
                    form.push(("statusIcon", icon.clone()));
                }
                if let Some(at) = clear_at {
                    form.push(("clearAt", at.to_string()));
                }
                (
                    Method::PUT,
                    "ocs/v2.php/apps/user_status/api/v1/user_status/message/custom",
                )
            }
            MessageChange::Predefined { id, clear_at } => {
                if id.is_empty() || id.len() > 128 || id.chars().any(char::is_control) {
                    return Err(self.error(ErrorKind::Malformed));
                }
                form.push(("messageId", id.clone()));
                if let Some(at) = clear_at {
                    form.push(("clearAt", at.to_string()));
                }
                (
                    Method::PUT,
                    "ocs/v2.php/apps/user_status/api/v1/user_status/message/predefined",
                )
            }
            MessageChange::Clear => (
                Method::DELETE,
                "ocs/v2.php/apps/user_status/api/v1/user_status/message",
            ),
        };
        let response = self
            .transport
            .ocs(context, method, route, &[], &form, false)
            .await
            .map_err(|error| self.mutation_error(error))?;
        if !matches!(change, MessageChange::Clear) {
            details(&response.data, self.transport.user())
                .map_err(|_| self.error(ErrorKind::Uncertain))?;
        }
        self.validate(context)
            .map_err(|error| self.mutation_error(error))?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn predefined_and_account_details_are_bounded_and_account_scoped() {
        let rows = serde_json::json!([{"id":"meeting","icon":"📅","message":"In a meeting"}]);
        assert_eq!(predefined(&rows).unwrap()[0].id, "meeting");
        assert!(predefined(&serde_json::json!(vec![serde_json::json!({}); 33])).is_err());
        assert!(predefined(&serde_json::json!([{"id":"","icon":"☕","message":"Break"}])).is_err());
        let status = serde_json::json!({"userId":"alice","status":"away","icon":"☕","message":"Break","clearAt":123});
        assert_eq!(details(&status, "alice").unwrap().clear_at, Some(123));
        assert_eq!(details(&status, "bob"), Err(ErrorKind::WrongScope));
    }

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
