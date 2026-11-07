//! Account-bound HPB protocol owner. One socket per admitted Client; the existing
//! account scheduler drains invalidations and performs REST reconciliation.
//! No account password, cookies, device authority or application policy enters
//! the WebSocket task. Sources and wire invariants are recorded in `wire`.
mod session;
mod settings;
mod state;
mod wire;
pub use settings::ExternalSettings;

use crate::{
    Client, Error, ErrorKind, NetworkRuntime, Retry,
    events::{LiveEvent, LiveEventBatch, LiveState, Mailbox},
    models::{RoomToken, SignalingMode, SignalingSettings},
    scope::{Cancellation, Moment, OperationContext, SessionScope},
    signaling::{Event, Negotiation, PeerSession, Signal, SignalBatch, StreamKind},
};
use std::{
    collections::VecDeque,
    future::Future,
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
    },
    time::Duration,
};
use tokio::sync::{mpsc, oneshot};
use zeroize::Zeroizing;

pub(crate) struct AccountBinding {
    pub(crate) scope: SessionScope,
    pub(crate) backend: String,
    pub(crate) origin: String,
    pub(crate) user: Zeroizing<String>,
    pub(crate) cancellation: Cancellation,
    pub(crate) runtime: NetworkRuntime,
    // Retain the existing account charge until its socket task has terminated.
    pub(crate) _charge: Arc<crate::transport::AccountCharge>,
}
struct Shared {
    account: AccountBinding,
    stop: Cancellation,
    settings: Mutex<ExternalSettings>,
    state: Mutex<State>,
}
struct State {
    status: LiveState,
    first_transport_failure: Option<ErrorKind>,
    events: Mailbox,
    session: Option<PeerSession>,
    mcu: bool,
    generation: u64,
    room: Option<Room>,
}
#[derive(Clone, Copy, Eq, PartialEq)]
enum RoomPurpose {
    Call,
    Chat,
}
struct Room {
    purpose: RoomPurpose,
    replaced: Arc<AtomicBool>,
    token: RoomToken,
    own: PeerSession,
    generation: u64,
    cancellation: Cancellation,
    joined: bool,
    members: Vec<wire::Member>,
    in_call: Vec<PeerSession>,
    subscription_requests: Vec<String>,
    selected: Vec<Selection>,
    retired_screens: Vec<(PeerSession, Negotiation)>,
    retired_subscriptions: Vec<(PeerSession, StreamKind, Negotiation)>,
    signals: VecDeque<(usize, Event)>,
    signal_bytes: usize,
    typing: Vec<(PeerSession, std::time::Instant)>,
}
struct Selection {
    request_id: Option<String>,
    stream: StreamKind,
    talk: PeerSession,
    sfu: PeerSession,
    sid: Option<Negotiation>,
    publisher: bool,
}
impl Room {
    fn retire_subscription(
        &mut self,
        peer: &PeerSession,
        stream: StreamKind,
    ) -> Result<(), ErrorKind> {
        for old in self
            .selected
            .iter()
            .filter(|p| &p.talk == peer && p.stream == stream && !p.publisher)
        {
            if let Some(sid) = &old.sid {
                if self.retired_subscriptions.len() >= 64 {
                    return Err(ErrorKind::Busy);
                }
                self.retired_subscriptions
                    .push((old.sfu.clone(), stream, sid.clone()));
            }
        }
        self.selected
            .retain(|p| &p.talk != peer || p.stream != stream || p.publisher);
        Ok(())
    }
    fn membership(&mut self, own_sfu: &PeerSession) -> Result<(), ErrorKind> {
        if self.purpose != RoomPurpose::Call {
            return Ok(());
        }
        // A room join is only routing information. HPB rejects requestoffer
        // until both SFU sessions are in its call (hub.go isInSameCall).
        // This gate narrows the REST admission; it never grants media itself.
        let members = if self.in_call.contains(own_sfu) {
            self.members
                .iter()
                .filter(|m| self.in_call.contains(&m.sfu))
                .map(|m| m.talk.clone())
                .collect()
        } else {
            Vec::new()
        };
        self.push(Event::SfuMembership(members))
    }
    fn push(&mut self, event: Event) -> Result<(), ErrorKind> {
        let bytes = match &event {
            Event::Signal {
                signal: Signal::Description(s),
                ..
            } => s.as_str().len(),
            Event::Signal {
                signal: Signal::Candidate(c),
                ..
            } => c.candidate().len() + c.mid().len(),
            _ => 1024,
        };
        // Ephemeral activity never displaces a required media or policy event.
        while self.signals.len() >= 84 || self.signal_bytes + bytes > 192 * 1024 {
            let Some(index) = self
                .signals
                .iter()
                .position(|(_, event)| matches!(event, Event::Activity { .. }))
            else {
                break;
            };
            if let Some((size, _)) = self.signals.remove(index) {
                self.signal_bytes -= size;
            }
        }
        if self.signals.len() >= 84 || self.signal_bytes + bytes > 192 * 1024 {
            self.cancellation.cancel();
            return Err(ErrorKind::Busy);
        }
        self.signal_bytes += bytes;
        self.signals.push_back((bytes, event));
        Ok(())
    }
    fn push_activity(&mut self, from: PeerSession, activity: crate::signaling::activity::Activity) {
        use crate::signaling::activity::Activity;
        // Coalesce each hand so a delayed raise cannot replace a later lower.
        if let Activity::RaisedHand { timestamp, .. } = &activity
            && let Some((_, event)) = self.signals.iter_mut().find(|(_, event)| {
                matches!(event, Event::Activity { from: sender, activity: Activity::RaisedHand { .. } } if sender == &from)
            })
        {
            if let Event::Activity { activity: Activity::RaisedHand { timestamp: previous, .. }, .. } = event
                && *timestamp > *previous
            {
                *event = Event::Activity { from, activity };
            }
            return;
        }
        let count = self
            .signals
            .iter()
            .filter(|(_, event)| matches!(event, Event::Activity { .. }))
            .count();
        if count >= 16 || self.signals.len() >= 84 || self.signal_bytes + 1024 > 192 * 1024 {
            return;
        }
        self.signal_bytes += 1024;
        self.signals
            .push_back((1024, Event::Activity { from, activity }));
    }
}
pub(crate) struct Owner {
    handle: Handle,
}
#[derive(Clone)]
pub(crate) struct Handle {
    shared: Arc<Shared>,
    commands: mpsc::Sender<Command>,
}
enum Command {
    Join {
        generation: u64,
        deadline: Moment,
        cancel: Cancellation,
        reply: oneshot::Sender<Result<(), ErrorKind>>,
    },
    Typing {
        generation: u64,
        started: bool,
        deadline: Moment,
        cancel: Cancellation,
        reply: oneshot::Sender<Result<(), ErrorKind>>,
    },
    ForceMute {
        generation: u64,
        peer: PeerSession,
        target: PeerSession,
        deadline: Moment,
        cancel: Cancellation,
        reply: oneshot::Sender<Result<(), ErrorKind>>,
    },
    Activity {
        generation: u64,
        peer: PeerSession,
        activity: crate::signaling::activity::Activity,
        deadline: Moment,
        cancel: Cancellation,
        reply: oneshot::Sender<Result<(), ErrorKind>>,
    },
    Send {
        generation: u64,
        stream: StreamKind,
        peer: PeerSession,
        sid: Option<Negotiation>,
        signal: Option<Signal>,
        deadline: Moment,
        cancel: Cancellation,
        reply: oneshot::Sender<Result<(), ErrorKind>>,
    },
}
impl Command {
    fn reject(self, error: ErrorKind) {
        match self {
            Self::Join { reply, .. }
            | Self::Typing { reply, .. }
            | Self::Send { reply, .. }
            | Self::Activity { reply, .. }
            | Self::ForceMute { reply, .. } => {
                let _ = reply.send(Err(error));
            }
        }
    }
}
impl Shared {
    fn error(&self, kind: ErrorKind) -> Error {
        Error {
            kind,
            account: Some(self.account.scope.account),
            retry: Retry::AfterReconciliation,
        }
    }
    fn live(&self) -> bool {
        !self.stop.is_cancelled() && !self.account.cancellation.is_cancelled()
    }
    fn validate(&self, context: &OperationContext) -> Result<(), ErrorKind> {
        if context.scope != self.account.scope {
            return Err(ErrorKind::WrongScope);
        }
        if !self.live() || context.cancellation.is_cancelled() {
            return Err(ErrorKind::Cancelled);
        }
        let now = self.account.runtime.now();
        if context.deadline.epoch != now.epoch || context.deadline.millis <= now.millis {
            return Err(ErrorKind::Deadline);
        }
        Ok(())
    }
    fn status(&self, status: LiveState) {
        if let Ok(mut state) = self.state.lock() {
            state.status = status;
            if matches!(status, LiveState::Stopped(_))
                && let Some(room) = &state.room
            {
                room.cancellation.cancel();
            }
        }
    }
}
impl Owner {
    pub(crate) fn start(
        account: AccountBinding,
        settings: ExternalSettings,
    ) -> Result<Self, ErrorKind> {
        if account.user.as_str() != settings.user.as_str() {
            return Err(ErrorKind::WrongScope);
        }
        let shared = Arc::new(Shared {
            account,
            stop: Cancellation::default(),
            settings: Mutex::new(settings),
            state: Mutex::new(State {
                status: LiveState::Connecting,
                first_transport_failure: None,
                events: Mailbox::default(),
                session: None,
                mcu: false,
                generation: 0,
                room: None,
            }),
        });
        let (commands, receiver) = mpsc::channel(32);
        tokio::spawn(session::run(shared.clone(), receiver));
        Ok(Self {
            handle: Handle { shared, commands },
        })
    }
    pub(crate) fn handle(&self) -> Handle {
        self.handle.clone()
    }
    pub(crate) fn retirement_pending(&self) -> bool {
        self.handle
            .shared
            .state
            .lock()
            .map_or(true, |s| s.room.is_some())
    }
    fn transport_stopped(&self) -> bool {
        self.handle.shared.state.lock().is_ok_and(|state| {
            matches!(
                state.status,
                LiveState::Stopped(ErrorKind::Transport | ErrorKind::Deadline)
            )
        })
    }
    fn poll(&self, context: &OperationContext) -> Result<LiveEventBatch, Error> {
        let shared = &self.handle.shared;
        shared.validate(context).map_err(|e| shared.error(e))?;
        let mut state = shared
            .state
            .lock()
            .map_err(|_| shared.error(ErrorKind::Uncertain))?;
        Ok(LiveEventBatch {
            scope: context.scope,
            state: state.status,
            first_transport_failure: state.first_transport_failure,
            events: state.events.drain(),
        })
    }
}
impl Drop for Owner {
    fn drop(&mut self) {
        self.handle.shared.stop.cancel();
    }
}
/// A single current call route. Cloning shares irreversible closure, never a new
/// room/call generation. Only a successful HTTP room activation can create it.
#[derive(Clone)]
pub(crate) struct Route {
    replaced: Arc<AtomicBool>,
    handle: Handle,
    generation: u64,
    cancellation: Cancellation,
}
impl Handle {
    pub(crate) fn update(&self, settings: &ExternalSettings) -> Result<(), ErrorKind> {
        let mut current = self
            .shared
            .settings
            .lock()
            .map_err(|_| ErrorKind::Uncertain)?;
        if current.servers != settings.servers || current.user != settings.user {
            self.shared.stop.cancel();
            return Err(ErrorKind::WrongScope);
        }
        *current = settings.clone();
        Ok(())
    }
    pub(crate) async fn join(
        &self,
        context: &OperationContext,
        token: RoomToken,
        own: PeerSession,
    ) -> Result<Route, ErrorKind> {
        self.join_for(context, token, own, RoomPurpose::Call).await
    }
    pub(crate) async fn join_chat(
        &self,
        context: &OperationContext,
        token: RoomToken,
        own: PeerSession,
    ) -> Result<Route, ErrorKind> {
        self.join_for(context, token, own, RoomPurpose::Chat).await
    }
    async fn join_for(
        &self,
        context: &OperationContext,
        token: RoomToken,
        own: PeerSession,
        purpose: RoomPurpose,
    ) -> Result<Route, ErrorKind> {
        self.shared.validate(context)?;
        let cancellation = Cancellation::default();
        let replaced = Arc::new(AtomicBool::new(false));
        let generation = {
            let mut state = self.shared.state.lock().map_err(|_| ErrorKind::Uncertain)?;
            if let Some(room) = &state.room {
                if !room.cancellation.is_cancelled() {
                    return Err(ErrorKind::Busy);
                }
                // Socket retirement must complete before another call admission.
                return Err(ErrorKind::Uncertain);
            }
            state.generation = state.generation.checked_add(1).ok_or(ErrorKind::Busy)?;
            let generation = state.generation;
            state.room = Some(Room {
                purpose,
                replaced: replaced.clone(),
                token,
                own,
                generation,
                cancellation: cancellation.clone(),
                joined: false,
                members: Vec::with_capacity(100),
                in_call: Vec::with_capacity(100),
                subscription_requests: Vec::with_capacity(64),
                selected: Vec::with_capacity(17),
                retired_screens: Vec::with_capacity(32),
                retired_subscriptions: Vec::with_capacity(64),
                signals: VecDeque::new(),
                signal_bytes: 0,
                typing: Vec::with_capacity(100),
            });
            generation
        };
        let route = Route {
            replaced,
            handle: self.clone(),
            generation,
            cancellation,
        };
        let (reply, receive) = oneshot::channel();
        let guard = Abort(route.cancellation.clone(), false);
        self.commands
            .try_send(Command::Join {
                generation,
                deadline: context.deadline,
                cancel: context.cancellation.clone(),
                reply,
            })
            .map_err(|_| ErrorKind::Busy)?;
        until(
            &self.shared,
            context.deadline,
            &context.cancellation,
            async { receive.await.map_err(|_| ErrorKind::Transport)? },
        )
        .await?;
        self.shared.validate(context)?;
        drop(guard.disarm());
        Ok(route)
    }
}
struct Abort(Cancellation, bool);
impl Abort {
    fn disarm(mut self) -> Self {
        self.1 = true;
        self
    }
}
impl Drop for Abort {
    fn drop(&mut self) {
        if !self.1 {
            self.0.cancel();
        }
    }
}
impl Route {
    pub(crate) fn session_replaced(&self) -> bool {
        self.replaced.load(Ordering::Acquire)
    }
    pub(crate) fn update(&self, settings: &ExternalSettings) -> Result<(), ErrorKind> {
        self.handle.update(settings)
    }
    pub(crate) fn matches(&self, token: &RoomToken, own: &PeerSession) -> bool {
        self.with_room(|room, _| Ok(&room.token == token && &room.own == own))
            .unwrap_or(false)
    }
    pub(crate) fn close(&self) {
        self.cancellation.cancel();
    }
    pub(crate) fn is_closed(&self) -> bool {
        self.cancellation.is_cancelled() || !self.handle.shared.live()
    }
    fn with_room<T>(
        &self,
        f: impl FnOnce(&mut Room, &PeerSession) -> Result<T, ErrorKind>,
    ) -> Result<T, ErrorKind> {
        if self.is_closed() {
            return Err(ErrorKind::Cancelled);
        }
        let mut state = self
            .handle
            .shared
            .state
            .lock()
            .map_err(|_| ErrorKind::Uncertain)?;
        let session = state.session.clone().ok_or(ErrorKind::Transport)?;
        let room = state.room.as_mut().ok_or(ErrorKind::WrongScope)?;
        if room.generation != self.generation || !room.joined {
            return Err(ErrorKind::WrongScope);
        }
        f(room, &session)
    }
    #[cfg(test)]
    pub(crate) fn select(&self, peer: PeerSession, sid: Negotiation) -> Result<(), ErrorKind> {
        self.select_stream(peer, sid, StreamKind::Camera)
    }
    pub(crate) fn select_stream(
        &self,
        peer: PeerSession,
        sid: Negotiation,
        stream: StreamKind,
    ) -> Result<(), ErrorKind> {
        self.with_room(|room, own_sfu| {
            if let Some(selection) = room
                .selected
                .iter_mut()
                .find(|p| p.talk == peer && p.stream == stream)
            {
                return if selection.sid.as_ref() == Some(&sid) {
                    Ok(())
                } else {
                    Err(ErrorKind::Conflict)
                };
            }
            // Only the publisher's SID is client-created. A subscriber must
            // first request an offer and bind the SID supplied by the SFU.
            if peer != room.own {
                return Err(ErrorKind::WrongScope);
            }
            if room.selected.len() >= 20 {
                return Err(ErrorKind::Busy);
            }
            room.selected.push(Selection {
                stream,
                talk: peer,
                sfu: own_sfu.clone(),
                sid: Some(sid),
                publisher: true,
                request_id: None,
            });
            Ok(())
        })
    }
    pub(crate) fn remove_stream(&self, peer: &PeerSession, stream: StreamKind) {
        let _ = self.with_room(|room, _| {
            if stream == StreamKind::Screen {
                for selected in room
                    .selected
                    .iter()
                    .filter(|p| &p.talk == peer && p.stream == stream)
                {
                    if let Some(sid) = &selected.sid
                        && room.retired_screens.len() < 32
                    {
                        room.retired_screens
                            .push((selected.sfu.clone(), sid.clone()));
                    }
                }
            }
            // Retire the negotiation, not the participant's routing identity.
            // A fresh REST observation and current HPB call membership can
            // admit a new subscription after that participant unmutes/rejoins.
            if let Err(error) = room.retire_subscription(peer, stream) {
                room.cancellation.cancel();
                return Err(error);
            }
            room.selected
                .retain(|p| &p.talk != peer || p.stream != stream);
            Ok(())
        });
    }
    /// Snapshot of remote Talk sessions that typed recently in this chat route.
    pub(crate) fn typing_sessions(&self) -> Result<Vec<PeerSession>, ErrorKind> {
        self.with_room(|room, _| {
            if room.purpose != RoomPurpose::Chat {
                return Err(ErrorKind::WrongScope);
            }
            let now = std::time::Instant::now();
            room.typing
                .retain(|(_, at)| now.saturating_duration_since(*at) < Duration::from_secs(8));
            Ok(room
                .typing
                .iter()
                .map(|(session, _)| session.clone())
                .collect())
        })
    }
    /// Send only to currently observed room sessions. A late command cannot
    /// act after the route has been retired or replaced.
    pub(crate) async fn send_typing(
        &self,
        context: &OperationContext,
        started: bool,
    ) -> Result<(), ErrorKind> {
        self.handle.shared.validate(context)?;
        self.with_room(|room, _| {
            if room.purpose == RoomPurpose::Chat {
                Ok(())
            } else {
                Err(ErrorKind::WrongScope)
            }
        })?;
        let (reply, receive) = oneshot::channel();
        self.handle
            .commands
            .try_send(Command::Typing {
                generation: self.generation,
                started,
                deadline: context.deadline,
                cancel: context.cancellation.clone(),
                reply,
            })
            .map_err(|_| ErrorKind::Busy)?;
        until(
            &self.handle.shared,
            context.deadline,
            &context.cancellation,
            async { receive.await.map_err(|_| ErrorKind::Transport)? },
        )
        .await
    }
    #[cfg(test)]
    pub(crate) async fn request_offer(
        &self,
        context: &OperationContext,
        peer: &PeerSession,
    ) -> Result<(), ErrorKind> {
        self.request_stream_offer(context, peer, StreamKind::Camera)
            .await
    }
    pub(crate) fn reset_subscription(
        &self,
        peer: &PeerSession,
        stream: StreamKind,
    ) -> Result<(), ErrorKind> {
        self.with_room(|room, _| room.retire_subscription(peer, stream))
    }
    pub(crate) async fn request_stream_offer(
        &self,
        context: &OperationContext,
        peer: &PeerSession,
        stream: StreamKind,
    ) -> Result<(), ErrorKind> {
        self.with_room(|room, _| {
            if peer == &room.own {
                return Err(ErrorKind::WrongScope);
            }
            if room
                .selected
                .iter()
                .any(|p| &p.talk == peer && p.stream == stream)
            {
                return Err(ErrorKind::Conflict);
            }
            if room.selected.len() >= 20 {
                return Err(ErrorKind::Busy);
            }
            let member = room
                .members
                .iter()
                .find(|m| &m.talk == peer)
                .ok_or(ErrorKind::NotFound)?;
            room.selected.push(Selection {
                stream,
                talk: peer.clone(),
                sfu: member.sfu.clone(),
                sid: None,
                publisher: false,
                request_id: None,
            });
            Ok(())
        })?;
        self.send_stream(context, peer, None, None, stream).await
    }
    #[cfg(test)]
    pub(crate) async fn send(
        &self,
        context: &OperationContext,
        peer: &PeerSession,
        sid: Option<&Negotiation>,
        signal: Option<Signal>,
    ) -> Result<(), ErrorKind> {
        self.send_stream(context, peer, sid, signal, StreamKind::Camera)
            .await
    }
    pub(crate) async fn send_stream(
        &self,
        context: &OperationContext,
        peer: &PeerSession,
        sid: Option<&Negotiation>,
        signal: Option<Signal>,
        stream: StreamKind,
    ) -> Result<(), ErrorKind> {
        self.handle.shared.validate(context)?;
        if self.is_closed() {
            return Err(ErrorKind::Cancelled);
        }
        let (reply, receive) = oneshot::channel();
        // Dropped/expired mutation futures retire the call. A queued command may
        // never become a later automatic action after its caller has abandoned it.
        let guard = Abort(self.cancellation.clone(), false);
        self.handle
            .commands
            .try_send(Command::Send {
                generation: self.generation,
                stream,
                peer: peer.clone(),
                sid: sid.cloned(),
                signal,
                deadline: context.deadline,
                cancel: context.cancellation.clone(),
                reply,
            })
            .map_err(|_| ErrorKind::Busy)?;
        until(
            &self.handle.shared,
            context.deadline,
            &context.cancellation,
            async { receive.await.map_err(|_| ErrorKind::Transport)? },
        )
        .await?;
        if self.is_closed() {
            return Err(ErrorKind::Cancelled);
        }
        drop(guard.disarm());
        Ok(())
    }
    pub(crate) async fn send_force_mute(
        &self,
        context: &OperationContext,
        peer: &PeerSession,
        target: &PeerSession,
    ) -> Result<(), ErrorKind> {
        self.handle.shared.validate(context)?;
        self.with_room(|room, _| {
            if peer == &room.own
                || target == &room.own
                || !room.members.iter().any(|member| &member.talk == peer)
                || !room.members.iter().any(|member| &member.talk == target)
            {
                return Err(ErrorKind::WrongScope);
            }
            Ok(())
        })?;
        let (reply, receive) = oneshot::channel();
        self.handle
            .commands
            .try_send(Command::ForceMute {
                generation: self.generation,
                peer: peer.clone(),
                target: target.clone(),
                deadline: context.deadline,
                cancel: context.cancellation.clone(),
                reply,
            })
            .map_err(|_| ErrorKind::Busy)?;
        until(
            &self.handle.shared,
            context.deadline,
            &context.cancellation,
            async { receive.await.map_err(|_| ErrorKind::Transport)? },
        )
        .await?;
        if self.is_closed() {
            return Err(ErrorKind::Cancelled);
        }
        Ok(())
    }

