//! Default-off synthetic internal-signaling diagnostic. Never production authority.
//! Only the explicit backend-test feature (or crate tests) exports this surface.
use super::transport::{
    DiagnosticCompletion, DiagnosticLane, DiagnosticOperation, DiagnosticPrepared,
};
use super::{NextcloudClient, wire};
use nextcloud_talk_api::signaling::{DescriptionKind, IceCandidate, SessionDescription};
use reqwest::Method;
use serde_json::{Value, json};
use std::fmt;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Weak};
use std::time::{Duration, Instant};
use talk_core::bounded::{Page, Text};
use talk_core::ids::CallScope;
use talk_core::lifecycle::Cancellation;
use talk_core::policy::Decision;
use talk_core::policy::Moment;
use talk_core::ports::{ErrorKind, OperationContext, PortError, ProtocolRequest};

const CANDIDATES: u8 = 64;
const DESCRIPTIONS: u8 = 2;
const ROWS: usize = 84;
const TYPED_BYTES: usize = 192 * 1024;
const SIGNALS: usize = 66;
const PAGE: usize = 32;

/// Opaque, shared identity only. Sharing it never copies signal payloads.
pub struct DiagnosticPeer(Arc<PeerIdentity>);
struct PeerIdentity {
    call: CallScope,
    session: zeroize::Zeroizing<String>,
    negotiation: Text<64>,
    should_offer: bool,
}
impl DiagnosticPeer {
    fn new(
        call: CallScope,
        session: &str,
        sid: &str,
        should_offer: bool,
    ) -> Result<Self, ErrorKind> {
        checked_session(session)?;
        checked_sid(sid)?;
        Ok(Self(Arc::new(PeerIdentity {
            call,
            session: zeroize::Zeroizing::new(session.to_owned()),
            negotiation: Text::new(sid).map_err(|_| ErrorKind::TooLarge)?,
            should_offer,
        })))
    }
    fn shared(&self) -> Self {
        Self(self.0.clone())
    }
    pub fn same_session(&self, other: &Self) -> bool {
        self.0.call == other.0.call && self.0.session == other.0.session
    }
    pub fn same_negotiation(&self, other: &Self) -> bool {
        self.same_session(other) && self.0.negotiation == other.0.negotiation
    }
    pub fn should_offer(&self) -> bool {
        self.0.should_offer
    }
}
impl fmt::Debug for DiagnosticPeer {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("DiagnosticPeer([redacted])")
    }
}

