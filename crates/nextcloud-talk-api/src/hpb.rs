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
    signaling::{Event, Negotiation, PeerSession, Signal, SignalBatch},
};
use std::{
    collections::VecDeque,
    future::Future,
    sync::{Arc, Mutex},
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
struct Room {
    token: RoomToken,
    own: PeerSession,
    generation: u64,
    cancellation: Cancellation,
    joined: bool,
    members: Vec<wire::Member>,
    selected: Vec<Selection>,
    retired: Vec<PeerSession>,
    signals: VecDeque<(usize, Event)>,
    signal_bytes: usize,
}
struct Selection {
    talk: PeerSession,
    sfu: PeerSession,
    sid: Option<Negotiation>,
    publisher: bool,
}
impl Room {
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
        if self.signals.len() == 84 || self.signal_bytes + bytes > 192 * 1024 {
            self.cancellation.cancel();
            return Err(ErrorKind::Busy);
        }
        self.signal_bytes += bytes;
        self.signals.push_back((bytes, event));
        Ok(())
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
    Send {
        generation: u64,
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
            Self::Join { reply, .. } | Self::Send { reply, .. } => {
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
        self.shared.validate(context)?;
        let cancellation = Cancellation::default();
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
                token,
                own,
                generation,
                cancellation: cancellation.clone(),
                joined: false,
                members: Vec::with_capacity(100),
                selected: Vec::with_capacity(17),
                retired: Vec::with_capacity(100),
                signals: VecDeque::new(),
                signal_bytes: 0,
            });
            generation
        };
        let route = Route {
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
    pub(crate) fn select(&self, peer: PeerSession, sid: Negotiation) -> Result<(), ErrorKind> {
        self.with_room(|room, own_sfu| {
            if room.retired.contains(&peer) {
                return Err(ErrorKind::WrongScope);
            }
            if let Some(selection) = room.selected.iter_mut().find(|p| p.talk == peer) {
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
            if room.selected.len() == 17 {
                return Err(ErrorKind::Busy);
            }
            room.selected.push(Selection {
                talk: peer,
                sfu: own_sfu.clone(),
                sid: Some(sid),
                publisher: true,
            });
            Ok(())
        })
    }
    pub(crate) fn remove(&self, peer: &PeerSession) {
        let _ = self.with_room(|room, _| {
            room.selected.retain(|p| &p.talk != peer);
            if room.retired.len() == 100 {
                room.cancellation.cancel();
                return Err(ErrorKind::Busy);
            }
            if !room.retired.contains(peer) {
                room.retired.push(peer.clone());
            }
            Ok(())
        });
    }
    pub(crate) async fn request_offer(
        &self,
        context: &OperationContext,
        peer: &PeerSession,
    ) -> Result<(), ErrorKind> {
        self.with_room(|room, _| {
            if peer == &room.own || room.retired.contains(peer) {
                return Err(ErrorKind::WrongScope);
            }
            if room.selected.iter().any(|p| &p.talk == peer) {
                return Err(ErrorKind::Conflict);
            }
            if room.selected.len() == 17 {
                return Err(ErrorKind::Busy);
            }
            let member = room
                .members
                .iter()
                .find(|m| &m.talk == peer)
                .ok_or(ErrorKind::NotFound)?;
            room.selected.push(Selection {
                talk: peer.clone(),
                sfu: member.sfu.clone(),
                sid: None,
                publisher: false,
            });
            Ok(())
        })?;
        self.send(context, peer, None, None).await
    }
    pub(crate) async fn send(
        &self,
        context: &OperationContext,
        peer: &PeerSession,
        sid: Option<&Negotiation>,
        signal: Option<Signal>,
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
        self.configure_hpb(context, settings)?;
        let own = self.own_room_session(context, room)?;
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
