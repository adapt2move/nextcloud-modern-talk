//! Normal, account-bound meeting ownership. The long poll owns its transport
//! lane independently of chat and policy reads. Dropping or cancelling either
//! lane closes this meeting and cannot silently recreate it.
use crate::{
    Client, Error, ErrorKind, Reply, Request,
    models::*,
    scope::*,
    signaling::*,
    transport::{MeetingLifetime, Transport},
};
use reqwest::Method;
use serde_json::json;
use std::sync::Arc;
use zeroize::Zeroizing;

#[derive(Clone)]
pub struct MeetingStop(Arc<MeetingLifetime>);
impl MeetingStop {
    pub fn close(&self) {
        self.0.close();
    }
    pub fn is_closed(&self) -> bool {
        self.0.cancellation.is_cancelled()
    }
}

struct Binding {
    room: RoomToken,
    room_id: u64,
    session: Zeroizing<String>,
    scope: SessionScope,
    lifetime: Arc<MeetingLifetime>,
}
impl Binding {
    fn context(&self, c: &OperationContext) -> Result<OperationContext, Error> {
        self.lifetime.validate(c)?;
        if c.scope != self.scope {
            return Err(self.error(ErrorKind::WrongScope));
        }
        Ok(OperationContext {
            scope: c.scope,
            deadline: c.deadline,
            cancellation: Cancellation::from_signal(Combined(
                c.cancellation.clone(),
                self.lifetime.cancellation.clone(),
            )),
        })
    }
    fn error(&self, kind: ErrorKind) -> Error {
        Error {
            kind,
            account: Some(self.scope.account),
            retry: crate::Retry::AfterReconciliation,
        }
    }
}
struct Combined(Cancellation, Cancellation);
impl CancellationSignal for Combined {
    fn cancel(&self) {
        self.0.cancel();
        self.1.cancel();
    }
    fn is_cancelled(&self) -> bool {
        self.0.is_cancelled() || self.1.is_cancelled()
    }
}

pub struct MeetingReader {
    transport: Transport,
    binding: Arc<Binding>,
}
pub struct MeetingWriter {
    transport: Transport,
    binding: Arc<Binding>,
    selected: Vec<PeerBudget>,
    leave_started: bool,
}
struct PeerBudget {
    peer: PeerSession,
    negotiation: Negotiation,
    descriptions: u8,
    candidates: u16,
}

impl Client {
    /// Consume the successful normal join handle. This grants only protocol
    /// ownership, never capture permission, policy freshness or E2EE.
    pub fn open_meeting(
        &mut self,
        context: &OperationContext,
        call: CallHandle,
    ) -> Result<(MeetingReader, MeetingWriter, MeetingStop), Error> {
        self.validate(context)?;
        if !Arc::ptr_eq(&self.owner, &call.owner)
            || call.scope != context.scope
            || call.origin != self.transport.host()
            || self.call_generation != Some(call.generation)
            || self.active_room.as_ref() != Some(&call.room)
        {
            return Err(self.error(ErrorKind::WrongScope));
        }
        let session = self
            .room_session
            .as_ref()
            .ok_or_else(|| self.error(ErrorKind::WrongScope))?
            .clone();
        let room_id = call
            .room_id
            .ok_or_else(|| self.error(ErrorKind::WrongScope))?;
        let (reader, writer, lifetime) = self.transport.open_meeting(context)?;
        let binding = Arc::new(Binding {
            room: call.room,
            room_id,
            session,
            scope: call.scope,
            lifetime: lifetime.clone(),
        });
        // The autonomous owner now handles signaling; the consumed handle cannot
        // also be used through the original serial send/poll API.
        self.call_generation = None;
        Ok((
            MeetingReader {
                transport: reader,
                binding: binding.clone(),
            },
            MeetingWriter {
                transport: writer,
                binding,
                selected: Vec::with_capacity(16),
                leave_started: false,
            },
            MeetingStop(lifetime),
        ))
    }
}

/// Cancels on abandonment, including while a destructive poll is in flight.
struct ReadGuard(Arc<MeetingLifetime>, bool);
impl Drop for ReadGuard {
    fn drop(&mut self) {
        if !self.1 {
            self.0.close();
        }
    }
}
impl MeetingReader {
    pub async fn poll(&mut self, context: &OperationContext) -> Result<SignalBatch, Error> {
        let context = self.binding.context(context)?;
        let mut guard = ReadGuard(self.binding.lifetime.clone(), false);
        let route = format!(
            "ocs/v2.php/apps/spreed/api/v3/signaling/{}",
            self.binding.room.as_str()
        );
        let response = self.transport.signaling_poll(&context, &route).await?;
        self.binding.lifetime.validate(&context)?;
        let batch = crate::signaling::parse_batch(
            &response.data,
            &self.binding.session,
            self.binding.room_id,
        )
        .map_err(|kind| self.binding.error(kind))?;
        guard.1 = true;
        Ok(batch)
    }
}
impl Drop for MeetingReader {
    fn drop(&mut self) {
        self.binding.lifetime.close();
    }
}