/// Outgoing adapter-private signal. The native sender owns its separate charge.
pub enum Signal {
    Offer(Text<65536>),
    Answer(Text<65536>),
    Candidate {
        candidate: Text<2048>,
        mid: Text<64>,
        index: u8,
    },
}
impl fmt::Debug for Signal {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("Signal([redacted])")
    }
}
/// Borrowed incoming payload. No owned text can be detached from a guarded page.
pub enum SignalRef<'a> {
    Offer(&'a str),
    Answer(&'a str),
    Candidate {
        candidate: &'a str,
        mid: &'a str,
        index: u8,
    },
}
enum RetainedSignal {
    Offer(Box<str>),
    Answer(Box<str>),
    Candidate {
        candidate: Box<str>,
        mid: Box<str>,
        index: u8,
    },
}
pub struct ReceivedSignal {
    peer: DiagnosticPeer,
    signal: RetainedSignal,
}
impl ReceivedSignal {
    pub fn peer(&self) -> &DiagnosticPeer {
        &self.peer
    }
    pub fn signal(&self) -> SignalRef<'_> {
        match &self.signal {
            RetainedSignal::Offer(s) => SignalRef::Offer(s),
            RetainedSignal::Answer(s) => SignalRef::Answer(s),
            RetainedSignal::Candidate {
                candidate,
                mid,
                index,
            } => SignalRef::Candidate {
                candidate,
                mid,
                index: *index,
            },
        }
    }
}
impl fmt::Debug for ReceivedSignal {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("ReceivedSignal([redacted])")
    }
}
#[derive(Clone, Copy, Default)]
struct Membership {
    peer_count: u16,
    observed: bool,
    selected_present: Option<bool>,
    selected_in_call: Option<bool>,
}
struct ResponseLease {
    _slot: tokio::sync::OwnedSemaphorePermit,
    valid: AtomicBool,
    ended: Arc<AtomicBool>,
    account_cancellation: Cancellation,
    operation_cancellation: Cancellation,
    expires: Instant,
    request_started: Moment,
}
impl ResponseLease {
    fn validate(&self) -> Result<(), ErrorKind> {
        if !self.valid.load(Ordering::Acquire)
            || self.ended.load(Ordering::Acquire)
            || self.account_cancellation.is_cancelled()
            || self.operation_cancellation.is_cancelled()
        {
            Err(ErrorKind::Cancelled)
        } else if Instant::now() >= self.expires {
            Err(ErrorKind::Deadline)
        } else {
            Ok(())
        }
    }
}
/// Private non-Clone ownership. `inspect` lends values; no into_iter/owned field
/// accessor exists. Even an empty final page retains its response reservation.
pub struct DiagnosticPoll {
    lease: Arc<ResponseLease>,
    signals: Box<[ReceivedSignal]>,
    membership: Membership,
    has_more: bool,
    typed_bytes: usize,
}
pub struct DiagnosticPollView<'a> {
    pub signals: &'a [ReceivedSignal],
    pub peer_count: u16,
    pub membership_observed: bool,
    pub selected_peer_present: Option<bool>,
    /// Explicit selected-session IN_CALL bit from the same validated snapshot.
    /// None means no selected session or unknown/missing metadata, never departure.
    pub selected_peer_in_call: Option<bool>,
    pub has_more: bool,
    /// Original destructive request start, never refreshed by pagination.
    pub request_started: Moment,
    /// Whole-response typed text charge, shared by all pages (not per-page).
    pub retained_typed_bytes: usize,
}
impl DiagnosticPoll {
    pub fn inspect(&self) -> Result<DiagnosticPollView<'_>, ErrorKind> {
        self.lease.validate()?;
        Ok(DiagnosticPollView {
            signals: &self.signals,
            peer_count: self.membership.peer_count,
            membership_observed: self.membership.observed,
            selected_peer_present: self.membership.selected_present,
            selected_peer_in_call: self.membership.selected_in_call,
            has_more: self.has_more,
            request_started: self.lease.request_started,
            retained_typed_bytes: self.typed_bytes,
        })
    }
}
struct Mailbox {
    lease: Arc<ResponseLease>,
    signals: Vec<ReceivedSignal>, // capacity exactly66; never grows.
    membership: Membership,
    typed_bytes: usize,
}
impl Mailbox {
    fn page(&mut self) -> Result<DiagnosticPoll, ErrorKind> {
        self.lease.validate()?;
        let size = self.signals.len().min(PAGE);
        let mut values = Vec::new();
        values
            .try_reserve_exact(size)
            .map_err(|_| ErrorKind::Busy)?;
        if values.capacity() != size {
            return Err(ErrorKind::TooLarge);
        }
        values.extend(self.signals.drain(..size));
        Ok(DiagnosticPoll {
            lease: self.lease.clone(),
            signals: values.into_boxed_slice(),
            membership: self.membership,
            has_more: !self.signals.is_empty(),
            typed_bytes: self.typed_bytes,
        })
    }
}
#[derive(Default)]
pub(super) struct SignalingState {
    // Deliberately no take/reset/getter; only a fresh Runtime admission constructs it.
    _admission: Option<super::transport::DiagnosticCallAdmission>,
    call: Option<CallScope>,
    terminated: bool,
    ended: Arc<AtomicBool>,
    duplex: bool,
    reader: Option<DiagnosticLane>,
    outbound: Option<DiagnosticLane>,
    selected: Option<DiagnosticPeer>,
    awaiting_offer: bool,
    selected_handle_issued: bool,
    candidates: u8,
    descriptions: u8,
    sent_candidates: u8,
    sent_descriptions: u8,
    polling: bool,
    mailbox: Option<Mailbox>,
    response: Weak<ResponseLease>,
}
impl SignalingState {
    fn admitted(admission: super::transport::DiagnosticCallAdmission, call: CallScope) -> Self {
        debug_assert_eq!(admission.scope(), call);
        let mut state = Self::default();
        state._admission = Some(admission);
        state.call = Some(call);
        state
    }
    pub(super) fn invalidate(&mut self) {
        self.terminated = true;
        self.ended.store(true, Ordering::Release);
        self.reader = None;
        if let Some(lease) = self.response.upgrade() {
            lease.valid.store(false, Ordering::Release);
        }
        self.mailbox = None;
        self.selected = None;
        // Counters/admission survive. Drop never unburns the Runtime tombstone.
    }
    fn validate(&self, call: CallScope) -> Result<(), ErrorKind> {
        if self.call != Some(call) {
            Err(ErrorKind::WrongScope)
        } else if self.terminated || self.ended.load(Ordering::Acquire) {
            Err(ErrorKind::Conflict)
        } else if self.polling && !self.duplex {
            Err(ErrorKind::Uncertain)
        } else {
            Ok(())
        }
    }
    fn select(&mut self, peer: &DiagnosticPeer) -> Result<(), ErrorKind> {
        self.validate(peer.0.call)?;
        if let Some(selected) = &self.selected {
            if !selected.same_session(peer)
                || (!self.awaiting_offer && !selected.same_negotiation(peer))
            {
                return Err(ErrorKind::WrongScope);
            }
        } else {
            self.selected = Some(peer.shared());
            self.awaiting_offer = !peer.should_offer();
        }
        Ok(())
    }
}
impl Drop for SignalingState {
    fn drop(&mut self) {
        self.invalidate();
    }
}
fn checked_session(value: &str) -> Result<(), ErrorKind> {
    if value.is_empty()
        || value == "0"
        || value.len() > 512
        || !value.bytes().all(|b| b.is_ascii_graphic())
    {
        Err(ErrorKind::Malformed)
    } else {
        Ok(())
    }
}
fn checked_sid(value: &str) -> Result<(), ErrorKind> {
    if value.is_empty() || value.len() > 64 || !value.bytes().all(|b| b.is_ascii_graphic()) {
        Err(ErrorKind::Malformed)
    } else {
        Ok(())
    }
}
fn borrowed_text(value: Option<&Value>, max: usize) -> Result<&str, ErrorKind> {
    let value = value.and_then(Value::as_str).ok_or(ErrorKind::Malformed)?;
    if value.len() > max {
        Err(ErrorKind::TooLarge)
    } else {
        Ok(value)
    }
}
fn charge(total: &mut usize, bytes: usize) -> Result<(), ErrorKind> {
    if bytes > TYPED_BYTES - *total {
        return Err(ErrorKind::TooLarge);
    }
    *total += bytes;
    Ok(())
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SignalSubmission {
    AcceptedForProcessing,
}

/// Default-off diagnostic position only. Never carries wire values or server text.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum DiagnosticPollStage {
    NotPolled,
    Scope,
    Transport,
    Rows,
    Membership,
    EnvelopeText,
    EnvelopeJson,
    RoomType,
    Sender,
    Recipient,
    Negotiation,
    Payload,
    Description,
    Candidate,
    Capacity,
    Complete,
}

/// Counts of outer row types only; no envelope, session, payload or server text.
/// `rows_at_least` saturates at 34. Type counts cover only the first 33 rows;
/// `types_truncated` means they are lower bounds, never whole-response totals.
/// None from the getter means the bounded transport did not deliver an array.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct DiagnosticRowCounts {
    pub rows_at_least: u8,
    pub rows_exact: bool,
    pub sampled_rows: u8,
    pub users_in_room: u8,
    pub messages: u8,
    pub other_type: u8,
    pub missing_type: u8,
    pub types_truncated: bool,
}

impl DiagnosticRowCounts {
    fn observe(rows: &[Value]) -> Self {
        let mut counts = Self {
            rows_at_least: rows.len().min(34) as u8,
            rows_exact: rows.len() <= 34,
            types_truncated: rows.len() > 33,
            ..Self::default()
        };
        for row in rows.iter().take(33) {
            counts.sampled_rows += 1;
            match row.get("type").and_then(Value::as_str) {
                Some("usersInRoom") => counts.users_in_room += 1,
                Some("message") => counts.messages += 1,
                Some(_) => counts.other_type += 1,
                None => counts.missing_type += 1,
            }
        }
        counts
    }
}

