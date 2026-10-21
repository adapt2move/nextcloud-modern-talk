//! Nextcloud HTTP/OCS deep module. Wire identifiers, authenticated transport and
//! correlation journals stay here; core receives bounded normalized observations.
mod call_policy;
mod content;
mod conversation;
#[cfg(any(test, feature = "backend-test"))]
pub mod diagnostics;
mod live;
mod meeting;
mod meeting_runtime;
mod messaging;
mod notifications;
#[cfg(any(test, target_os = "macos"))]
mod pdf_preview;
mod people;
mod preferences;
mod projection;
mod tags;
#[cfg(any(test, feature = "backend-test"))]
pub mod test_support;
#[cfg(test)]
mod tests;
mod transport;
mod wire;

pub use transport::NetworkRuntime;

use reqwest::Method;
use serde_json::Value;
use talk_core::bounded::{Page, Text};
use talk_core::ids::{CallScope, ConversationId, OperationId};
use talk_core::lifecycle::Cancellation;
use talk_core::policy::Decision;
use talk_core::ports::{
    Availability, ErrorKind, NextcloudPort, OperationContext, PortError, ProtocolReply,
    ProtocolRequest, Task,
};
use talk_core::protocol::{
    CallObservation, Conversation, ConversationList, HistoryPage, Message, MessageId, MessageKind,
    Participant, Participants, SendMessage, SendReceipt, SendState, SignalingBackend,
};
use transport::Transport;
#[allow(unused_imports, reason = "Reserved private identity seam")]
pub(crate) use transport::{AuthenticatedBinding, ServerOrigin};
use wire::Capabilities;
use zeroize::Zeroizing;

/// Identity-owned notification for an authoritative autonomous account probe.
/// The receiver checks the exact runtime before closing admission and suppression.
pub(crate) type AccountRejection =
    std::sync::Arc<dyn Fn(talk_core::ids::AccountScope) + Send + Sync>;

/// Adapter-private browser grant intake. No ordinary authenticated client escapes
/// while the canonical identity is unknown. This is not a primary-password API.
#[allow(
    dead_code,
    reason = "Private intake may be unwired in a transport-only composition"
)]
pub(crate) struct LoginBinding {
    pub(crate) scope: talk_core::ids::AccountScope,
    pub(crate) host: talk_core::ids::HostId,
    pub(crate) origin: ServerOrigin,
    pub(crate) login_name: Zeroizing<String>,
    pub(crate) password: Zeroizing<String>,
    pub(crate) expected_user: Option<Zeroizing<String>>,
    pub(crate) cancellation: Cancellation,
}

pub struct UnconfiguredNextcloud;
impl NextcloudPort for UnconfiguredNextcloud {
    fn availability(&self) -> Availability {
        Availability::Unconfigured
    }
    fn request(
        &mut self,
        _context: OperationContext,
        _request: ProtocolRequest,
    ) -> Task<'_, ProtocolReply> {
        Box::pin(async { Err(PortError::unconfigured()) })
    }
}

struct RoomBinding {
    id: ConversationId,
    token: String,
    server_id: Option<u64>,
}
struct PendingSend {
    operation: OperationId,
    conversation: ConversationId,
    reference: String,
    state: SendState,
}
struct ActiveConversation {
    id: ConversationId,
    session: zeroize::Zeroizing<String>,
}