#[derive(Clone, Copy)]
pub enum Observation {
    Account,
    Capabilities,
    Room,
    ConnectedParticipants,
    CallParticipants,
    SignalingSettings,
}
pub struct ObservedReply {
    pub started: Moment,
    pub received: Moment,
    pub reply: Reply,
}

impl MeetingWriter {
    pub fn now(&self) -> Moment {
        self.transport.now()
    }
    pub fn own_session(&self) -> Result<PeerSession, ErrorKind> {
        PeerSession::parse(&self.binding.session)
    }
    pub async fn observe(
        &mut self,
        context: &OperationContext,
        observation: Observation,
    ) -> Result<ObservedReply, Error> {
        let context = self.binding.context(context)?;
        let request = match observation {
            Observation::Account => Request::Account,
            Observation::Capabilities => Request::Capabilities,
            Observation::Room => Request::Room(self.binding.room.clone()),
            Observation::ConnectedParticipants => Request::Participants(self.binding.room.clone()),
            Observation::CallParticipants => Request::CallParticipants(self.binding.room.clone()),
            Observation::SignalingSettings => Request::SignalingSettings(self.binding.room.clone()),
        };
        let (method, route, query, form, kind, _) =
            crate::plan(&request).map_err(|k| self.binding.error(k))?;
        let started = self.transport.now();
        let response = self
            .transport
            .ocs(
                &context,
                method,
                &route,
                &query,
                &form,
                matches!(observation, Observation::Account),
            )
            .await?;
        let reply = crate::decode(kind, response).map_err(|k| self.binding.error(k))?;
        match &reply {
            Reply::Account(account) if account.id != self.transport.user() => {
                return Err(self.binding.error(ErrorKind::WrongScope));
            }
            Reply::Room(room)
                if room.token != self.binding.room.as_str()
                    || room.server_id != Some(self.binding.room_id) =>
            {
                return Err(self.binding.error(ErrorKind::WrongScope));
            }
            _ => {}
        }
        self.binding.lifetime.validate(&context)?;
        Ok(ObservedReply {
            started,
            received: self.transport.now(),
            reply,
        })
    }
    /// Called after the owner has reconciled current membership. Each selected
    /// peer/SID has a separate finite exchange budget; a replaced SID retires the
    /// old entry instead of letting stale candidates attach to a newer peer.
    pub fn select(&mut self, peer: PeerSession, negotiation: Negotiation) -> Result<(), Error> {
        if self.binding.lifetime.cancellation.is_cancelled() {
            return Err(self.binding.error(ErrorKind::Cancelled));
        }
        if peer.0.as_str() == self.binding.session.as_str() {
            return Err(self.binding.error(ErrorKind::WrongScope));
        }
        if let Some(existing) = self.selected.iter().find(|p| p.peer == peer) {
            return if existing.negotiation == negotiation {
                Ok(())
            } else {
                Err(self.binding.error(ErrorKind::Conflict))
            };
        }
        if self.selected.len() == 16 {
            return Err(self.binding.error(ErrorKind::Busy));
        }
        self.selected.push(PeerBudget {
            peer,
            negotiation,
            descriptions: 0,
            candidates: 0,
        });
        Ok(())
    }
    pub fn remove(&mut self, peer: &PeerSession) {
        self.selected.retain(|p| &p.peer != peer);
    }
    pub async fn send(
        &mut self,
        context: &OperationContext,
        peer: &PeerSession,
        negotiation: &Negotiation,
        signal: Signal,
    ) -> Result<AcceptedForProcessing, Error> {
        let context = self.binding.context(context)?;
        let Some(budget) = self
            .selected
            .iter_mut()
            .find(|p| &p.peer == peer && &p.negotiation == negotiation)
        else {
            return Err(self.binding.error(ErrorKind::WrongScope));
        };
        let (kind, payload) = match signal {
            Signal::Description(sdp) => {
                if budget.descriptions >= 8 {
                    return Err(self.binding.error(ErrorKind::TooLarge));
                }
                budget.descriptions += 1;
                let kind = if sdp.kind() == DescriptionKind::Offer {
                    "offer"
                } else {
                    "answer"
                };
                (kind, json!({"type":kind,"sdp":sdp.as_str()}))
            }
            Signal::Candidate(candidate) => {
                if budget.candidates >= 128 {
                    return Err(self.binding.error(ErrorKind::TooLarge));
                }
                budget.candidates += 1;
                (
                    "candidate",
                    json!({"candidate":{"candidate":candidate.candidate(),"sdpMid":candidate.mid(),"sdpMLineIndex":candidate.index()}}),
                )
            }
        };
        let inner = json!({"to":peer.0.as_str(),"sid":negotiation.0,"roomType":"video","type":kind,"payload":payload});
        let messages = json!([{"ev":"message","sessionId":self.binding.session.as_str(),"fn":inner.to_string()}]).to_string();
        if messages.len() > 128 * 1024 {
            return Err(self.binding.error(ErrorKind::TooLarge));
        }
        let mut guard = ReadGuard(self.binding.lifetime.clone(), false);
        let route = format!(
            "ocs/v2.php/apps/spreed/api/v3/signaling/{}",
            self.binding.room.as_str()
        );
        self.transport
            .ocs(
                &context,
                Method::POST,
                &route,
                &[],
                &[("messages", messages)],
                false,
            )
            .await
            .map_err(|_| self.binding.error(ErrorKind::Uncertain))?;
        self.binding.lifetime.validate(&context)?;
        guard.1 = true;
        Ok(AcceptedForProcessing)
    }
    /// Local closure is first. This leaves only the call, preserving chat and
    /// conversation membership. An uncertain response is never replayed here.
    pub async fn leave(&mut self, context: &OperationContext) -> Result<(), Error> {
        self.binding.lifetime.close();
        if self.leave_started {
            return Err(self.binding.error(ErrorKind::Conflict));
        }
        self.leave_started = true;
        if context.scope != self.binding.scope {
            return Err(self.binding.error(ErrorKind::WrongScope));
        }
        let route = format!(
            "ocs/v2.php/apps/spreed/api/v4/call/{}",
            self.binding.room.as_str()
        );
        self.transport
            .ocs(context, Method::DELETE, &route, &[], &[], false)
            .await
            .map(|_| ())
            .map_err(|_| self.binding.error(ErrorKind::Uncertain))
    }
}
impl Drop for MeetingWriter {
    fn drop(&mut self) {
        self.binding.lifetime.close();
    }
}