impl NextcloudClient {
    /// After an error, reports only the allowlisted stage at which polling stopped.
    pub fn diagnostic_poll_stage(&self) -> DiagnosticPollStage {
        self.diagnostic_poll_stage
    }
    /// Latest poll only; reset before scope admission. Observations grant no authority.
    pub fn diagnostic_row_counts(&self) -> Option<DiagnosticRowCounts> {
        self.diagnostic_row_counts
    }
    /// Synthetic AV only. No capture is opened by this method. The separate harness
    /// must use synthetic tracks and cannot turn this into a production media grant.
    pub async fn diagnostic_join_synthetic_call(
        &mut self,
        context: &OperationContext,
        call: CallScope,
    ) -> Result<(), PortError> {
        self.diagnostic_membership_scope = None;
        self.diagnostic_signaling.invalidate();
        if call.account != context.scope
            || !call.is_consistent()
            || self.observed_call.is_some_and(|old| old != call)
        {
            return Err(self.error(ErrorKind::WrongScope));
        }
        self.transport.validate(context)?;
        self.check_room(call.conversation)?;
        let admission = self.transport.diagnostic_call_admission(context, call)?;
        self.execute(
            context,
            ProtocolRequest::ActivateConversation(call.conversation),
        )
        .await?;
        let caps = self.capabilities(context).await?;
        let data = self.room_data(context, call.conversation).await?;
        let room = wire::room(&data, call.conversation, &caps).map_err(|k| self.error(k))?;
        let number = data
            .get("id")
            .and_then(Value::as_u64)
            .filter(|id| *id > 0)
            .ok_or_else(|| self.error(ErrorKind::Malformed))?;
        if room.channel
            || room.classified
            || room.unknown_restrictions
            || caps.call_enabled != Decision::Allowed
            || caps.recording_consent != Some(0)
            || caps.encryption_required != Some(false)
            || !caps.has("signaling-v3")
        {
            return Err(self.error(ErrorKind::Unsupported));
        }
        let token = self.check_room(call.conversation)?.to_owned();
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
        if settings.data.get("signalingMode").and_then(Value::as_str) != Some("internal") {
            return Err(self.error(ErrorKind::Unsupported));
        }
        let route = self.route(call.conversation, "v4/call", "")?;
        // flags=IN_CALL|WITH_AUDIO|WITH_VIDEO, only under the diagnostic feature.
        self.write(
            context,
            Method::POST,
            &route,
            &[("flags", "7".into()), ("silent", "true".into())],
        )
        .await?;
        self.observed_call = Some(call);
        self.diagnostic_scope(context, call)?;
        self.diagnostic_membership_scope = Some((call, number));
        self.diagnostic_signaling = SignalingState::admitted(admission, call);
        Ok(())
    }
    /// Up to 16 current peers; IDs stay opaque and bound to this account/host/call.
    pub async fn diagnostic_peers(
        &mut self,
        context: &OperationContext,
        call: CallScope,
    ) -> Result<Page<DiagnosticPeer, 16>, PortError> {
        self.diagnostic_scope(context, call)?;
        let route = self.route(call.conversation, "v4/call", "")?;
        let response = self
            .transport
            .ocs(context, Method::GET, &route, &[], &[], false)
            .await?;
        let entries = response
            .data
            .as_array()
            .ok_or_else(|| self.error(ErrorKind::Malformed))?;
        if entries.len() > 17 {
            return Err(self.error(ErrorKind::TooLarge));
        }
        // Call GET omits flags in released Talk. Participant v4 supplies aggregate
        // flags plus sessionIds. Only one connected session for an actor is
        // unambiguous; inactive sessions from another browser do not count.
        let participants_route = self.route(call.conversation, "v4/room", "/participants")?;
        let participants = self
            .transport
            .ocs(context, Method::GET, &participants_route, &[], &[], false)
            .await?;
        let participants = participants
            .data
            .as_array()
            .ok_or_else(|| self.error(ErrorKind::Malformed))?;
        if participants.len() > 100 {
            return Err(self.error(ErrorKind::TooLarge));
        }
        let mut peers = Page::empty();
        for entry in entries {
            let session = wire::text::<512>(entry.get("sessionId")).map_err(|k| self.error(k))?;
            if session.as_str().is_empty()
                || !session.as_str().bytes().all(|b| b.is_ascii_graphic())
            {
                return Err(self.error(ErrorKind::Malformed));
            }
            if self
                .active
                .as_ref()
                .is_some_and(|active| active.session.as_str() == session.as_str())
            {
                continue;
            }
            let person = participants
                .iter()
                .find(|p| {
                    p.get("sessionIds")
                        .and_then(Value::as_array)
                        .is_some_and(|ids| {
                            ids.len() <= 17
                                && ids.iter().any(|id| id.as_str() == Some(session.as_str()))
                        })
                })
                .ok_or_else(|| self.error(ErrorKind::Unsupported))?;
            if entries
                .iter()
                .filter(|p| {
                    p.get("actorType") == entry.get("actorType")
                        && p.get("actorId") == entry.get("actorId")
                })
                .count()
                != 1
            {
                return Err(self.error(ErrorKind::Unsupported));
            }
            let flags = wire::integer(person.get("inCall"))
                .filter(|n| n & !7 == 0 && n & 1 != 0)
                .ok_or_else(|| self.error(ErrorKind::Unsupported))?;
            let should_offer = flags & 6 == 0
                || self
                    .active
                    .as_ref()
                    .is_some_and(|a| session.as_str() < a.session.as_str());
            let mut random = [0; 16];
            getrandom::fill(&mut random).map_err(|_| self.error(ErrorKind::Transport))?;
            let sid = random
                .iter()
                .map(|n| format!("{n:02x}"))
                .collect::<String>();
            peers
                .try_push(
                    DiagnosticPeer::new(call, session.as_str(), &sid, should_offer)
                        .map_err(|kind| self.error(kind))?,
                )
                .map_err(|_| self.error(ErrorKind::TooLarge))?;
        }
        self.diagnostic_scope(context, call)?;
        Ok(peers)
    }
    fn diagnostic_scope(
        &mut self,
        context: &OperationContext,
        call: CallScope,
    ) -> Result<(), PortError> {
        if let Err(error) = self.transport.validate(context) {
            self.diagnostic_membership_scope = None;
            self.diagnostic_stop_signaling();
            return Err(error);
        }
        if self.observed_call != Some(call)
            || call.account != context.scope
            || !call.is_consistent()
            || self
                .active
                .as_ref()
                .is_none_or(|a| a.id != call.conversation)
        {
            self.diagnostic_membership_scope = None;
            self.diagnostic_stop_signaling();
            return Err(self.error(ErrorKind::WrongScope));
        }
        Ok(())
    }
    pub async fn diagnostic_send(
        &mut self,
        context: &OperationContext,
        peer: &DiagnosticPeer,
        signal: Signal,
    ) -> Result<SignalSubmission, PortError> {
        self.diagnostic_scope(context, peer.0.call)?;
        if self.diagnostic_signaling.duplex {
            return Err(self.error(ErrorKind::Busy));
        }
        let messages = self.diagnostic_messages(context, peer, signal)?;
        let route = self.route(peer.0.call.conversation, "v3/signaling", "")?;
        if let Err(error) = self
            .write(context, Method::POST, &route, &[("messages", messages)])
            .await
        {
            self.diagnostic_stop_signaling();
            return Err(error);
        }
        // Upstream silently drops some signals. This acknowledges processing only.
        Ok(SignalSubmission::AcceptedForProcessing)
    }
    fn diagnostic_messages(
        &mut self,
        context: &OperationContext,
        peer: &DiagnosticPeer,
        signal: Signal,
    ) -> Result<String, PortError> {
        let result = (|| {
            self.diagnostic_scope(context, peer.0.call)?;
            if let Err(kind) = self.diagnostic_signaling.select(peer) {
                self.diagnostic_signaling.invalidate();
                return Err(self.error(kind));
            }
            let state = &mut self.diagnostic_signaling;
            if state.awaiting_offer {
                return Err(self.error(ErrorKind::Conflict));
            }
            let counter = if matches!(&signal, Signal::Candidate { .. }) {
                &mut state.sent_candidates
            } else {
                &mut state.sent_descriptions
            };
            let limit = if matches!(&signal, Signal::Candidate { .. }) {
                CANDIDATES
            } else {
                DESCRIPTIONS
            };
            if *counter >= limit {
                state.invalidate();
                return Err(self.error(ErrorKind::TooLarge));
            }
            *counter += 1;
            let (kind, payload) = match signal {
                Signal::Offer(sdp) => {
                    SessionDescription::new(DescriptionKind::Offer, sdp.as_str())
                        .map_err(|_| self.error(ErrorKind::Malformed))?;
                    ("offer", json!({"type":"offer","sdp":sdp.as_str()}))
                }
                Signal::Answer(sdp) => {
                    SessionDescription::new(DescriptionKind::Answer, sdp.as_str())
                        .map_err(|_| self.error(ErrorKind::Malformed))?;
                    ("answer", json!({"type":"answer","sdp":sdp.as_str()}))
                }
                Signal::Candidate {
                    candidate,
                    mid,
                    index,
                } => {
                    IceCandidate::new(mid.as_str(), index, candidate.as_str())
                        .map_err(|_| self.error(ErrorKind::Malformed))?;
                    (
                        "candidate",
                        json!({"candidate":{"candidate":candidate.as_str(),"sdpMid":mid.as_str(),"sdpMLineIndex":index}}),
                    )
                }
            };
            let session = self
                .active
                .as_ref()
                .ok_or_else(|| self.error(ErrorKind::WrongScope))?
                .session
                .as_str();
            let inner = json!({"to":peer.0.session.as_str(),"sid":peer.0.negotiation.as_str(),"roomType":"video","type":kind,"payload":payload});
            let messages =
                json!([{"ev":"message","sessionId":session,"fn":inner.to_string()}]).to_string();
            if messages.len() > 128 * 1024 {
                return Err(self.error(ErrorKind::TooLarge));
            }
            Ok(messages)
        })();
        if result.is_err() {
            self.diagnostic_stop_signaling();
        }
        result
    }
    /// Select exactly one observed connected session. Local offerers bind the
    /// generated SID now; answerers admit only that session's first offer SID.
    pub fn diagnostic_select_peer(
        &mut self,
        context: &OperationContext,
        peer: &DiagnosticPeer,
    ) -> Result<(), PortError> {
        self.diagnostic_scope(context, peer.0.call)?;
        if let Err(kind) = self.diagnostic_signaling.select(peer) {
            self.diagnostic_signaling.invalidate();
            return Err(self.error(kind));
        }
        Ok(())
    }
    /// A single separately bounded opaque handle, sharing identity storage only.
    /// It can outlive a page; no signal payload or response reservation detaches.
    pub fn diagnostic_take_selected_peer(
        &mut self,
        context: &OperationContext,
        call: CallScope,
    ) -> Result<DiagnosticPeer, PortError> {
        self.diagnostic_scope(context, call)?;
        let state = &mut self.diagnostic_signaling;
        let result = state.validate(call).and_then(|()| {
            if state.awaiting_offer || state.selected_handle_issued {
                return Err(ErrorKind::Conflict);
            }
            let peer = state
                .selected
                .as_ref()
                .ok_or(ErrorKind::WrongScope)?
                .shared();
            state.selected_handle_issued = true;
            Ok(peer)
        });
        result.map_err(|kind| self.error(kind))
    }
    /// Synchronous local retirement; remote leave is a separate owned operation.
    pub fn diagnostic_stop_signaling(&mut self) {
        self.transport.diagnostic_stop();
        self.diagnostic_signaling.invalidate();
    }

