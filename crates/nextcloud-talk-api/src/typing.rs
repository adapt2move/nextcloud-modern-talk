//! Account-owned ephemeral chat signaling. An isolated HTTP session keeps chat
//! membership separate from call membership; no room/session credentials escape
//! this module. Server and room policy are rechecked before admitting the watch.
mod internal;

use crate::{
    Client, Error, ErrorKind, Reply, Request, hpb,
    models::{Participant, RoomToken, SignalingMode},
    scope::{Cancellation, Moment, OperationContext, SessionScope},
    signaling::PeerSession,
};
use std::{
    sync::{Arc, Mutex},
    time::{Duration, Instant},
};

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Person {
    pub actor_type: String,
    pub actor_id: String,
    pub display_name: String,
}
struct Shared {
    scope: SessionScope,
    room: RoomToken,
    cancel: Cancellation,
    account_cancel: Cancellation,
    runtime: crate::NetworkRuntime,
    can_send: std::sync::atomic::AtomicBool,
    last_activity: Mutex<Option<Instant>>,
    people: Mutex<Vec<Person>>,
}
/// A single watch for one account/runtime/room. Dropping it retires that watch.
pub struct Watch {
    shared: Arc<Shared>,
}
impl Watch {
    pub fn matches(&self, scope: SessionScope, room: &RoomToken) -> bool {
        self.shared.scope == scope
            && &self.shared.room == room
            && !self.shared.cancel.is_cancelled()
    }
    /// The caller must have fresh permission and the user's typing setting.
    /// A new activity lease expires automatically even if a stop is lost.
    pub fn activity(&self, context: &OperationContext, active: bool) -> Result<(), ErrorKind> {
        if context.scope != self.shared.scope {
            return Err(ErrorKind::WrongScope);
        }
        if !self.matches(context.scope, &self.shared.room)
            || context.cancellation.is_cancelled()
            || self.shared.account_cancel.is_cancelled()
        {
            return Err(ErrorKind::Cancelled);
        }
        let now = self.shared.runtime.now();
        if context.deadline.epoch != now.epoch || context.deadline.millis <= now.millis {
            return Err(ErrorKind::Deadline);
        }
        if active
            && !self
                .shared
                .can_send
                .load(std::sync::atomic::Ordering::Acquire)
        {
            return Err(ErrorKind::PermissionDenied);
        }
        let mut current = self
            .shared
            .last_activity
            .lock()
            .map_err(|_| ErrorKind::Uncertain)?;
        *current = active.then(Instant::now);
        Ok(())
    }
    pub fn people(&self, scope: SessionScope, room: &RoomToken) -> Vec<Person> {
        if !self.matches(scope, room) || self.shared.account_cancel.is_cancelled() {
            return Vec::new();
        }
        self.shared
            .people
            .lock()
            .map(|people| people.clone())
            .unwrap_or_default()
    }
}
impl Drop for Watch {
    fn drop(&mut self) {
        if let Ok(mut activity) = self.shared.last_activity.lock() {
            *activity = None;
        }
        if let Ok(mut people) = self.shared.people.lock() {
            people.clear();
        }
        self.shared.cancel.cancel();
    }
}
impl Client {
    /// Start only after the application owner checks membership and privacy.
    /// This method schedules I/O and never waits for the server.
    pub fn watch_typing(
        &self,
        context: &OperationContext,
        room: RoomToken,
    ) -> Result<Watch, Error> {
        self.validate(context)?;
        let runtime = tokio::runtime::Handle::try_current()
            .map_err(|_| self.error(ErrorKind::Unconfigured))?;
        if self
            .typing_active
            .swap(true, std::sync::atomic::Ordering::AcqRel)
        {
            return Err(self.error(ErrorKind::Busy));
        }
        let lease = Lease(self.typing_active.clone());
        let fork = self.typing_client();
        let shared = Arc::new(Shared {
            scope: context.scope,
            room,
            cancel: Cancellation::default(),
            account_cancel: self.cancellation(),
            runtime: self.transport.typing_runtime(),
            can_send: std::sync::atomic::AtomicBool::new(false),
            last_activity: Mutex::new(None),
            people: Mutex::new(Vec::new()),
        });
        let task_shared = shared.clone();
        runtime.spawn(async move {
            let _lease = lease;
            run(fork, task_shared).await;
        });
        Ok(Watch { shared })
    }
    fn typing_client(&self) -> Client {
        Client {
            transport: self.transport.typing_fork(),
            expected_user: self.expected_user.clone(),
            owner: Arc::new(()),
            active_room: None,
            room_session: None,
            call_generation: None,
            call_remote_pending: false,
            next_call: 0,
            sent_descriptions: 0,
            sent_candidates: 0,
            rooms: Vec::new(),
            sends: Vec::new(),
            creation_uncertain: false,
            tag_creation_uncertain: false,
            schedule_uncertain: Vec::new(),
            poll_creation_uncertain: Vec::new(),
            directory_cursor: None,
            hpb: None,
            hpb_route: None,
            live_settings_at: None,
            live_internal: false,
            typing_active: self.typing_active.clone(),
        }
    }
}
struct Lease(Arc<std::sync::atomic::AtomicBool>);
impl Drop for Lease {
    fn drop(&mut self) {
        self.0.store(false, std::sync::atomic::Ordering::Release);
    }
}
fn context(client: &Client, shared: &Shared, timeout: u64, cleanup: bool) -> OperationContext {
    let now = client.now();
    OperationContext {
        scope: shared.scope,
        deadline: Moment {
            epoch: now.epoch,
            millis: now.millis.saturating_add(timeout),
        },
        cancellation: if cleanup {
            Cancellation::default()
        } else {
            shared.cancel.clone()
        },
    }
}
fn desired(shared: &Shared, now: Instant) -> bool {
    shared
        .last_activity
        .lock()
        .ok()
        .and_then(|value| *value)
        .is_some_and(|at| now.saturating_duration_since(at) < Duration::from_secs(4))
}
fn map_people(members: &[Participant], typing: &[PeerSession], own: &str) -> Vec<Person> {
    let mut result: Vec<Person> = Vec::new();
    for member in members.iter().take(100) {
        let (Some(kind), Some(id), Some(name), Some(sessions)) = (
            &member.actor_type,
            &member.actor_id,
            &member.display_name,
            &member.sessions,
        ) else {
            continue;
        };
        if (kind == "users" && id == own) || name.trim().is_empty() || id.is_empty() {
            continue;
        }
        let matched = sessions
            .iter()
            .filter_map(|session| PeerSession::parse(session).ok())
            .any(|session| typing.iter().any(|typing| typing.matches(&session)));
        if matched
            && !result
                .iter()
                .any(|person| person.actor_type == *kind && person.actor_id == *id)
        {
            result.push(Person {
                actor_type: kind.clone(),
                actor_id: id.clone(),
                display_name: name.clone(),
            });
        }
    }
    result
}
async fn run(mut client: Client, shared: Arc<Shared>) {
    let result = run_inner(&mut client, &shared).await;
    #[cfg(feature = "backend-test")]
    if let Err(error) = &result {
        eprintln!("typing lab watch ended: {:?}", error.kind);
    }
    let _ = result;
    shared.cancel.cancel();
    if let Ok(mut people) = shared.people.lock() {
        people.clear();
    }
    // A single bounded best-effort leave; an uncertain admission is never replayed.
    if client.active_room.as_ref() == Some(&shared.room) && !shared.account_cancel.is_cancelled() {
        let cleanup = context(&client, &shared, 1500, true);
        let _ = client
            .request(&cleanup, Request::DeactivateRoom(shared.room.clone()))
            .await;
    }
}
enum Link {
    External {
        owner: hpb::Owner,
        route: hpb::Route,
    },
    Internal(internal::Link),
}
impl Link {
    async fn send(
        &self,
        client: &mut Client,
        context: &OperationContext,
        active: bool,
    ) -> Result<(), Error> {
        match self {
            Self::External { route, .. } => route
                .send_typing(context, active)
                .await
                .map_err(|error| client.error(error)),
            Self::Internal(link) => link.send(client, context, active).await,
        }
    }
    fn sessions(&self) -> Vec<PeerSession> {
        match self {
            Self::External { route, .. } => route.typing_sessions().unwrap_or_default(),
            Self::Internal(link) => link.sessions(),
        }
    }
}
impl Drop for Link {
    fn drop(&mut self) {
        if let Self::External { owner, route } = self {
            route.close();
            let _ = owner;
        }
    }
}
fn room_policy(room: &crate::models::Room) -> Option<bool> {
    let attributes = room.attributes?;
    let permissions = room.permissions?;
    if room.remote
        || attributes & !31 != 0
        || attributes & 12 != 0
        || !matches!(room.kind, Some(1..=3))
        || !matches!(room.read_only, Some(0 | 1))
        || !matches!(room.lobby_state, Some(0 | 1))
        || room.lobby_state == Some(1) && permissions & 8 == 0
    {
        return None;
    }
    Some(room.read_only == Some(0) && permissions & 128 != 0)
}
async fn refresh_room_policy(client: &mut Client, shared: &Shared) -> Result<(), Error> {
    shared
        .can_send
        .store(false, std::sync::atomic::Ordering::Release);
    let ctx = context(client, shared, 5000, false);
    let Reply::Room(room) = client
        .request(&ctx, Request::Room(shared.room.clone()))
        .await?
    else {
        return Err(client.error(ErrorKind::Malformed));
    };
    let Some(can_send) = room_policy(&room) else {
        return Err(client.error(ErrorKind::PermissionDenied));
    };
    if !can_send && let Ok(mut activity) = shared.last_activity.lock() {
        *activity = None;
    }
    shared
        .can_send
        .store(can_send, std::sync::atomic::Ordering::Release);
    Ok(())
}
async fn run_inner(client: &mut Client, shared: &Arc<Shared>) -> Result<(), Error> {
    let ctx = context(client, shared, 5000, false);
    let Reply::Capabilities(caps) = client.request(&ctx, Request::Capabilities).await? else {
        return Err(client.error(ErrorKind::Malformed));
    };
    if !caps.supported_protocol()
        || !caps.supports("typing-privacy")
        || caps.typing_privacy != Some(0)
    {
        return Err(client.error(ErrorKind::Unsupported));
    }
    refresh_room_policy(client, shared).await?;
    let ctx = context(client, shared, 5000, false);
    let Reply::SignalingSettings(settings) = client
        .request(&ctx, Request::SignalingSettings(shared.room.clone()))
        .await?
    else {
        return Err(client.error(ErrorKind::Malformed));
    };
    if settings.mode == SignalingMode::Unknown {
        return Err(client.error(ErrorKind::Unsupported));
    }
    let ctx = context(client, shared, 5000, false);
    let Reply::RoomSession(session) = client
        .request(&ctx, Request::ActivateRoom(shared.room.clone()))
        .await?
    else {
        return Err(client.error(ErrorKind::Malformed));
    };
    let own = PeerSession::parse(&session.into_secret()).map_err(|error| client.error(error))?;
    let link = match settings.mode {
        SignalingMode::External => {
            let external = settings
                .external
                .ok_or_else(|| client.error(ErrorKind::Malformed))?;
            let owner = hpb::Owner::start(client.transport.hpb_binding()?, external)
                .map_err(|error| client.error(error))?;
            let ctx = context(client, shared, 5000, false);
            let route = owner
                .handle()
                .join_chat(&ctx, shared.room.clone(), own)
                .await
                .map_err(|error| client.error(error))?;
            Link::External { owner, route }
        }
        SignalingMode::Internal => Link::Internal(internal::Link::start(client, shared, own)),
        SignalingMode::Unknown => return Err(client.error(ErrorKind::Unsupported)),
    };
    let mut members = Vec::new();
    let mut next_members = Instant::now();
    let mut next_privacy = Instant::now() + Duration::from_secs(15);
    let mut next_policy = Instant::now() + Duration::from_secs(10);
    let mut last_sent = None;
    let mut sent = false;
    // Even a cancelled or failed policy refresh must go through the best-effort
    // stop below before the chat session is retired.
    let result: Result<(), Error> = async {
        loop {
            if shared.cancel.is_cancelled() || shared.account_cancel.is_cancelled() {
                break;
            }
            let now = Instant::now();
            if now >= next_privacy {
                let ctx = context(client, shared, 5000, false);
                let Reply::Capabilities(caps) = client.request(&ctx, Request::Capabilities).await?
                else {
                    break;
                };
                if !caps.supported_protocol()
                    || !caps.supports("typing-privacy")
                    || caps.typing_privacy != Some(0)
                {
                    break;
                }
                next_privacy = Instant::now() + Duration::from_secs(15);
            }
            if now >= next_policy {
                refresh_room_policy(client, shared).await?;
                next_policy = Instant::now() + Duration::from_secs(10);
            }
            if now >= next_members {
                let ctx = context(client, shared, 5000, false);
                if let Ok(Reply::Participants(value)) = client
                    .request(&ctx, Request::Participants(shared.room.clone()))
                    .await
                {
                    members = value;
                }
                next_members = Instant::now() + Duration::from_secs(10);
            }
            let active =
                shared.can_send.load(std::sync::atomic::Ordering::Acquire) && desired(shared, now);
            if active != sent
                || (active
                    && last_sent.is_none_or(|last: Instant| {
                        now.saturating_duration_since(last) >= Duration::from_secs(2)
                    }))
            {
                let ctx = context(client, shared, 2000, false);
                if link.send(client, &ctx, active).await.is_ok() {
                    sent = active;
                    last_sent = Some(now);
                }
            }
            let sessions = link.sessions();
            let people = map_people(&members, &sessions, client.transport.user());
            if let Ok(mut current) = shared.people.lock() {
                *current = people;
            }
            tokio::time::sleep(Duration::from_millis(250)).await;
        }
        Ok(())
    }
    .await;
    if sent && !shared.account_cancel.is_cancelled() {
        let ctx = context(client, shared, 1000, true);
        let _ = link.send(client, &ctx, false).await;
    }
    drop(link);
    result
}