impl Client {
    pub fn own_room_session(
        &self,
        context: &OperationContext,
        room: &RoomToken,
    ) -> Result<PeerSession, Error> {
        self.validate(context)?;
        if self.active_room.as_ref() != Some(room) {
            return Err(self.error(ErrorKind::WrongScope));
        }
        PeerSession::parse(
            self.room_session
                .as_ref()
                .ok_or_else(|| self.error(ErrorKind::WrongScope))?,
        )
        .map_err(|k| self.error(k))
    }
    /// Reconcile a locally closed call without replaying an uncertain leave.
    /// The remote own session must be absent from the authoritative call list.
    pub async fn finish_meeting(
        &mut self,
        context: &OperationContext,
        room: RoomToken,
    ) -> Result<(), Error> {
        self.validate(context)?;
        if self.transport.meeting_active() {
            return Err(self.error(ErrorKind::Busy));
        }
        let own = self.own_room_session(context, &room)?;
        let reply = self
            .request(context, Request::CallParticipants(room))
            .await?;
        let Reply::CallParticipants(peers) = reply else {
            return Err(self.error(ErrorKind::Malformed));
        };
        if peers
            .iter()
            .any(|p| PeerSession::parse(&p.session).is_ok_and(|s| s == own))
        {
            return Err(self.error(ErrorKind::Conflict));
        }
        self.call_generation = None;
        self.call_remote_pending = false;
        Ok(())
    }
}
impl MeetingWriter {
    pub async fn flags(
        &mut self,
        context: &OperationContext,
        flags: ParticipantFlags,
    ) -> Result<(), Error> {
        let context = self.binding.context(context)?;
        let mut guard = ReadGuard(self.binding.lifetime.clone(), false);
        let route = format!(
            "ocs/v2.php/apps/spreed/api/v4/call/{}",
            self.binding.room.as_str()
        );
        self.transport
            .ocs(
                &context,
                Method::PUT,
                &route,
                &[],
                &[("flags", flags.bits().to_string())],
                false,
            )
            .await
            .map_err(|_| self.binding.error(ErrorKind::Uncertain))?;
        guard.1 = true;
        Ok(())
    }
}