    /// Without selection, only scoped membership may be observed. Signals never
    /// select a peer implicitly or create a fresh budget after destructive loss.
    pub async fn diagnostic_poll(
        &mut self,
        context: &OperationContext,
        call: CallScope,
    ) -> Result<DiagnosticPoll, PortError> {
        self.diagnostic_poll_selected(context, call, None).await
    }
    /// A destructive GET or a drain of its fully validated response. A retained
    /// final page still returns Busy on the next call, before any HTTP operation.
    pub async fn diagnostic_poll_selected(
        &mut self,
        context: &OperationContext,
        call: CallScope,
        selected: Option<&DiagnosticPeer>,
    ) -> Result<DiagnosticPoll, PortError> {
        self.diagnostic_row_counts = None;
        self.diagnostic_poll_stage = DiagnosticPollStage::Scope;
        self.diagnostic_scope(context, call)?;
        if let Some(peer) = selected {
            self.diagnostic_select_peer(context, peer)?;
        }
        if let Err(kind) = self.diagnostic_signaling.validate(call) {
            self.diagnostic_signaling.invalidate();
            return Err(self.error(kind));
        }
        let number = self
            .diagnostic_membership_scope
            .filter(|(scope, _)| *scope == call)
            .map(|(_, number)| number)
            .ok_or_else(|| self.error(ErrorKind::WrongScope))?;
        if self.diagnostic_signaling.mailbox.is_some() {
            return self.diagnostic_drain(context, call);
        }
        if self.diagnostic_signaling.duplex
            || self.diagnostic_signaling.response.upgrade().is_some()
        {
            return Err(self.error(ErrorKind::Busy)); // Non-admission, no budget reset.
        }
        let route = self.route(call.conversation, "v3/signaling", "")?;
        let slot = self.transport.diagnostic_response_slot()?;
        let started = self.transport.now();
        let available = context
            .deadline
            .millis
            .saturating_sub(started.millis)
            .min(35_000);
        let lease = Arc::new(ResponseLease {
            _slot: slot,
            valid: AtomicBool::new(true),
            ended: self.diagnostic_signaling.ended.clone(),
            account_cancellation: self.transport.cancellation(),
            operation_cancellation: context.cancellation.clone(),
            expires: Instant::now() + Duration::from_millis(available),
            request_started: started,
        });
        self.diagnostic_signaling.response = Arc::downgrade(&lease);
        // If this future is dropped while GET is in flight, this flag survives;
        // a later call terminates as Uncertain. Destructive polls are not replayed.
        self.diagnostic_signaling.polling = true;
        self.diagnostic_poll_stage = DiagnosticPollStage::Transport;
        let response = match self.transport.diagnostic_poll(context, &route).await {
            Ok(response) => response,
            Err(error) => {
                self.diagnostic_signaling.invalidate();
                return Err(error);
            }
        };
        let local = self
            .active
            .as_ref()
            .expect("scope checked")
            .session
            .as_str();
        let parsed = parse_response(
            &response.data,
            local,
            number,
            &self.diagnostic_signaling,
            lease,
            &mut self.diagnostic_poll_stage,
            &mut self.diagnostic_row_counts,
        );
        let (mailbox, selected, candidates, descriptions) = match parsed {
            Ok(parsed) => parsed,
            Err(kind) => {
                self.diagnostic_signaling.invalidate();
                return Err(self.error(kind));
            }
        };
        self.diagnostic_scope(context, call)?;
        self.diagnostic_signaling.selected = selected;
        self.diagnostic_signaling.awaiting_offer =
            self.diagnostic_signaling.awaiting_offer && descriptions == 0;
        self.diagnostic_signaling.candidates = candidates;
        self.diagnostic_signaling.descriptions = descriptions;
        self.diagnostic_signaling.polling = false;
        self.diagnostic_signaling.mailbox = Some(mailbox);
        self.diagnostic_drain(context, call)
    }
    fn diagnostic_drain(
        &mut self,
        context: &OperationContext,
        call: CallScope,
    ) -> Result<DiagnosticPoll, PortError> {
        self.diagnostic_poll_stage = DiagnosticPollStage::Scope;
        self.diagnostic_scope(context, call)?;
        if let Err(kind) = self.diagnostic_signaling.validate(call) {
            self.diagnostic_signaling.invalidate();
            return Err(self.error(kind));
        }
        let result = self
            .diagnostic_signaling
            .mailbox
            .as_mut()
            .ok_or(ErrorKind::WrongScope)
            .and_then(Mailbox::page);
        match result {
            Ok(page) => {
                if !page.has_more {
                    self.diagnostic_signaling.mailbox = None;
                }
                self.diagnostic_poll_stage = DiagnosticPollStage::Complete;
                Ok(page)
            }
            Err(kind) => {
                self.diagnostic_signaling.invalidate();
                Err(self.error(kind))
            }
        }
    }
}

