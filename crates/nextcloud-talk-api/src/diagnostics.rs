//! Default-off bridge for the existing bounded synthetic internal-signaling consumer.
//! Tickets own reservations; no request, credential, arbitrary URL or cookie can escape.
//! OCS rows remain bounded neutral JSON here so the diagnostic consumer can preserve its
//! transactional stage observations. This surface is not a production event loop.
use crate::{Client, Error, ErrorKind, models::RoomToken, scope::*, transport};
pub struct Admission(transport::DiagnosticCallAdmission);
pub struct Lane(transport::DiagnosticLane);
pub struct Prepared(transport::DiagnosticPrepared);
pub struct Completion(transport::DiagnosticCompletion);
pub enum Operation<'a> {
    Read(&'a RoomToken),
    Signal(&'a RoomToken, &'a str),
    Leave(&'a RoomToken),
}
impl Prepared {
    pub async fn wait(self) -> Completion {
        Completion(self.0.wait().await)
    }
}
impl Completion {
    pub fn discard(self) -> Lane {
        Lane(self.0.discard())
    }
}
impl Client {
    pub fn diagnostic_admit(
        &self,
        context: &OperationContext,
        call: CallKey,
    ) -> Result<Admission, Error> {
        self.transport
            .diagnostic_call_admission(context, call)
            .map(Admission)
    }
    pub fn diagnostic_response_slot(&self) -> Result<tokio::sync::OwnedSemaphorePermit, Error> {
        self.transport.diagnostic_response_slot()
    }
    pub fn diagnostic_duplex(
        &mut self,
        context: &OperationContext,
        admission: &mut Admission,
    ) -> Result<(Lane, Lane), Error> {
        self.transport
            .diagnostic_duplex(context, &mut admission.0)
            .map(|(a, b)| (Lane(a), Lane(b)))
    }
    pub fn diagnostic_stop(&self) {
        self.transport.diagnostic_stop()
    }
    pub async fn diagnostic_poll(
        &mut self,
        context: &OperationContext,
        room: &RoomToken,
    ) -> Result<serde_json::Value, Error> {
        self.transport
            .signaling_poll(
                context,
                &format!("ocs/v2.php/apps/spreed/api/v3/signaling/{}", room.as_str()),
            )
            .await
            .map(|r| r.data)
    }
    pub async fn diagnostic_send(
        &mut self,
        context: &OperationContext,
        room: &RoomToken,
        messages: &str,
    ) -> Result<(), Error> {
        if messages.len() > 128 * 1024 {
            return Err(self.error(ErrorKind::TooLarge));
        }
        self.validate(context)?;
        self.transport
            .ocs(
                context,
                reqwest::Method::POST,
                &format!("ocs/v2.php/apps/spreed/api/v3/signaling/{}", room.as_str()),
                &[],
                &[("messages", messages.into())],
                false,
            )
            .await
            .map(|_| ())
            .map_err(|e| self.mutation_error(e))
    }
    pub fn diagnostic_prepare(
        &mut self,
        context: &OperationContext,
        call: CallKey,
        lane: Lane,
        operation: Operation<'_>,
    ) -> Result<Prepared, Error> {
        let (room, leave) = match operation {
            Operation::Read(r) | Operation::Signal(r, _) => (r, false),
            Operation::Leave(r) => (r, true),
        };
        let route = format!(
            "ocs/v2.php/apps/spreed/api/{}/{}",
            if leave { "v4/call" } else { "v3/signaling" },
            room.as_str()
        );
        let op = match operation {
            Operation::Read(_) => transport::DiagnosticOperation::Read { route: &route },
            Operation::Signal(_, messages) => transport::DiagnosticOperation::Signal {
                route: &route,
                messages,
            },
            Operation::Leave(_) => transport::DiagnosticOperation::Leave { route: &route },
        };
        self.transport
            .diagnostic_prepare(context, call, lane.0, op)
            .map(Prepared)
    }
    pub fn diagnostic_commit(
        &mut self,
        context: &OperationContext,
        call: CallKey,
        completion: Completion,
    ) -> (Lane, Result<serde_json::Value, Error>) {
        let (lane, r) = self
            .transport
            .diagnostic_commit(context, call, completion.0);
        (Lane(lane), r.map(|r| r.data))
    }
}
/// Used by the diagnostic consumer and owner-private fixture intake only.
pub fn bounded_json(bytes: &[u8]) -> Result<serde_json::Value, ErrorKind> {
    crate::json::json(bytes)
}