    /// Activity is ephemeral and never grants media authority. An abandoned
    /// command is dropped by the socket owner instead of retiring the call.
    pub(crate) async fn send_activity(
        &self,
        context: &OperationContext,
        peer: &PeerSession,
        activity: crate::signaling::activity::Activity,
    ) -> Result<(), ErrorKind> {
        self.handle.shared.validate(context)?;
        activity.encode()?;
        self.with_room(|room, _| {
            if peer == &room.own || !room.members.iter().any(|member| &member.talk == peer) {
                return Err(ErrorKind::WrongScope);
            }
            Ok(())
        })?;
        let (reply, receive) = oneshot::channel();
        self.handle
            .commands
            .try_send(Command::Activity {
                generation: self.generation,
                peer: peer.clone(),
                activity,
                deadline: context.deadline,
                cancel: context.cancellation.clone(),
                reply,
            })
            .map_err(|_| ErrorKind::Busy)?;
        until(
            &self.handle.shared,
            context.deadline,
            &context.cancellation,
            async { receive.await.map_err(|_| ErrorKind::Transport)? },
        )
        .await?;
        if self.is_closed() {
            return Err(ErrorKind::Cancelled);
        }
        Ok(())
    }
    pub(crate) async fn poll(&self, context: &OperationContext) -> Result<SignalBatch, ErrorKind> {
        until(
            &self.handle.shared,
            context.deadline,
            &context.cancellation,
            async {
                for _ in 0..40 {
                    let events = self.with_room(|room, _| {
                        let events = room.signals.drain(..).map(|(_, e)| e).collect::<Vec<_>>();
                        room.signal_bytes = 0;
                        Ok(events)
                    })?;
                    if !events.is_empty() {
                        return Ok(SignalBatch { events });
                    }
                    tokio::time::sleep(Duration::from_millis(25)).await;
                }
                Ok(SignalBatch { events: Vec::new() })
            },
        )
        .await
    }
}
async fn until<T>(
    shared: &Shared,
    deadline: Moment,
    cancel: &Cancellation,
    future: impl Future<Output = Result<T, ErrorKind>>,
) -> Result<T, ErrorKind> {
    tokio::pin!(future);
    let mut tick = tokio::time::interval(Duration::from_millis(25));
    loop {
        if !shared.live() || cancel.is_cancelled() {
            return Err(ErrorKind::Cancelled);
        }
        let now = shared.account.runtime.now();
        if now.epoch != deadline.epoch || now.millis >= deadline.millis {
            return Err(ErrorKind::Deadline);
        }
        tokio::select! {
            biased;
            _ = tick.tick() => {},
            result = &mut future => {
                if !shared.live() || cancel.is_cancelled() { return Err(ErrorKind::Cancelled); }
                let now = shared.account.runtime.now();
                if now.epoch != deadline.epoch || now.millis >= deadline.millis { return Err(ErrorKind::Deadline); }
                return result;
            }
        }
    }
}