/// An actual notification fetched under this exact account owner. Neither a
/// room with an active call nor a renderer can construct this invitation.
pub struct CallInvitation {
    id: u64,
    room: RoomToken,
    eligible: bool,
    owner: Arc<()>,
}
impl CallInvitation {
    pub fn id(&self) -> u64 {
        self.id
    }
    pub fn room(&self) -> &RoomToken {
        &self.room
    }
    pub fn eligible(&self) -> bool {
        self.eligible
    }
}
impl Client {
    pub async fn call_invitations(
        &mut self,
        c: &OperationContext,
    ) -> Result<Vec<CallInvitation>, Error> {
        let Reply::Account(account) = self.request(c, Request::Account).await? else {
            return Err(self.error(ErrorKind::Malformed));
        };
        let Reply::Capabilities(caps) = self.request(c, Request::Capabilities).await? else {
            return Err(self.error(ErrorKind::Malformed));
        };
        if !caps.supported_protocol()
            || !caps.supports("call-notification-state-api")
            || !caps.notification_endpoints.iter().any(|s| s == "list")
        {
            return Err(self.error(ErrorKind::Unsupported));
        }
        let response = self
            .transport
            .ocs(
                c,
                Method::GET,
                "ocs/v2.php/apps/notifications/api/v2/notifications",
                &[],
                &[],
                false,
            )
            .await?;
        let rows = response
            .data
            .as_array()
            .ok_or_else(|| self.error(ErrorKind::Malformed))?;
        if rows.len() > 100 {
            return Err(self.error(ErrorKind::TooLarge));
        }
        let mut result = Vec::with_capacity(8);
        for row in rows {
            if row["app"].as_str() != Some("spreed") || row["object_type"].as_str() != Some("call")
            {
                continue;
            }
            if row["user"].as_str() != Some(account.id.as_str()) {
                return Err(self.error(ErrorKind::WrongScope));
            }
            if result.len() >= 8 {
                return Err(self.error(ErrorKind::TooLarge));
            }
            let id = row["notification_id"]
                .as_u64()
                .filter(|id| *id != 0)
                .ok_or_else(|| self.error(ErrorKind::Malformed))?;
            let room = RoomToken::parse(
                row["object_id"]
                    .as_str()
                    .ok_or_else(|| self.error(ErrorKind::Malformed))?,
            )
            .map_err(|k| self.error(k))?;
            if result.iter().any(|item: &CallInvitation| item.id == id) {
                return Err(self.error(ErrorKind::Malformed));
            }
            let eligible = row["shouldNotify"].as_bool() == Some(true)
                && response.notification_allowed == Some(true)
                && caps
                    .notification_endpoints
                    .iter()
                    .any(|s| s == "user-status");
            result.push(CallInvitation {
                id,
                room,
                eligible,
                owner: self.owner.clone(),
            });
        }
        Ok(result)
    }
    pub async fn invitation_current(
        &mut self,
        c: &OperationContext,
        invitation: &CallInvitation,
    ) -> Result<bool, Error> {
        self.validate(c)?;
        if !Arc::ptr_eq(&self.owner, &invitation.owner) {
            return Err(self.error(ErrorKind::WrongScope));
        }
        let route = format!(
            "ocs/v2.php/apps/spreed/api/v4/call/{}/notification-state",
            invitation.room.as_str()
        );
        match self
            .transport
            .ocs(c, Method::GET, &route, &[], &[], false)
            .await
        {
            Ok(response) => match response.status {
                200 => Ok(true),
                201 => Ok(false),
                _ => Err(self.error(ErrorKind::Malformed)),
            },
            Err(error) if error.kind == ErrorKind::NotFound => Ok(false),
            Err(error) => Err(error),
        }
    }
    /// One-shot dismissal of this fetched notification, never a room-wide end.
    pub async fn decline_invitation(
        &mut self,
        c: &OperationContext,
        invitation: CallInvitation,
    ) -> Result<(), Error> {
        self.validate(c)?;
        if !Arc::ptr_eq(&self.owner, &invitation.owner) {
            return Err(self.error(ErrorKind::WrongScope));
        }
        let route = format!(
            "ocs/v2.php/apps/notifications/api/v2/notifications/{}",
            invitation.id
        );
        self.transport
            .ocs(c, Method::DELETE, &route, &[], &[], false)
            .await
            .map(|_| ())
            .map_err(|_| self.error(ErrorKind::Uncertain))
    }
}
