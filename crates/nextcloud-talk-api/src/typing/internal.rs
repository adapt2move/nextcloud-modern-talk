//! Internal signaling uses the same startedTyping/stoppedTyping payloads as HPB.
//! A separate bounded long-poll lane shares only the isolated chat cookie owner.
use super::*;
use serde_json::{Value, json};

#[derive(Default)]
struct State {
    members: Vec<PeerSession>,
    typing: Vec<(PeerSession, Instant)>,
}
pub(super) struct Link {
    state: Arc<Mutex<State>>,
    own: PeerSession,
    cancel: Cancellation,
    task: tokio::task::JoinHandle<()>,
}
impl Link {
    pub fn start(client: &mut Client, shared: &Arc<Shared>, own: PeerSession) -> Self {
        let reader = client.transport.typing_reader();
        let state = Arc::new(Mutex::new(State::default()));
        let cancel = Cancellation::default();
        let task = tokio::spawn(receive(
            reader,
            shared.clone(),
            state.clone(),
            own.clone(),
            cancel.clone(),
        ));
        Self {
            state,
            own,
            cancel,
            task,
        }
    }
    pub fn sessions(&self) -> Vec<PeerSession> {
        let Ok(mut state) = self.state.lock() else {
            return Vec::new();
        };
        let now = Instant::now();
        state
            .typing
            .retain(|(_, at)| now.saturating_duration_since(*at) < Duration::from_secs(8));
        state
            .typing
            .iter()
            .map(|(session, _)| session.clone())
            .collect()
    }
    pub async fn send(
        &self,
        client: &mut Client,
        context: &OperationContext,
        started: bool,
    ) -> Result<(), Error> {
        let sessions = self
            .state
            .lock()
            .map_err(|_| client.error(ErrorKind::Uncertain))?
            .members
            .clone();
        let mut messages = Vec::new();
        for peer in sessions.iter().filter(|peer| *peer != &self.own) {
            let payload = json!({"type":if started { "startedTyping" } else { "stoppedTyping" }, "to":peer.0.as_str()});
            messages.push(
                json!({"ev":"message", "sessionId":self.own.0.as_str(), "fn":payload.to_string()}),
            );
        }
        if messages.is_empty() {
            return Ok(());
        }
        let payload =
            serde_json::to_string(&messages).map_err(|_| client.error(ErrorKind::Malformed))?;
        if payload.len() > 128 * 1024 {
            return Err(client.error(ErrorKind::TooLarge));
        }
        let route = format!(
            "ocs/v2.php/apps/spreed/api/v3/signaling/{}",
            client
                .active_room
                .as_ref()
                .ok_or_else(|| client.error(ErrorKind::WrongScope))?
                .as_str()
        );
        client
            .transport
            .ocs(
                context,
                reqwest::Method::POST,
                &route,
                &[],
                &[("messages", payload)],
                false,
            )
            .await?;
        Ok(())
    }
}
impl Drop for Link {
    fn drop(&mut self) {
        self.cancel.cancel();
        self.task.abort();
    }
}
fn ingest(state: &mut State, value: &Value, own: &PeerSession) -> Result<(), ErrorKind> {
    let rows = value.as_array().ok_or(ErrorKind::Malformed)?;
    if rows.len() > 100 {
        return Err(ErrorKind::TooLarge);
    }
    for row in rows {
        match row["type"].as_str() {
            Some("usersInRoom") => {
                let users = row["data"].as_array().ok_or(ErrorKind::Malformed)?;
                if users.len() > 100 {
                    return Err(ErrorKind::TooLarge);
                }
                let mut members = Vec::new();
                for user in users {
                    let session = PeerSession::parse(
                        user["sessionId"].as_str().ok_or(ErrorKind::Malformed)?,
                    )?;
                    if !members.contains(&session) {
                        members.push(session);
                    }
                }
                state
                    .typing
                    .retain(|(session, _)| members.contains(session));
                state.members = members;
            }
            Some("message") => {
                let data = match &row["data"] {
                    Value::String(raw) if raw.len() <= 8192 => crate::json::json(raw.as_bytes())?,
                    Value::Object(_) => row["data"].clone(),
                    _ => continue,
                };
                let started = match data["type"].as_str() {
                    Some("startedTyping") => true,
                    Some("stoppedTyping") => false,
                    _ => continue,
                };
                if data["to"].as_str() != Some(own.0.as_str()) {
                    continue;
                }
                let Some(sender) = data["from"]
                    .as_str()
                    .and_then(|s| PeerSession::parse(s).ok())
                else {
                    continue;
                };
                if &sender == own || !state.members.contains(&sender) {
                    continue;
                }
                state.typing.retain(|(session, _)| session != &sender);
                if started && state.typing.len() < 100 {
                    state.typing.push((sender, Instant::now()));
                }
            }
            _ => {}
        }
    }
    Ok(())
}
async fn receive(
    mut reader: crate::transport::Transport,
    shared: Arc<Shared>,
    state: Arc<Mutex<State>>,
    own: PeerSession,
    cancel: Cancellation,
) {
    let route = format!(
        "ocs/v2.php/apps/spreed/api/v3/signaling/{}",
        shared.room.as_str()
    );
    let mut failures = 0u32;
    while !cancel.is_cancelled()
        && !shared.cancel.is_cancelled()
        && !shared.account_cancel.is_cancelled()
    {
        let now = reader.now();
        let context = OperationContext {
            scope: shared.scope,
            deadline: Moment {
                epoch: now.epoch,
                millis: now.millis.saturating_add(36_000),
            },
            cancellation: cancel.clone(),
        };
        match reader.signaling_poll(&context, &route).await {
            Ok(response) => {
                failures = 0;
                if let Ok(mut current) = state.lock() {
                    if ingest(&mut current, &response.data, &own).is_err() {
                        current.members.clear();
                        current.typing.clear();
                    }
                } else {
                    return;
                }
            }
            Err(_) => {
                if let Ok(mut current) = state.lock() {
                    current.members.clear();
                    current.typing.clear();
                }
                failures = failures.saturating_add(1).min(5);
                tokio::time::sleep(Duration::from_millis(250u64 << failures)).await;
            }
        }
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn room_membership_and_recipient_are_required_for_typing() {
        let own = PeerSession::parse("own").unwrap();
        let mut state = State::default();
        let events = json!([
            {"type":"usersInRoom", "data":[{"sessionId":"own"}, {"sessionId":"other"}]},
            {"type":"message", "data":json!({"type":"startedTyping", "from":"other", "to":"own"}).to_string()}
        ]);
        ingest(&mut state, &events, &own).unwrap();
        assert_eq!(state.typing.len(), 1);
        ingest(&mut state, &json!([{"type":"message","data":{"type":"stoppedTyping","from":"other","to":"different"}}]), &own).unwrap();
        assert_eq!(state.typing.len(), 1);
        ingest(
            &mut state,
            &json!([{"type":"usersInRoom","data":[{"sessionId":"own"}]}]),
            &own,
        )
        .unwrap();
        assert!(state.typing.is_empty());
    }
}