impl Client {
    pub fn cancel_prepared_external_meeting(&mut self) {
        if let Some(route) = self.hpb_route.take() {
            route.close();
        }
    }
    /// One bounded drain for the admitted account scheduler. Internal deployments
    /// explicitly retain HTTP fallback. This never joins a room or starts media.
    pub async fn poll_live_events(
        &mut self,
        context: &OperationContext,
    ) -> Result<LiveEventBatch, Error> {
        self.validate(context)?;
        let now = self.now();
        let refresh = self.live_settings_at.is_none_or(|at| {
            at.epoch != now.epoch || now.millis.saturating_sub(at.millis) >= 60_000
        });
        if refresh {
            let response = self
                .transport
                .ocs(
                    context,
                    reqwest::Method::GET,
                    "ocs/v2.php/apps/spreed/api/v3/signaling/settings",
                    &[],
                    &[],
                    false,
                )
                .await?;
            let settings = SignalingSettings::decode(&response.data).map_err(|e| self.error(e))?;
            self.configure_hpb(context, &settings)?;
            self.live_settings_at = Some(now);
            self.live_internal = settings.mode == SignalingMode::Internal;
        }
        if let Some(owner) = &self.hpb {
            return owner.poll(context);
        }
        Ok(LiveEventBatch {
            scope: context.scope,
            state: if self.live_internal {
                LiveState::Internal
            } else {
                LiveState::Stopped(ErrorKind::Unsupported)
            },
            first_transport_failure: None,
            events: Vec::new(),
        })
    }
    pub(crate) fn configure_hpb(
        &mut self,
        context: &OperationContext,
        settings: &SignalingSettings,
    ) -> Result<(), Error> {
        self.validate(context)?;
        if settings.mode != SignalingMode::External {
            self.cancel_prepared_external_meeting();
            self.hpb = None;
            return if settings.mode == SignalingMode::Internal {
                Ok(())
            } else {
                Err(self.error(ErrorKind::Unsupported))
            };
        }
        let external = settings
            .external
            .as_ref()
            .ok_or_else(|| self.error(ErrorKind::Malformed))?;
        if let Some(owner) = &self.hpb {
            owner.handle.update(external).map_err(|e| self.error(e))?;
        } else {
            let binding = self.transport.hpb_binding()?;
            self.hpb = Some(Owner::start(binding, external.clone()).map_err(|e| self.error(e))?);
        }
        Ok(())
    }
    /// Complete HPB room admission before the REST call join. The room session
    /// is the one returned by this same client's successful ActivateRoom.
    pub async fn prepare_external_meeting(
        &mut self,
        context: &OperationContext,
        room: &RoomToken,
        settings: &SignalingSettings,
    ) -> Result<(), Error> {
        if settings.mode != SignalingMode::External {
            return Err(self.error(ErrorKind::Unsupported));
        }
        let own = self.own_room_session(context, room)?;
        if self.hpb.as_ref().is_some_and(Owner::transport_stopped) {
            // Only fresh, validated room/call admission renews an exhausted
            // transport owner. Routine event polling cannot restore a stopped
            // session, and old route handles remain irreversibly closed.
            self.cancel_prepared_external_meeting();
            self.hpb = None;
        }
        self.configure_hpb(context, settings)?;
        let handle = self
            .hpb
            .as_ref()
            .ok_or_else(|| self.error(ErrorKind::Unsupported))?
            .handle();
        let route = handle
            .join(context, room.clone(), own)
            .await
            .map_err(|e| self.error(e))?;
        self.hpb_route = Some(route);
        Ok(())
    }
}

#[cfg(test)]
mod tests;