/// Constructed by the identity adapter, never by a view. One serialized owner/account.
/// `request` borrows the owner; dropping it cancels observation/I/O, but a dispatched
/// mutation remains uncertain in its finite journal. This journal is memory-only.
pub struct NextcloudClient {
    account_rejection: Option<AccountRejection>,
    media_service: Option<std::sync::Arc<crate::media::MediaService>>,
    meeting_owner: Option<meeting::CallOwner>,
    incoming_invitation: Option<nextcloud_talk_api::meeting::CallInvitation>,
    dismissed_calls: Vec<u64>,
    message_notifications: notifications::Observation,
    call_bindings: Vec<(ConversationId, CallScope)>,
    transport: Transport,
    rooms: Vec<RoomBinding>,
    sends: Vec<PendingSend>,
    actions: Vec<conversation::PendingAction>,
    uploads: Vec<messaging::UploadName>,
    members: Vec<conversation::MemberBinding>,
    recipients: Vec<conversation::RecipientBinding>,
    people: Vec<people::PersonBinding>,
    direct_attempts: Vec<people::DirectAttempt>,
    integrations: Vec<conversation::IntegrationBinding>,
    active: Option<ActiveConversation>,
    observed_call: Option<CallScope>,
    creation_uncertain: bool,
    tags: Vec<tags::TagBinding>,
    tag_creation_uncertain: bool,
    #[cfg(any(test, feature = "backend-test"))]
    diagnostic_poll_stage: diagnostics::DiagnosticPollStage,
    #[cfg(any(test, feature = "backend-test"))]
    diagnostic_row_counts: Option<diagnostics::DiagnosticRowCounts>,
    #[cfg(any(test, feature = "backend-test"))]
    diagnostic_membership_scope: Option<(CallScope, u64)>,
    #[cfg(any(test, feature = "backend-test"))]
    diagnostic_signaling: diagnostics::SignalingState,
}
impl NextcloudClient {
    /// Consuming constructor: a reused client cannot enter this path. The private
    /// temporary client has no rooms, sessions or sends, and drops on every exit.
    #[allow(
        dead_code,
        reason = "Private intake may be unwired in a transport-only composition"
    )]
    pub(crate) async fn validate_login(
        binding: LoginBinding,
        runtime: NetworkRuntime,
        context: OperationContext,
    ) -> Result<(Zeroizing<String>, talk_core::protocol::AccountProbe), PortError> {
        let expected = binding.expected_user;
        let mut client = Self::with_transport(Transport::new_login(
            AuthenticatedBinding {
                scope: binding.scope,
                host: binding.host,
                origin: binding.origin,
                // Provisional only within this consuming chain; no ordinary request
                // or client is returned until authoritative canonical qualification.
                user: binding.login_name.clone(),
                login_name: binding.login_name,
                password: binding.password,
                cancellation: binding.cancellation,
            },
            runtime,
            expected.clone(),
        )?)?;
        let response = client
            .transport
            .ocs(
                &context,
                Method::GET,
                "ocs/v2.php/cloud/user",
                &[],
                &[],
                true,
            )
            .await?;
        let user = canonical_id(&response.data).map_err(|kind| client.error(kind))?;
        if expected
            .as_ref()
            .is_some_and(|value| value.as_str() != user)
        {
            return Err(client.error(ErrorKind::WrongScope));
        }
        let user = Zeroizing::new(user.to_owned());
        client.transport.bind_canonical_user(user.clone());
        let reply = client
            .execute(&context, ProtocolRequest::InspectAccount)
            .await?;
        client.transport.validate(&context)?;
        match reply {
            ProtocolReply::Account(probe) => Ok((user, probe)),
            _ => Err(client.error(ErrorKind::Malformed)),
        }
    }
    #[allow(
        dead_code,
        reason = "production authentication is owned by the later identity adapter"
    )]
    pub(crate) fn from_identity(
        binding: AuthenticatedBinding,
        runtime: NetworkRuntime,
    ) -> Result<Self, PortError> {
        Self::with_transport(Transport::new(binding, runtime)?)
    }
    fn with_transport(transport: Transport) -> Result<Self, PortError> {
        Ok(Self {
            account_rejection: None,
            transport,
            media_service: None,
            meeting_owner: None,
            incoming_invitation: None,
            dismissed_calls: Vec::new(),
            message_notifications: notifications::Observation::default(),
            call_bindings: Vec::new(),
            rooms: Vec::new(),
            sends: Vec::new(),
            actions: Vec::new(),
            uploads: Vec::new(),
            members: Vec::new(),
            recipients: Vec::new(),
            people: Vec::new(),
            direct_attempts: Vec::new(),
            integrations: Vec::new(),
            active: None,
            observed_call: None,
            creation_uncertain: false,
            tags: Vec::new(),
            tag_creation_uncertain: false,
            #[cfg(any(test, feature = "backend-test"))]
            diagnostic_poll_stage: diagnostics::DiagnosticPollStage::NotPolled,
            #[cfg(any(test, feature = "backend-test"))]
            diagnostic_row_counts: None,
            #[cfg(any(test, feature = "backend-test"))]
            diagnostic_membership_scope: None,
            #[cfg(any(test, feature = "backend-test"))]
            diagnostic_signaling: diagnostics::SignalingState::default(),
        })
    }
    pub(crate) fn set_account_rejection_handler(&mut self, handler: AccountRejection) {
        self.account_rejection = Some(handler);
    }
    /// Closing the identity/runtime cancellation prevents all later dispatch and delivery.
    pub fn cancellation(&self) -> Cancellation {
        self.transport.cancellation()
    }
    fn error(&self, kind: ErrorKind) -> PortError {
        self.transport.error(kind)
    }
    fn check_room(&self, id: ConversationId) -> Result<&str, PortError> {
        if id.account != self.transport.scope().account || id.host != self.transport.host() {
            return Err(self.error(ErrorKind::WrongScope));
        }
        self.rooms
            .iter()
            .find(|r| r.id == id)
            .map(|r| r.token.as_str())
            .ok_or_else(|| self.error(ErrorKind::NotFound))
    }
    /// Private persistence seam. The server's room ID never reaches presentation.
    pub(crate) fn room_identity(&self, id: ConversationId) -> Result<Option<u64>, PortError> {
        self.check_room(id)?;
        Ok(self
            .rooms
            .iter()
            .find(|room| room.id == id)
            .and_then(|room| room.server_id))
    }
    fn bind_room(&mut self, data: &Value) -> Result<ConversationId, PortError> {
        let token = data
            .get("token")
            .and_then(Value::as_str)
            .ok_or_else(|| self.error(ErrorKind::Malformed))?;
        if !(4..=30).contains(&token.len())
            || !token
                .bytes()
                .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit())
        {
            return Err(self.error(ErrorKind::Malformed));
        }
        let server_id = data
            .get("id")
            .and_then(Value::as_u64)
            .filter(|value| *value > 0);
        if let Some(room) = self.rooms.iter_mut().find(|r| r.token == token) {
            if room.server_id.is_some() && server_id.is_some() && room.server_id != server_id {
                return Err(self.error(ErrorKind::WrongScope));
            }
            room.server_id = server_id;
            return Ok(room.id);
        }
        if self.rooms.len() == 256 {
            return Err(self.error(ErrorKind::Busy));
        }
        let mut bytes = [0; 16];
        getrandom::fill(&mut bytes).map_err(|_| self.error(ErrorKind::Transport))?;
        let id = ConversationId {
            account: self.transport.scope().account,
            host: self.transport.host(),
            local: u128::from_ne_bytes(bytes),
        };
        if self.rooms.iter().any(|r| r.id == id) {
            return Err(self.error(ErrorKind::Conflict));
        }
        self.rooms.push(RoomBinding {
            id,
            token: token.to_owned(),
            server_id,
        });
        Ok(id)
    }
    fn route(&self, id: ConversationId, prefix: &str, suffix: &str) -> Result<String, PortError> {
        Ok(format!(
            "ocs/v2.php/apps/spreed/api/{prefix}/{}{suffix}",
            self.check_room(id)?
        ))
    }
    async fn capabilities(
        &mut self,
        context: &OperationContext,
    ) -> Result<Capabilities, PortError> {
        let response = self
            .transport
            .ocs(
                context,
                Method::GET,
                "ocs/v2.php/cloud/capabilities",
                &[],
                &[],
                false,
            )
            .await?;
        Capabilities::decode(&response.data).map_err(|k| self.error(k))
    }
    async fn probe(&mut self, context: &OperationContext) -> Result<Text<256>, PortError> {
        let response = match self
            .transport
            .ocs(
                context,
                Method::GET,
                "ocs/v2.php/cloud/user",
                &[],
                &[],
                true,
            )
            .await
        {
            Err(error) if error.kind == ErrorKind::ReauthenticationRequired => {
                self.transport.cancellation().cancel();
                return Err(error);
            }
            other => other?,
        };
        let user = canonical_id(&response.data).map_err(|kind| self.error(kind))?;
        if user != self.transport.user() {
            return Err(self.error(ErrorKind::WrongScope));
        }
        // Canonical/login IDs are private identity bindings, never fallback labels.
        let display = response
            .data
            .get("displayname")
            .or_else(|| response.data.get("display-name"));
        match display {
            Some(value) if !value.is_null() => wire::text(Some(value)).map_err(|k| self.error(k)),
            _ => Text::new("").map_err(|_| self.error(ErrorKind::Malformed)),
        }
    }
    async fn room(
        &mut self,
        context: &OperationContext,
        id: ConversationId,
        caps: &Capabilities,
    ) -> Result<Conversation, PortError> {
        let data = self.room_data(context, id).await?;
        let mut room = wire::room(&data, id, caps).map_err(|k| self.error(k))?;
        self.project_call(&mut room, &data, caps)?;
        self.project_room_tags(&mut room, &data, caps)?;
        Ok(room)
    }
    async fn room_data(
        &mut self,
        context: &OperationContext,
        id: ConversationId,
    ) -> Result<Value, PortError> {
        let route = self.route(id, "v4/room", "")?;
        let response = self
            .transport
            .ocs(context, Method::GET, &route, &[], &[], false)
            .await?;
        if response.data.get("token").and_then(Value::as_str) != Some(self.check_room(id)?) {
            return Err(self.error(ErrorKind::WrongScope));
        }
        if self.bind_room(&response.data)? != id {
            return Err(self.error(ErrorKind::WrongScope));
        }
        Ok(response.data)
    }
    fn allowed(&self, decision: Decision) -> Result<(), PortError> {
        match decision {
            Decision::Allowed => Ok(()),
            Decision::Denied => Err(self.error(ErrorKind::PermissionDenied)),
            _ => Err(self.error(ErrorKind::Unsupported)),
        }
    }
    fn supported(&self, caps: &Capabilities) -> Result<(), PortError> {
        if caps.supported() {
            Ok(())
        } else {
            Err(self.error(ErrorKind::Unsupported))
        }
    }
    fn mutation_error(&self, error: PortError) -> PortError {
        if matches!(
            error.kind,
            ErrorKind::PermissionDenied
                | ErrorKind::NotFound
                | ErrorKind::Conflict
                | ErrorKind::RateLimited
        ) {
            error
        } else {
            self.error(ErrorKind::Uncertain)
        }
    }
    async fn write(
        &mut self,
        context: &OperationContext,
        method: Method,
        route: &str,
        form: &[(&str, String)],
    ) -> Result<transport::Response, PortError> {
        self.transport.validate(context)?;
        self.transport
            .ocs(context, method, route, &[], form, false)
            .await
            .map_err(|e| self.mutation_error(e))
    }
    fn message(&self, data: &Value, conversation: ConversationId) -> Result<Message, PortError> {
        if data.get("token").and_then(Value::as_str) != Some(self.check_room(conversation)?) {
            return Err(self.error(ErrorKind::WrongScope));
        }
        let sequence = wire::integer(data.get("id"))
            .filter(|n| *n > 0)
            .ok_or_else(|| self.error(ErrorKind::Malformed))?;
        let kind = match data.get("messageType").and_then(Value::as_str) {
            Some("comment") => MessageKind::Comment,
            Some("comment_deleted") => MessageKind::Deleted,
            Some("system") => MessageKind::System,
            _ => MessageKind::Unsupported,
        };
        let own_operation = data
            .get("referenceId")
            .and_then(Value::as_str)
            .and_then(|reference| {
                self.sends
                    .iter()
                    .find(|s| {
                        s.conversation == conversation
                            && s.reference == reference
                            && data.get("actorType").and_then(Value::as_str) == Some("users")
                            && data.get("actorId").and_then(Value::as_str)
                                == Some(self.transport.user())
                    })
                    .map(|s| s.operation)
            });
        Ok(Message {
            id: MessageId {
                conversation,
                sequence,
            },
            author: wire::text(data.get("actorDisplayName")).map_err(|k| self.error(k))?,
            text: if kind == MessageKind::Deleted {
                Text::new("").expect("empty bounded text")
            } else {
                wire::text(data.get("message")).map_err(|k| self.error(k))?
            },
            kind,
            timestamp: wire::integer(data.get("timestamp"))
                .ok_or_else(|| self.error(ErrorKind::Malformed))?,
            expires_at: wire::integer(data.get("expirationTimestamp")).filter(|n| *n > 0),
            edited_at: wire::integer(data.get("lastEditTimestamp")).filter(|n| *n > 0),
            own_operation,
        })
    }
    async fn history(
        &mut self,
        context: &OperationContext,
        conversation: ConversationId,
        before: Option<MessageId>,
        caps: &Capabilities,
    ) -> Result<HistoryPage, PortError> {
        if before.is_some_and(|m| m.conversation != conversation || m.sequence == 0) {
            return Err(self.error(ErrorKind::WrongScope));
        }
        let room = self.room(context, conversation, caps).await?;
        self.allowed(room.actions.read_messages)?;
        let route = self.route(conversation, "v1/chat", "")?;
        let mut query = vec![
            ("lookIntoFuture", "0".to_owned()),
            ("limit", "50".to_owned()),
            ("setReadMarker", "0".to_owned()),
        ];
        if let Some(id) = before {
            query.push(("lastKnownMessageId", id.sequence.to_string()));
        }
        let response = self.transport.history(context, &route, &query).await?;
        let array = response
            .data
            .as_array()
            .ok_or_else(|| self.error(ErrorKind::Malformed))?;
        if array.len() > 50 {
            return Err(self.error(ErrorKind::TooLarge));
        }
        let mut messages = Page::empty();
        for data in array {
            let message = self.message(data, conversation)?;
            if messages
                .as_slice()
                .iter()
                .any(|m: &Message| m.id == message.id)
            {
                return Err(self.error(ErrorKind::Malformed));
            }
            messages
                .try_push(message)
                .map_err(|_| self.error(ErrorKind::TooLarge))?;
        }
        // Never advance a cursor from partial/malformed data. A cursor is scoped to its room.
        let older = if messages.as_slice().is_empty() {
            None
        } else {
            response
                .last_given
                .filter(|n| *n > 0 && before.is_none_or(|id| *n < id.sequence))
                .map(|sequence| MessageId {
                    conversation,
                    sequence,
                })
        };
        self.transport.validate(context)?;
        for message in messages.as_slice() {
            if let Some(operation) = message.own_operation
                && let Some(send) = self
                    .sends
                    .iter_mut()
                    .find(|s| s.operation == operation && s.conversation == conversation)
            {
                send.state = SendState::ServerAccepted(message.id);
            }
        }
        Ok(HistoryPage {
            conversation,
            messages,
            older,
            common_read: response
                .common_read
                .filter(|n| *n > 0 && caps.read_receipts == Some(true))
                .map(|sequence| MessageId {
                    conversation,
                    sequence,
                }),
        })
    }
    async fn send(
        &mut self,
        context: &OperationContext,
        request: SendMessage,
        caps: &Capabilities,
    ) -> Result<SendReceipt, PortError> {
        if let Some(existing) = self.sends.iter().find(|s| s.operation == request.operation) {
            if existing.conversation != request.conversation {
                return Err(self.error(ErrorKind::WrongScope));
            }
            return Ok(SendReceipt {
                operation: existing.operation,
                state: existing.state,
            });
        }
        if !caps.has("chat-reference-id") {
            return Err(self.error(ErrorKind::Unsupported));
        }
        let limit = caps
            .message_limit
            .ok_or_else(|| self.error(ErrorKind::Unsupported))?;
        if request.text.as_str().trim().is_empty()
            || request.text.as_str().chars().count() > limit as usize
        {
            return Err(self.error(ErrorKind::TooLarge));
        }
        let room = self.room(context, request.conversation, caps).await?;
        self.allowed(room.actions.send_message)?;
        if self.sends.len() == 100 {
            if let Some(index) = self
                .sends
                .iter()
                .position(|s| s.state != SendState::Uncertain)
            {
                self.sends.remove(index);
            } else {
                return Err(self.error(ErrorKind::Busy));
            }
        }
        let mut bytes = [0; 32];
        getrandom::fill(&mut bytes).map_err(|_| self.error(ErrorKind::Transport))?;
        let reference = bytes.iter().map(|b| format!("{b:02x}")).collect::<String>();
        if self.sends.iter().any(|s| s.reference == reference) {
            return Err(self.error(ErrorKind::Conflict));
        }
        let route = self.route(request.conversation, "v1/chat", "")?;
        self.transport.validate(context)?;
        // Before the first potentially side-effecting await: drop/cancel leaves uncertainty.
        self.sends.push(PendingSend {
            operation: request.operation,
            conversation: request.conversation,
            reference: reference.clone(),
            state: SendState::Uncertain,
        });
        let response = self
            .write(
                context,
                Method::POST,
                &route,
                &[
                    ("message", request.text.as_str().to_owned()),
                    ("referenceId", reference),
                ],
            )
            .await;
        match response {
            Ok(response) => {
                let message = self
                    .message(&response.data, request.conversation)
                    .map_err(|_| self.error(ErrorKind::Uncertain))?;
                if message.own_operation != Some(request.operation) {
                    return Err(self.error(ErrorKind::Uncertain));
                }
                let state = SendState::ServerAccepted(message.id);
                self.sends
                    .last_mut()
                    .expect("entry retained by serialized owner")
                    .state = state;
                Ok(SendReceipt {
                    operation: request.operation,
                    state,
                })
            }
            Err(error) => {
                if error.kind != ErrorKind::Uncertain {
                    self.sends.last_mut().expect("entry retained").state = SendState::Rejected;
                }
                Err(error)
            }
        }
    }
    async fn execute(
        &mut self,
        context: &OperationContext,
        request: ProtocolRequest,
    ) -> Result<ProtocolReply, PortError> {
        #[cfg(any(test, feature = "backend-test"))]
        if matches!(
            &request,
            ProtocolRequest::Leave(_)
                | ProtocolRequest::DeactivateConversation(_)
                | ProtocolRequest::LeaveConversation(_)
                | ProtocolRequest::ActivateConversation(_)
        ) {
            // Local eligibility closes before a leave/replacement can fail or be dropped.
            self.diagnostic_membership_scope = None;
            self.transport.diagnostic_stop();
            self.diagnostic_signaling.invalidate();
        }
        let validation = self.transport.validate(context);
        #[cfg(any(test, feature = "backend-test"))]
        if validation.is_err() {
            self.transport.diagnostic_stop();
            self.diagnostic_signaling.invalidate();
        }
        validation?;
        let room_id = match &request {
            ProtocolRequest::ReconcileConversation(id)
            | ProtocolRequest::Participants(id)
            | ProtocolRequest::LeaveConversation(id)
            | ProtocolRequest::ActivateConversation(id)
            | ProtocolRequest::DeactivateConversation(id) => Some(*id),
            ProtocolRequest::History { conversation, .. }
            | ProtocolRequest::Messaging { conversation, .. }
            | ProtocolRequest::ConversationSettings { conversation, .. } => Some(*conversation),
            ProtocolRequest::SetArchived { conversation, .. } => Some(*conversation),
            ProtocolRequest::Send(send) => Some(send.conversation),
            ProtocolRequest::MarkRead(id) => Some(id.conversation),
            ProtocolRequest::ObserveCall(call)
            | ProtocolRequest::Leave(call)
            | ProtocolRequest::RefreshCallPolicy(call) => {
                if call.account != context.scope || !call.is_consistent() {
                    return Err(self.error(ErrorKind::WrongScope));
                }
                Some(call.conversation)
            }
            _ => None,
        };
        if let Some(id) = room_id {
            self.check_room(id)?;
        }
        match request {
            ProtocolRequest::Meeting(command) => return self.meeting(context, command).await,
            ProtocolRequest::PollIncomingCalls => return self.poll_incoming_calls(context).await,
            ProtocolRequest::PollMessageNotifications => {
                return self.poll_message_notifications(context).await;
            }
            ProtocolRequest::PollLiveEvents => {
                return self
                    .transport
                    .poll_live_events(context)
                    .await
                    .map(ProtocolReply::LiveEvents);
            }
            ProtocolRequest::ProbeAccount => {
                self.probe(context).await?;
                return Ok(ProtocolReply::AccountValid(context.scope));
            }
            ProtocolRequest::SendStatus(operation) => {
                return self
                    .sends
                    .iter()
                    .find(|s| s.operation == operation)
                    .map(|s| {
                        ProtocolReply::Send(SendReceipt {
                            operation,
                            state: s.state,
                        })
                    })
                    .ok_or_else(|| self.error(ErrorKind::NotFound));
            }
            ProtocolRequest::RefreshCallPolicy(call) => {
                if !self.call_bindings.iter().any(|(_, scope)| *scope == call) {
                    return Err(self.error(ErrorKind::WrongScope));
                }
                return Ok(ProtocolReply::Policy(Box::new(
                    self.policy_sources(context, call).await?.inputs,
                )));
            }
            ProtocolRequest::Leave(call) => {
                if self.observed_call != Some(call)
                    || !call.is_consistent()
                    || call.account != context.scope
                    || self.active.as_ref().is_none_or(|active| {
                        active.id != call.conversation || active.session.is_empty()
                    })
                {
                    return Err(self.error(ErrorKind::WrongScope));
                }
                let route = self.route(call.conversation, "v4/call", "")?;
                self.write(context, Method::DELETE, &route, &[]).await?;
                self.observed_call = None;
                return Ok(ProtocolReply::LeaveAcknowledged(call));
            }
            ProtocolRequest::DeactivateConversation(id) => {
                if self.active.as_ref().is_none_or(|active| active.id != id) {
                    return Err(self.error(ErrorKind::WrongScope));
                }
                let route = self.route(id, "v4/room", "/participants/active")?;
                self.write(context, Method::DELETE, &route, &[]).await?;
                self.active = None;
                self.observed_call = None;
                return Ok(ProtocolReply::ConversationInactive(id));
            }
            _ => {}
        }
        if request == ProtocolRequest::InspectAccount {
            let display = self.probe(context).await?;
            let caps = self.capabilities(context).await?;
            return Ok(ProtocolReply::Account(caps.probe(context.scope, display)));
        }
        let caps = self.capabilities(context).await?;
        self.supported(&caps)?;
        match request {
            ProtocolRequest::Content { operation, command } => Ok(ProtocolReply::Content {
                operation,
                result: self.content(context, operation, command, &caps).await?,
            }),
            ProtocolRequest::Messaging {
                conversation,
                operation,
                command,
            } => Ok(ProtocolReply::Messaging {
                conversation,
                operation,
                result: self
                    .messaging(context, conversation, operation, command, &caps)
                    .await?,
            }),
            ProtocolRequest::ConversationSettings {
                conversation,
                operation,
                command,
            } => Ok(ProtocolReply::ConversationSettings {
                conversation,
                operation,
                result: self
                    .settings(context, conversation, operation, command, &caps)
                    .await?,
            }),
            ProtocolRequest::ListConversations => {
                let response = self
                    .transport
                    .ocs(
                        context,
                        Method::GET,
                        "ocs/v2.php/apps/spreed/api/v4/room",
                        &[("noStatusUpdate", "1".into())],
                        &[],
                        false,
                    )
                    .await?;
                let list = response
                    .data
                    .as_array()
                    .ok_or_else(|| self.error(ErrorKind::Malformed))?;
                if list.len() > 100 {
                    return Err(self.error(ErrorKind::TooLarge));
                }
                let mut rooms = Page::empty();
                for data in list {
                    let id = self.bind_room(data)?;
                    if rooms.as_slice().iter().any(|r: &Conversation| r.id == id) {
                        return Err(self.error(ErrorKind::Malformed));
                    }
                    let mut room = wire::room(data, id, &caps).map_err(|k| self.error(k))?;
                    self.project_call(&mut room, data, &caps)?;
                    self.project_room_tags(&mut room, data, &caps)?;
                    rooms
                        .try_push(room)
                        .map_err(|_| self.error(ErrorKind::TooLarge))?;
                }
                self.creation_uncertain = false;
                Ok(ProtocolReply::Conversations(ConversationList {
                    scope: context.scope,
                    rooms,
                }))
            }
            ProtocolRequest::ReconcileConversation(id) => Ok(ProtocolReply::Conversation(
                self.room(context, id, &caps).await?,
            )),
            ProtocolRequest::ReadPreferences { request } => Ok(ProtocolReply::Preferences(
                talk_core::preferences::account::AccountPreferenceResult {
                    request,
                    values: caps.account_preferences(context.scope),
                },
            )),
            ProtocolRequest::SetPreference { request, change } => {
                self.set_preference(context, request, change, &caps).await
            }
            ProtocolRequest::SetArchived {
                conversation,
                archived,
            } => {
                self.set_archived(context, conversation, archived, &caps)
                    .await
            }
            ProtocolRequest::SetFavorite {
                conversation,
                favorite,
            } => {
                self.set_favorite(context, conversation, favorite, &caps)
                    .await
            }
            request @ (ProtocolRequest::ReadTags
            | ProtocolRequest::CreateTag { .. }
            | ProtocolRequest::RenameTag { .. }
            | ProtocolRequest::DeleteTag(_)
            | ProtocolRequest::SetTagCollapsed { .. }
            | ProtocolRequest::ReorderTags { .. }
            | ProtocolRequest::SetConversationTags { .. }) => {
                self.tag_request(context, request, &caps).await
            }
            ProtocolRequest::Participants(id) => {
                let room = self.room(context, id, &caps).await?;
                self.allowed(room.actions.list_participants)?;
                let route = self.route(id, "v4/room", "/participants")?;
                let response = self
                    .transport
                    .ocs(context, Method::GET, &route, &[], &[], false)
                    .await?;
                let list = response
                    .data
                    .as_array()
                    .ok_or_else(|| self.error(ErrorKind::Malformed))?;
                if list.len() > 100 {
                    return Err(self.error(ErrorKind::TooLarge));
                }
                let mut people = Page::empty();
                for person in list {
                    people
                        .try_push(Participant {
                            display_name: wire::text(person.get("displayName"))
                                .map_err(|k| self.error(k))?,
                            role: wire::role(person.get("participantType")),
                            is_self: person.get("actorType").and_then(Value::as_str)
                                == Some("users")
                                && person.get("actorId").and_then(Value::as_str)
                                    == Some(self.transport.user()),
                            in_call: wire::integer(person.get("inCall")).map(|v| v & 1 != 0),
                        })
                        .map_err(|_| self.error(ErrorKind::TooLarge))?;
                }
                Ok(ProtocolReply::Participants(Participants {
                    conversation: id,
                    people,
                }))
            }
            ProtocolRequest::History {
                conversation,
                before,
            } => Ok(ProtocolReply::History(
                self.history(context, conversation, before, &caps).await?,
            )),
            ProtocolRequest::Send(request) => Ok(ProtocolReply::Send(
                self.send(context, request, &caps).await?,
            )),
            ProtocolRequest::MarkRead(id) => {
                if id.sequence == 0 || !caps.has("chat-read-marker") {
                    return Err(self.error(ErrorKind::Unsupported));
                }
                let room = self.room(context, id.conversation, &caps).await?;
                self.allowed(room.actions.read_messages)?;
                let route = self.route(id.conversation, "v1/chat", "/read")?;
                self.write(
                    context,
                    Method::POST,
                    &route,
                    &[("lastReadMessage", id.sequence.to_string())],
                )
                .await?;
                Ok(ProtocolReply::ReadAcknowledged(id))
            }
            ProtocolRequest::FindPeople { query } => self.find_people(context, query).await,
            ProtocolRequest::CreateDirect {
                operation,
                recipient,
            } => {
                self.create_direct(context, operation, recipient, &caps)
                    .await
            }
            ProtocolRequest::CreateGroup { name } => {
                if self.creation_uncertain {
                    return Err(self.error(ErrorKind::Uncertain));
                }
                self.allowed(caps.can_create)?;
                if name.as_str().trim().is_empty() {
                    return Err(self.error(ErrorKind::Malformed));
                }
                self.creation_uncertain = true;
                let response = self
                    .write(
                        context,
                        Method::POST,
                        "ocs/v2.php/apps/spreed/api/v4/room",
                        &[
                            ("roomType", "2".into()),
                            ("roomName", name.as_str().to_owned()),
                        ],
                    )
                    .await?;
                let id = self
                    .bind_room(&response.data)
                    .map_err(|_| self.error(ErrorKind::Uncertain))?;
                let mut room = wire::room(&response.data, id, &caps)
                    .map_err(|_| self.error(ErrorKind::Uncertain))?;
                self.project_room_tags(&mut room, &response.data, &caps)
                    .map_err(|_| self.error(ErrorKind::Uncertain))?;
                self.creation_uncertain = false;
                Ok(ProtocolReply::Conversation(room))
            }
            ProtocolRequest::LeaveConversation(id) => {
                let room = self.room(context, id, &caps).await?;
                self.allowed(room.actions.leave_conversation)?;
                let route = self.route(id, "v4/room", "/participants/self")?;
                self.write(context, Method::DELETE, &route, &[]).await?;
                self.rooms.retain(|r| r.id != id);
                if self.active.as_ref().is_some_and(|active| active.id == id) {
                    self.active = None;
                    self.observed_call = None;
                }
                Ok(ProtocolReply::ConversationLeft(id))
            }
            ProtocolRequest::ActivateConversation(id) => {
                if self.active.as_ref().is_some_and(|a| a.id != id) {
                    return Err(self.error(ErrorKind::Busy));
                }
                let room = self.room(context, id, &caps).await?;
                self.allowed(room.actions.read_messages)?;
                let route = self.route(id, "v4/room", "/participants/active")?;
                let response = self
                    .write(context, Method::POST, &route, &[("force", "false".into())])
                    .await?;
                let session = wire::text::<512>(response.data.get("sessionId"))
                    .map_err(|_| self.error(ErrorKind::Uncertain))?;
                if session.as_str().is_empty() || session.as_str() == "0" {
                    return Err(self.error(ErrorKind::Uncertain));
                }
                self.active = Some(ActiveConversation {
                    id,
                    session: zeroize::Zeroizing::new(session.as_str().to_owned()),
                });
                Ok(ProtocolReply::ConversationActive(id))
            }
            ProtocolRequest::ObserveCall(call) => {
                if !call.is_consistent()
                    || call.account != context.scope
                    || self.observed_call.is_some_and(|old| old != call)
                {
                    return Err(self.error(ErrorKind::WrongScope));
                }
                let room = self.room(context, call.conversation, &caps).await?;
                self.allowed(room.actions.list_participants)?;
                if room.channel
                    || room.classified
                    || room.unknown_restrictions
                    || !caps.has("signaling-v3")
                {
                    return Err(self.error(ErrorKind::Unsupported));
                }
                let token = self.check_room(call.conversation)?.to_owned();
                let route = self.route(call.conversation, "v4/call", "")?;
                let response = self
                    .transport
                    .ocs(context, Method::GET, &route, &[], &[], false)
                    .await?;
                let people = response
                    .data
                    .as_array()
                    .ok_or_else(|| self.error(ErrorKind::Malformed))?;
                if people.len() > 100 {
                    return Err(self.error(ErrorKind::TooLarge));
                }
                let connected_participants = people.len() as u16;
                let settings = self
                    .transport
                    .ocs(
                        context,
                        Method::GET,
                        "ocs/v2.php/apps/spreed/api/v3/signaling/settings",
                        &[("token", token)],
                        &[],
                        false,
                    )
                    .await?;
                let signaling = match settings.data.get("signalingMode").and_then(Value::as_str) {
                    Some("internal") => SignalingBackend::Internal,
                    Some("external") => SignalingBackend::External,
                    _ => SignalingBackend::Unknown,
                };
                self.observed_call = Some(call);
                Ok(ProtocolReply::Call(CallObservation {
                    call,
                    connected_participants,
                    signaling,
                    native_media: if caps.call_enabled == Decision::Allowed
                        && caps.recording_consent.is_some()
                        && caps.encryption_required.is_some()
                    {
                        Decision::Denied
                    } else {
                        Decision::Unknown
                    },
                }))
            }
            _ => Err(self.error(ErrorKind::Unsupported)),
        }
    }
}
impl NextcloudPort for NextcloudClient {
    fn availability(&self) -> Availability {
        if self.transport.cancellation().is_cancelled() {
            Availability::Unconfigured
        } else {
            Availability::Ready
        }
    }
    fn request(
        &mut self,
        context: OperationContext,
        request: ProtocolRequest,
    ) -> Task<'_, ProtocolReply> {
        Box::pin(async move {
            let mutation = match &request {
                ProtocolRequest::Content { command, .. } => content::is_mutation(command),
                ProtocolRequest::Messaging { command, .. } => messaging::is_mutation(command),
                ProtocolRequest::ConversationSettings { command, .. } => {
                    conversation::is_mutation(command)
                }
                request => matches!(
                    request,
                    ProtocolRequest::Send(_)
                        | ProtocolRequest::SetPreference { .. }
                        | ProtocolRequest::SetArchived { .. }
                        | ProtocolRequest::SetFavorite { .. }
                        | ProtocolRequest::CreateTag { .. }
                        | ProtocolRequest::RenameTag { .. }
                        | ProtocolRequest::DeleteTag(_)
                        | ProtocolRequest::SetTagCollapsed { .. }
                        | ProtocolRequest::ReorderTags { .. }
                        | ProtocolRequest::SetConversationTags { .. }
                        | ProtocolRequest::CreateGroup { .. }
                        | ProtocolRequest::CreateDirect { .. }
                        | ProtocolRequest::MarkRead(_)
                        | ProtocolRequest::LeaveConversation(_)
                        | ProtocolRequest::ActivateConversation(_)
                        | ProtocolRequest::DeactivateConversation(_)
                        | ProtocolRequest::Leave(_)
                ),
            };
            let tag_create = matches!(request, ProtocolRequest::CreateTag { .. });
            let result = self.execute(&context, request).await;
            if result
                .as_ref()
                .err()
                .is_some_and(|error| error.kind == ErrorKind::ReauthenticationRequired)
            {
                return result;
            }
            if let Err(error) = self.transport.validate(&context) {
                if tag_create {
                    self.tag_creation_uncertain = true;
                }
                return Err(
                    if mutation
                        && result
                            .as_ref()
                            .err()
                            .is_none_or(|e| e.kind == ErrorKind::Uncertain)
                    {
                        self.error(ErrorKind::Uncertain)
                    } else {
                        error
                    },
                );
            }
            result
        })
    }
}

// Both initial qualification and every authoritative account probe use this parser.
// Invalid successful data is not evidence that a different identity authenticated.
fn canonical_id(data: &Value) -> Result<&str, ErrorKind> {
    data.get("id")
        .and_then(Value::as_str)
        .filter(|s| !s.is_empty() && s.len() <= 256 && !s.chars().any(char::is_control))
        .ok_or(ErrorKind::Malformed)
}