// The following default-off tickets hold no public wire/credential access. One
// reader and one outbound owner move through prepare -> wait -> owner commit.
struct TicketGuard {
    context: OperationContext,
    call: CallScope,
    selected: Option<DiagnosticPeer>,
    awaiting_offer: bool,
    room: u64,
    local: zeroize::Zeroizing<String>,
    ended: Arc<AtomicBool>,
    armed: bool,
    leave: bool,
}
impl Drop for TicketGuard {
    fn drop(&mut self) {
        if self.armed {
            self.ended.store(true, Ordering::Release);
        }
    }
}
impl TicketGuard {
    fn validate(
        &self,
        owner: &mut NextcloudClient,
        context: &OperationContext,
        call: CallScope,
    ) -> Result<(), PortError> {
        owner.diagnostic_scope(context, call)?;
        owner.transport.validate(&self.context)?;
        let state = &owner.diagnostic_signaling;
        if self.call != call
            || !Arc::ptr_eq(&self.ended, &state.ended)
            || owner.diagnostic_membership_scope != Some((call, self.room))
            || owner
                .active
                .as_ref()
                .is_none_or(|active| active.session.as_str() != self.local.as_str())
        {
            return Err(owner.error(ErrorKind::WrongScope));
        }
        if !self.leave {
            state.validate(call).map_err(|kind| owner.error(kind))?;
            if state.awaiting_offer != self.awaiting_offer
                || match (&state.selected, &self.selected) {
                    (Some(current), Some(captured)) => !current.same_negotiation(captured),
                    _ => true,
                }
            {
                return Err(owner.error(ErrorKind::WrongScope));
            }
        }
        Ok(())
    }
}
/// Single-use, non-Clone destructive-read ownership. Dropping its future or
/// completion ends this negotiation; it never becomes a retry capability.
pub struct DiagnosticRead {
    prepared: DiagnosticPrepared,
    guard: TicketGuard,
    lease: Arc<ResponseLease>,
}
pub struct DiagnosticReadCompletion {
    completion: DiagnosticCompletion,
    guard: TicketGuard,
    lease: Arc<ResponseLease>,
}
impl DiagnosticRead {
    pub async fn wait(self) -> DiagnosticReadCompletion {
        DiagnosticReadCompletion {
            completion: self.prepared.wait().await,
            guard: self.guard,
            lease: self.lease,
        }
    }
}
/// One outbound operation, without an extra queue. Its existing write may settle
/// after local stop; the returned capability is then usable only for one Leave.
pub struct DiagnosticWrite {
    prepared: DiagnosticPrepared,
    guard: TicketGuard,
}
pub struct DiagnosticWriteCompletion {
    completion: DiagnosticCompletion,
    guard: TicketGuard,
}
impl DiagnosticWrite {
    pub async fn wait(self) -> DiagnosticWriteCompletion {
        DiagnosticWriteCompletion {
            completion: self.prepared.wait().await,
            guard: self.guard,
        }
    }
}
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum DiagnosticWriteResult {
    AcceptedForProcessing,
    Left,
}

