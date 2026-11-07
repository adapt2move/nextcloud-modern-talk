//! Minimal bounded result from Nextcloud's authenticated TaskProcessing API.
//! Pinned to server v34.0.5/v35.0.2 TaskProcessingApiController and Task.php.
use crate::ErrorKind;
use serde_json::Value;

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum State {
    Pending,
    Failed,
    Finished(String),
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Task {
    pub id: u64,
    pub task_type: String,
    pub app: String,
    pub user: Option<String>,
    pub custom_id: Option<String>,
    pub state: State,
}

impl Task {
    pub(crate) fn decode(value: &Value) -> Result<Self, ErrorKind> {
        let task = value.get("task").ok_or(ErrorKind::Malformed)?;
        let id = task
            .get("id")
            .and_then(Value::as_u64)
            .filter(|id| *id > 0)
            .ok_or(ErrorKind::Malformed)?;
        let field = |key: &str, max: usize| -> Result<Option<String>, ErrorKind> {
            match task.get(key) {
                None | Some(Value::Null) => Ok(None),
                Some(Value::String(value)) if value.len() <= max => Ok(Some(value.clone())),
                Some(Value::String(_)) => Err(ErrorKind::TooLarge),
                _ => Err(ErrorKind::Malformed),
            }
        };
        let state = match task.get("status").and_then(Value::as_str) {
            Some("STATUS_SCHEDULED" | "STATUS_RUNNING" | "STATUS_UNKNOWN") => State::Pending,
            Some("STATUS_FAILED" | "STATUS_CANCELLED") => State::Failed,
            Some("STATUS_SUCCESSFUL") => {
                let output = task
                    .get("output")
                    .and_then(|output| output.get("output"))
                    .and_then(Value::as_str)
                    .ok_or(ErrorKind::Malformed)?;
                if output.len() > 4096 {
                    return Err(ErrorKind::TooLarge);
                }
                State::Finished(output.to_owned())
            }
            _ => return Err(ErrorKind::Malformed),
        };
        Ok(Self {
            id,
            task_type: field("type", 128)?.ok_or(ErrorKind::Malformed)?,
            app: field("appId", 128)?.ok_or(ErrorKind::Malformed)?,
            user: field("userId", 512)?,
            custom_id: field("customId", 512)?,
            state,
        })
    }
}