impl NextcloudClient {
    pub fn diagnostic_start_duplex(
        &mut self,
        context: &OperationContext,
        call: CallScope,
    ) -> Result<(), PortError> {
        self.diagnostic_scope(context, call)?;
        let state = &mut self.diagnostic_signaling;
        if let Err(kind) = state.validate(call) {
            self.diagnostic_stop_signaling();
            return Err(self.error(kind));
        }
        if state.duplex {
            return Err(self.error(ErrorKind::Conflict));
        }
        if state.selected.is_none() {
            return Err(self.error(ErrorKind::WrongScope));
        }
        if state.response.upgrade().is_some() || state.mailbox.is_some() {
            return Err(self.error(ErrorKind::Busy));
        }
        let token = state._admission.as_mut().expect("validated admitted scope");
        let (reader, outbound) = self.transport.diagnostic_duplex(context, token)?;
        let state = &mut self.diagnostic_signaling;
        state.reader = Some(reader);
        state.outbound = Some(outbound);
        state.duplex = true;
        Ok(())
    }
    fn diagnostic_ticket_guard(
        &mut self,
        context: &OperationContext,
        call: CallScope,
        leave: bool,
    ) -> Result<TicketGuard, PortError> {
        self.diagnostic_scope(context, call)?;
        let state = &self.diagnostic_signaling;
        if !leave {
            state.validate(call).map_err(|kind| self.error(kind))?;
        }
        if !state.duplex || state.call != Some(call) {
            return Err(self.error(ErrorKind::WrongScope));
        }
        let room = self
            .diagnostic_membership_scope
            .filter(|(bound, _)| *bound == call)
            .map(|(_, room)| room)
            .ok_or_else(|| self.error(ErrorKind::WrongScope))?;
        Ok(TicketGuard {
            context: OperationContext {
                scope: context.scope,
                deadline: context.deadline,
                cancellation: context.cancellation.clone(),
            },
            call,
            selected: state.selected.as_ref().map(DiagnosticPeer::shared),
            awaiting_offer: state.awaiting_offer,
            room,
            local: zeroize::Zeroizing::new(
                self.active
                    .as_ref()
                    .expect("scope checked")
                    .session
                    .as_str()
                    .to_owned(),
            ),
            ended: state.ended.clone(),
            armed: true,
            leave,
        })
    }
    pub fn diagnostic_prepare_read(
        &mut self,
        context: &OperationContext,
        call: CallScope,
    ) -> Result<DiagnosticRead, PortError> {
        self.diagnostic_row_counts = None;
        self.diagnostic_poll_stage = DiagnosticPollStage::Scope;
        self.diagnostic_scope(context, call)?;
        if let Err(kind) = self.diagnostic_signaling.validate(call) {
            self.diagnostic_stop_signaling();
            return Err(self.error(kind));
        }
        let state = &self.diagnostic_signaling;
        if !state.duplex {
            return Err(self.error(ErrorKind::WrongScope));
        }
        if state.reader.is_none() || state.mailbox.is_some() || state.response.upgrade().is_some() {
            return Err(self.error(ErrorKind::Busy));
        }
        // Shared response charge is acquired before preparing/issuing GET.
        let slot = self.transport.diagnostic_response_slot()?;
        let route = self.route(call.conversation, "v3/signaling", "")?;
        let guard = self.diagnostic_ticket_guard(context, call, false)?;
        let started = self.transport.now();
        let available = context
            .deadline
            .millis
            .saturating_sub(started.millis)
            .min(35_000);
        let lease = Arc::new(ResponseLease {
            _slot: slot,
            valid: AtomicBool::new(true),
            ended: guard.ended.clone(),
            account_cancellation: self.transport.cancellation(),
            operation_cancellation: context.cancellation.clone(),
            expires: Instant::now() + Duration::from_millis(available),
            request_started: started,
        });
        let lane = self
            .diagnostic_signaling
            .reader
            .take()
            .expect("reader availability checked");
        let prepared = self.transport.diagnostic_prepare(
            context,
            call,
            lane,
            DiagnosticOperation::Read { route: &route },
        )?;
        self.diagnostic_signaling.polling = true;
        self.diagnostic_signaling.response = Arc::downgrade(&lease);
        self.diagnostic_poll_stage = DiagnosticPollStage::Transport;
        Ok(DiagnosticRead {
            prepared,
            guard,
            lease,
        })
    }
    pub fn diagnostic_finish_read(
        &mut self,
        context: &OperationContext,
        call: CallScope,
        finished: DiagnosticReadCompletion,
    ) -> Result<DiagnosticPoll, PortError> {
        let DiagnosticReadCompletion {
            completion,
            mut guard,
            lease,
        } = finished;
        let validated = guard
            .validate(self, context, call)
            .and_then(|()| lease.validate().map_err(|kind| self.error(kind)));
        if let Err(error) = validated {
            drop(completion.discard());
            self.diagnostic_stop_signaling();
            return Err(error);
        }
        let (lane, response) = self.transport.diagnostic_commit(context, call, completion);
        let response = match response {
            Ok(response) => response,
            Err(error) => {
                drop(lane);
                self.diagnostic_stop_signaling();
                return Err(error);
            }
        };
        let parsed = parse_response(
            &response.data,
            guard.local.as_str(),
            guard.room,
            &self.diagnostic_signaling,
            lease,
            &mut self.diagnostic_poll_stage,
            &mut self.diagnostic_row_counts,
        );
        let (mailbox, selected, candidates, descriptions) = match parsed {
            Ok(parsed) => parsed,
            Err(kind) => {
                drop(lane);
                self.diagnostic_stop_signaling();
                return Err(self.error(kind));
            }
        };
        // Parsing is synchronous but deadline/cancellation can change independently.
        guard.validate(self, context, call)?;
        let state = &mut self.diagnostic_signaling;
        state.selected = selected;
        state.awaiting_offer = state.awaiting_offer && descriptions == 0;
        state.candidates = candidates;
        state.descriptions = descriptions;
        state.polling = false;
        state.mailbox = Some(mailbox);
        state.reader = Some(lane);
        guard.armed = false;
        self.diagnostic_drain(context, call)
    }
    /// No I/O; the original lease/scope/deadline applies to every borrowed page.
    pub fn diagnostic_drain_page(
        &mut self,
        context: &OperationContext,
        call: CallScope,
    ) -> Result<DiagnosticPoll, PortError> {
        self.diagnostic_drain(context, call)
    }
    pub fn diagnostic_prepare_send(
        &mut self,
        context: &OperationContext,
        peer: &DiagnosticPeer,
        signal: Signal,
    ) -> Result<DiagnosticWrite, PortError> {
        self.diagnostic_scope(context, peer.0.call)?;
        if let Err(kind) = self.diagnostic_signaling.validate(peer.0.call) {
            self.diagnostic_stop_signaling();
            return Err(self.error(kind));
        }
        if !self.diagnostic_signaling.duplex {
            return Err(self.error(ErrorKind::WrongScope));
        }
        if self.diagnostic_signaling.outbound.is_none() {
            return Err(self.error(ErrorKind::Busy));
        }
        let messages = self.diagnostic_messages(context, peer, signal)?;
        let route = self.route(peer.0.call.conversation, "v3/signaling", "")?;
        let guard = self.diagnostic_ticket_guard(context, peer.0.call, false)?;
        let lane = self
            .diagnostic_signaling
            .outbound
            .take()
            .expect("outbound availability checked");
        let prepared = self.transport.diagnostic_prepare(
            context,
            peer.0.call,
            lane,
            DiagnosticOperation::Signal {
                route: &route,
                messages: &messages,
            },
        )?;
        Ok(DiagnosticWrite { prepared, guard })
    }
    pub fn diagnostic_prepare_leave(
        &mut self,
        context: &OperationContext,
        call: CallScope,
    ) -> Result<DiagnosticWrite, PortError> {
        self.diagnostic_scope(context, call)?;
        self.diagnostic_stop_signaling();
        if self.diagnostic_signaling.outbound.is_none() {
            return Err(self.error(ErrorKind::Busy));
        }
        let route = self.route(call.conversation, "v4/call", "")?;
        let guard = self.diagnostic_ticket_guard(context, call, true)?;
        let lane = self
            .diagnostic_signaling
            .outbound
            .take()
            .expect("outbound availability checked");
        let prepared = self.transport.diagnostic_prepare(
            context,
            call,
            lane,
            DiagnosticOperation::Leave { route: &route },
        )?;
        Ok(DiagnosticWrite { prepared, guard })
    }
    pub fn diagnostic_finish_write(
        &mut self,
        context: &OperationContext,
        call: CallScope,
        finished: DiagnosticWriteCompletion,
    ) -> Result<DiagnosticWriteResult, PortError> {
        let DiagnosticWriteCompletion {
            completion,
            mut guard,
        } = finished;
        let (lane, result) = match guard.validate(self, context, call) {
            Ok(()) => self.transport.diagnostic_commit(context, call, completion),
            Err(error) => (completion.discard(), Err(error)),
        };
        let current = Arc::ptr_eq(&guard.ended, &self.diagnostic_signaling.ended)
            && self.diagnostic_signaling.call == Some(guard.call);
        if current && !guard.leave {
            self.diagnostic_signaling.outbound = Some(lane);
        } else {
            drop(lane);
        }
        guard.armed = false;
        match result {
            Err(error) => {
                self.diagnostic_stop_signaling();
                Err(error)
            }
            Ok(_) if guard.leave => {
                self.diagnostic_stop_signaling();
                self.observed_call = None;
                self.diagnostic_membership_scope = None;
                Ok(DiagnosticWriteResult::Left)
            }
            Ok(_) => Ok(DiagnosticWriteResult::AcceptedForProcessing),
        }
    }
}

// This transaction owns at most66 fixed slots. It validates every row before
// committing either counters or a page, and charges text before retaining it.
fn parse_response(
    value: &Value,
    local: &str,
    room: u64,
    state: &SignalingState,
    lease: Arc<ResponseLease>,
    stage: &mut DiagnosticPollStage,
    counts: &mut Option<DiagnosticRowCounts>,
) -> Result<(Mailbox, Option<DiagnosticPeer>, u8, u8), ErrorKind> {
    *stage = DiagnosticPollStage::Rows;
    let rows = value.as_array().ok_or(ErrorKind::Malformed)?;
    *counts = Some(DiagnosticRowCounts::observe(rows)); // Original saturating semantics.
    if rows.len() > ROWS {
        return Err(ErrorKind::TooLarge);
    }
    let mut signals = Vec::new();
    signals
        .try_reserve_exact(SIGNALS)
        .map_err(|_| ErrorKind::Busy)?;
    if signals.capacity() != SIGNALS {
        return Err(ErrorKind::TooLarge);
    }
    let mut selected = state.selected.as_ref().map(DiagnosticPeer::shared);
    let mut awaiting = state.awaiting_offer;
    let (mut candidates, mut descriptions) = (state.candidates, state.descriptions);
    let (mut ignored, mut snapshots) = (0, 0);
    let mut membership_result = Membership::default();
    // Conservatively charge the shared selected identity in each response, in
    // addition to its one separately bounded lifetime allocation.
    let mut bytes = selected.as_ref().map_or(0, |peer| {
        peer.0.session.len() + peer.0.negotiation.as_str().len()
    });
    for row in rows {
        lease.validate()?;
        match row.get("type").and_then(Value::as_str) {
            Some("usersInRoom") => {
                *stage = DiagnosticPollStage::Membership;
                snapshots += 1;
                if snapshots > 2 {
                    return Err(ErrorKind::TooLarge);
                }
                let users = row
                    .get("data")
                    .and_then(Value::as_array)
                    .ok_or(ErrorKind::Malformed)?;
                if users.len() > 17 {
                    return Err(ErrorKind::TooLarge);
                }
                let (present, in_call) = membership(users, room, selected.as_ref())?;
                membership_result = Membership {
                    peer_count: users.len() as u16,
                    observed: true,
                    selected_present: present,
                    selected_in_call: in_call,
                };
            }
            Some("message") => {
                *stage = DiagnosticPollStage::EnvelopeText;
                let raw = borrowed_text(row.get("data"), 131072)?;
                *stage = DiagnosticPollStage::EnvelopeJson;
                let data = wire::json(raw.as_bytes())?;
                *stage = DiagnosticPollStage::RoomType;
                if data.get("roomType").and_then(Value::as_str) != Some("video") {
                    return Err(ErrorKind::Unsupported);
                }
                *stage = DiagnosticPollStage::Sender;
                let from = borrowed_text(data.get("from"), 512)?;
                checked_session(from)?;
                let peer = selected.as_ref().ok_or(ErrorKind::WrongScope)?;
                if from != peer.0.session.as_str() {
                    return Err(ErrorKind::WrongScope);
                }
                *stage = DiagnosticPollStage::Recipient;
                if data.get("to").and_then(Value::as_str) != Some(local) {
                    return Err(ErrorKind::WrongScope);
                }
                let kind = data
                    .get("type")
                    .and_then(Value::as_str)
                    .ok_or(ErrorKind::Malformed)?;
                if matches!(kind, "mute" | "unmute" | "endOfCandidates") {
                    ignored += 1;
                    if ignored > 16 {
                        return Err(ErrorKind::TooLarge);
                    }
                    if let Some(sid) = data.get("sid") {
                        let sid = borrowed_text(Some(sid), 64)?;
                        checked_sid(sid)?;
                        if !awaiting && sid != peer.0.negotiation.as_str() {
                            return Err(ErrorKind::WrongScope);
                        }
                    }
                    let payload = data.get("payload");
                    if kind == "endOfCandidates" {
                        if payload.is_some_and(|p| {
                            !p.is_null() && p.as_object().is_none_or(|o| !o.is_empty())
                        }) {
                            return Err(ErrorKind::Unsupported);
                        }
                    } else if !matches!(
                        payload.and_then(|p| p.get("name")).and_then(Value::as_str),
                        Some("audio" | "video")
                    ) {
                        return Err(ErrorKind::Unsupported);
                    }
                    continue; // Never a native command or capture authority.
                }
                if !matches!(kind, "offer" | "answer" | "candidate") {
                    return Err(ErrorKind::Unsupported);
                }
                *stage = DiagnosticPollStage::Negotiation;
                let sid = borrowed_text(data.get("sid"), 64)?;
                checked_sid(sid)?;
                if awaiting {
                    if kind != "offer" {
                        return Err(ErrorKind::Conflict);
                    }
                    // New identity before copy, charged explicitly even though
                    // the previous generated SID will drop at transaction end.
                    charge(&mut bytes, from.len() + sid.len())?;
                    selected = Some(DiagnosticPeer::new(peer.0.call, from, sid, false)?);
                    awaiting = false;
                } else if sid != peer.0.negotiation.as_str() {
                    return Err(ErrorKind::WrongScope);
                }
                *stage = DiagnosticPollStage::Payload;
                let payload = data.get("payload").ok_or(ErrorKind::Malformed)?;
                let signal = match kind {
                    "offer" | "answer" => {
                        descriptions = descriptions.checked_add(1).ok_or(ErrorKind::TooLarge)?;
                        if descriptions > DESCRIPTIONS {
                            return Err(ErrorKind::TooLarge);
                        }
                        if payload.get("type").and_then(Value::as_str) != Some(kind) {
                            return Err(ErrorKind::Malformed);
                        }
                        let sdp = borrowed_text(payload.get("sdp"), 65536)?;
                        *stage = DiagnosticPollStage::Description;
                        SessionDescription::validate(sdp).map_err(|_| ErrorKind::Malformed)?;
                        charge(&mut bytes, sdp.len())?;
                        if kind == "offer" {
                            RetainedSignal::Offer(sdp.into())
                        } else {
                            RetainedSignal::Answer(sdp.into())
                        }
                    }
                    "candidate" => {
                        candidates = candidates.checked_add(1).ok_or(ErrorKind::TooLarge)?;
                        if candidates > CANDIDATES {
                            return Err(ErrorKind::TooLarge);
                        }
                        let payload = payload.get("candidate").ok_or(ErrorKind::Malformed)?;
                        let candidate = borrowed_text(payload.get("candidate"), 2048)?;
                        let mid = borrowed_text(payload.get("sdpMid"), 64)?;
                        let index = payload
                            .get("sdpMLineIndex")
                            .and_then(Value::as_u64)
                            .and_then(|n| u8::try_from(n).ok())
                            .filter(|n| *n < 24)
                            .ok_or(ErrorKind::Malformed)?;
                        *stage = DiagnosticPollStage::Candidate;
                        IceCandidate::validate(mid, index, candidate)
                            .map_err(|_| ErrorKind::Malformed)?;
                        charge(&mut bytes, candidate.len() + mid.len())?;
                        RetainedSignal::Candidate {
                            candidate: candidate.into(),
                            mid: mid.into(),
                            index,
                        }
                    }
                    _ => unreachable!(),
                };
                *stage = DiagnosticPollStage::Capacity;
                if signals.len() >= SIGNALS {
                    return Err(ErrorKind::TooLarge);
                }
                signals.push(ReceivedSignal {
                    peer: selected.as_ref().ok_or(ErrorKind::WrongScope)?.shared(),
                    signal,
                });
            }
            _ => return Err(ErrorKind::Unsupported),
        }
    }
    Ok((
        Mailbox {
            lease,
            signals,
            membership: membership_result,
            typed_bytes: bytes,
        },
        selected,
        candidates,
        descriptions,
    ))
}

/// Talk 24.0.6 / 25.0.6 SignalingController::getUsersInRoom. Membership is the
/// sessionId in a scoped snapshot, never an actor ID, display name or row count.
fn membership(
    users: &[Value],
    room: u64,
    selected: Option<&DiagnosticPeer>,
) -> Result<(Option<bool>, Option<bool>), ErrorKind> {
    let mut in_call = None;
    let mut sessions = Vec::with_capacity(users.len());
    for user in users {
        let number = user
            .get("roomId")
            .and_then(Value::as_u64)
            .filter(|id| *id > 0)
            .ok_or(ErrorKind::Malformed)?;
        if number != room {
            return Err(ErrorKind::WrongScope);
        }
        let session = user
            .get("sessionId")
            .and_then(Value::as_str)
            .filter(|s| {
                !s.is_empty()
                    && *s != "0"
                    && s.len() <= 512
                    && s.bytes().all(|b| b.is_ascii_graphic())
            })
            .ok_or(ErrorKind::Malformed)?;
        if sessions.contains(&session)
            || user.get("lastPing").and_then(Value::as_u64).is_none()
            || user.get("inCall").is_some_and(|value| {
                !value.is_null() && value.as_u64().is_none_or(|flags| flags > u32::MAX as u64)
            })
            || user
                .get("participantPermissions")
                .and_then(Value::as_u64)
                .is_none()
            || ["userId", "actorType", "actorId"]
                .iter()
                .any(|key| user.get(key).and_then(Value::as_str).is_none())
        {
            return Err(ErrorKind::Malformed);
        }
        if selected.is_some_and(|peer| peer.0.session.as_str() == session) {
            // Missing/null and unsupported or unknown flag sets carry no call-exit
            // observation. Only the exact documented IN_CALL bit in a known
            // admitted AV flag set yields Some. WITH_PHONE is not admitted by
            // this diagnostic profile. Presence remains a separate observation.
            in_call = user
                .get("inCall")
                .and_then(Value::as_u64)
                .filter(|flags| flags & !7 == 0)
                .map(|flags| flags & 1 != 0);
        }
        sessions.push(session);
    }
    Ok((
        selected.map(|peer| sessions.contains(&peer.0.session.as_str())),
        in_call,
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    use talk_core::ids::{
        AccountId, AccountScope, CallGeneration, ConversationId, HostId, RuntimeGeneration,
    };
    fn peer(call: CallScope, session: &str, sid: &str) -> DiagnosticPeer {
        DiagnosticPeer::new(call, session, sid, false).unwrap()
    }
    #[test]
    fn same_session_ignores_only_sid() {
        let account = AccountScope {
            account: AccountId::from_local(1),
            runtime: RuntimeGeneration::from_local(2),
        };
        let call = CallScope {
            account,
            conversation: ConversationId {
                account: account.account,
                host: HostId::from_local(3),
                local: 4,
            },
            call: CallGeneration::from_local(5),
        };
        let selected = peer(call, "session-one", "sid-one");
        assert!(selected.same_session(&peer(call, "session-one", "sid-two")));
        assert!(!selected.same_negotiation(&peer(call, "session-one", "sid-two")));
        assert!(!selected.same_session(&peer(call, "session-two", "sid-one")));
        let mut variants = [call; 6];
        variants[0].account.account = AccountId::from_local(11);
        variants[1].account.runtime = RuntimeGeneration::from_local(12);
        variants[2].conversation.account = AccountId::from_local(11);
        variants[3].conversation.host = HostId::from_local(13);
        variants[4].conversation.local = 14;
        variants[5].call = CallGeneration::from_local(15);
        for other in variants {
            assert!(!selected.same_session(&peer(other, "session-one", "sid-one")));
        }
    }
}

#[cfg(test)]
#[path = "diagnostic_tests/mailbox.rs"]
mod mailbox_tests;

/// Unit-fixture admission only. Real backend-test callers must complete join.
#[cfg(test)]
pub(super) fn admit_fixture(
    owner: &mut NextcloudClient,
    context: &OperationContext,
    call: CallScope,
    session: &str,
    sid: &str,
) {
    let token = owner
        .transport
        .diagnostic_call_admission(context, call)
        .unwrap();
    owner.diagnostic_signaling = SignalingState::admitted(token, call);
    owner.diagnostic_signaling.selected =
        Some(DiagnosticPeer::new(call, session, sid, true).unwrap());
    owner.observed_call = Some(call);
    owner.diagnostic_membership_scope = Some((call, 7));
}

#[cfg(test)]
#[path = "diagnostic_tests/duplex.rs"]
mod duplex_tests;
