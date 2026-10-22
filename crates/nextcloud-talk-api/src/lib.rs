//! Origin-bound Nextcloud Talk protocol owner. No application/UI, store or media engine.
//! Calls are serialized per client. Dropped or failed writes are never replayed; callers
//! must reconcile uncertainty. `backend-test` adds finite synthetic signaling lanes only.
#![forbid(unsafe_code)]
#![doc = include_str!("../README.md")]
pub mod chat;
pub mod content;
pub mod conversation;
#[cfg(any(test, feature = "backend-test"))]
pub mod diagnostics;
mod error;
pub mod events;
pub mod files;
pub mod hpb;
mod json;
pub mod meeting;
pub mod models;
pub mod notifications;
pub mod poll;
pub mod presence;
pub mod scope;
pub mod signaling;
mod transport;
pub use error::{ErrorKind, PortError as Error, Retry};
use models::*;
use reqwest::Method;
use scope::*;
pub use transport::{NetworkRuntime, ServerOrigin};
use zeroize::Zeroizing;
/// Maximum complete HTTP response body before JSON decoding.
pub const MAX_RESPONSE_BYTES: usize = 256 * 1024;

/// Consumed once by the authenticated owner. Never Debug/Clone, and no secret getters.
/// Only app passwords obtained/retained by the application's identity owner belong here.
/// ```compile_fail
/// use nextcloud_talk_api::AppCredential;
/// fn cannot_copy(secret: &AppCredential) -> AppCredential { secret.clone() }
/// ```
/// ```compile_fail
/// use nextcloud_talk_api::AppCredential;
/// fn cannot_log(secret: &AppCredential) { println!("{secret:?}"); }
/// ```
pub struct AppCredential {
    origin: ServerOrigin,
    login_name: Zeroizing<String>,
    password: Zeroizing<String>,
    expected_user: Option<Zeroizing<String>>,
}
impl AppCredential {
    pub fn new(
        origin: ServerOrigin,
        login_name: Zeroizing<String>,
        app_password: Zeroizing<String>,
        expected_user: Option<Zeroizing<String>>,
    ) -> Self {
        Self {
            origin,
            login_name,
            password: app_password,
            expected_user,
        }
    }
}

/// Semantic operations only. There is no authenticated URL, method, header or form API.
/// Readback/reconciliation policy and application permission decisions belong to the caller.
pub enum Request {
    Chat {
        room: RoomToken,
        command: chat::Command,
    },
    Conversation {
        room: RoomToken,
        command: conversation::Command,
    },
    Account,
    Capabilities,
    Rooms,
    Room(RoomToken),
    Participants(RoomToken),
    History {
        room: RoomToken,
        before: Option<u64>,
    },
    SendMessage {
        room: RoomToken,
        message: String,
        reference: String,
    },
    MarkRead {
        room: RoomToken,
        message: u64,
    },
    CreateGroup {
        name: String,
    },
    /// Server-filtered local users available when starting a conversation.
    FindPeople {
        search: String,
    },
    CreateDirect {
        user: String,
    },
    LeaveRoom(RoomToken),
    ActivateRoom(RoomToken),
    DeactivateRoom(RoomToken),
    SetArchived {
        room: RoomToken,
        archived: bool,
    },
    SetFavorite {
        room: RoomToken,
        favorite: bool,
    },
    SetPreference(Preference),
    Tags,
    CreateTag {
        name: String,
    },
    RenameTag {
        tag: TagId,
        name: String,
    },
    DeleteTag(TagId),
    CollapseTag {
        tag: TagId,
        collapsed: bool,
    },
    ReorderTags(Vec<TagId>),
    SetRoomTags {
        room: RoomToken,
        tags: Vec<TagId>,
    },
    CallParticipants(RoomToken),
    SignalingSettings(RoomToken),
    JoinCall {
        room: RoomToken,
        flags: ParticipantFlags,
        silent: bool,
        recording_consent: bool,
    },
    UpdateCallFlags {
        room: RoomToken,
        flags: ParticipantFlags,
    },
    LeaveCall(RoomToken),
}
pub enum Reply {
    Chat(chat::Response),
    Conversation(conversation::Response),
    Account(Account),
    Capabilities(Capabilities),
    Rooms(Vec<Room>),
    Room(Room),
    Participants(Vec<Participant>),
    History {
        messages: Vec<Message>,
        last_given: Option<u64>,
        common_read: Option<u64>,
    },
    Message(Message),
    RoomSession(RoomSession),
    Tags(Vec<Tag>),
    Tag(Tag),
    CallParticipants(Vec<CallParticipant>),
    SignalingSettings(SignalingSettings),
    CallJoined(signaling::CallHandle),
    Acknowledged,
}

/// One HTTP/cookie owner per admitted network lease; the runtime caps accounts and I/O.
pub struct Client {
    transport: transport::Transport,
    expected_user: Option<Zeroizing<String>>,
    owner: std::sync::Arc<()>,
    active_room: Option<RoomToken>,
    room_session: Option<Zeroizing<String>>,
    call_generation: Option<u64>,
    call_remote_pending: bool,
    next_call: u64,
    sent_descriptions: u8,
    sent_candidates: u8,
    rooms: Vec<(RoomToken, u64)>,
    sends: Vec<SendAttempt>,
    creation_uncertain: bool,
    tag_creation_uncertain: bool,
    schedule_uncertain: Vec<RoomToken>,
    poll_creation_uncertain: Vec<RoomToken>,
    hpb: Option<hpb::Owner>,
    hpb_route: Option<hpb::Route>,
    live_settings_at: Option<scope::Moment>,
    live_internal: bool,
}
struct SendAttempt {
    room: RoomToken,
    reference: String,
    uncertain: bool,
}

impl Client {
    pub fn new(
        credential: AppCredential,
        runtime: NetworkRuntime,
        scope: SessionScope,
        host: OriginId,
        cancellation: Cancellation,
    ) -> Result<Self, Error> {
        let provisional = credential
            .expected_user
            .as_ref()
            .unwrap_or(&credential.login_name)
            .clone();
        Ok(Self {
            transport: transport::Transport::new(
                transport::AuthenticatedBinding {
                    scope,
                    host,
                    origin: credential.origin,
                    login_name: credential.login_name,
                    user: provisional,
                    password: credential.password,
                    cancellation,
                },
                runtime,
            )?,
            expected_user: credential.expected_user,
            owner: std::sync::Arc::new(()),
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
            hpb: None,
            hpb_route: None,
            live_settings_at: None,
            live_internal: false,
        })
    }
    pub fn validate(&self, context: &OperationContext) -> Result<(), Error> {
        self.transport.validate(context)
    }
    /// Constructs a noncredential guest link; callers must first confirm public-room policy.
    pub fn public_room_link(&self, room: &RoomToken) -> Result<String, Error> {
        self.transport.public_room_link(room)
    }
    pub fn now(&self) -> Moment {
        self.transport.now()
    }
    pub fn cancellation(&self) -> Cancellation {
        self.transport.cancellation()
    }
    fn error(&self, kind: ErrorKind) -> Error {
        self.transport.error(kind)
    }
    fn mutation_error(&self, error: Error) -> Error {
        if !self.transport.request_dispatched()
            || matches!(
                error.kind,
                ErrorKind::PermissionDenied
                    | ErrorKind::ServerAccessDenied
                    | ErrorKind::NotFound
                    | ErrorKind::Conflict
                    | ErrorKind::RateLimited
            )
        {
            error
        } else {
            self.error(ErrorKind::Uncertain)
        }
    }
    fn observe_rooms(&mut self, reply: &Reply) -> Result<(), Error> {
        let observed = match reply {
            Reply::Rooms(rooms) => rooms.as_slice(),
            Reply::Room(room) => std::slice::from_ref(room),
            _ => return Ok(()),
        };
        let mut tokens = Vec::with_capacity(observed.len());
        let mut additions = Vec::new();
        for room in observed {
            if tokens.contains(&room.token.as_str()) {
                return Err(self.error(ErrorKind::Malformed));
            }
            tokens.push(room.token.as_str());
            if let Some(id) = room.server_id {
                let token = RoomToken::parse(&room.token).map_err(|k| self.error(k))?;
                if let Some((_, old)) = self.rooms.iter().find(|(t, _)| t == &token) {
                    if *old != id {
                        return Err(self.error(ErrorKind::WrongScope));
                    }
                } else {
                    additions.push((token, id));
                }
            }
        }
        if self.rooms.len() + additions.len() > 256 {
            return Err(self.error(ErrorKind::Busy));
        }
        self.rooms.extend(additions);
        Ok(())
    }
    pub async fn request(
        &mut self,
        context: &OperationContext,
        request: Request,
    ) -> Result<Reply, Error> {
        self.validate(context)?;
        if self.transport.meeting_active()
            && matches!(
                request,
                Request::ActivateRoom(_) | Request::DeactivateRoom(_) | Request::LeaveRoom(_)
            )
        {
            return Err(self.error(ErrorKind::Busy));
        }
        let create_room = matches!(
            request,
            Request::CreateGroup { .. } | Request::CreateDirect { .. }
        );
        if let Request::CreateDirect { user } = &request
            && user == self.transport.user()
        {
            return Err(self.error(ErrorKind::PermissionDenied));
        }
        let create_tag = matches!(request, Request::CreateTag { .. });
        let (method, route, query, form, kind, mutation) =
            plan(&request).map_err(|k| self.error(k))?;
        // Journals precede the first side-effecting await. Dropping this task keeps
        // uncertainty; a second call cannot replay a one-time write or its reference.
        let send_reference = match &request {
            Request::SendMessage {
                room, reference, ..
            }
            | Request::Chat {
                room,
                command: chat::Command::Send { reference, .. },
            }
            | Request::Chat {
                room,
                command: chat::Command::ShareObject { reference, .. },
            }
            | Request::Chat {
                room,
                command: chat::Command::ShareAttachment { reference, .. },
            } => Some((room, reference)),
            _ => None,
        };
        let send_index = if let Some((room, reference)) = send_reference {
            if let Some(old) = self.sends.iter().find(|s| s.reference == *reference) {
                return Err(self.error(if old.room != *room {
                    ErrorKind::WrongScope
                } else if old.uncertain {
                    ErrorKind::Uncertain
                } else {
                    ErrorKind::Conflict
                }));
            }
            // Keep unresolved writes until explicit reconciliation; settled entries
            // have a finite recent-replay window. References are caller-unique, never
            // server idempotency keys. Ordinary long-lived usage must not saturate.
            if self.sends.len() >= 100 {
                if let Some(index) = self.sends.iter().position(|s| !s.uncertain) {
                    self.sends.remove(index);
                } else {
                    return Err(self.error(ErrorKind::Busy));
                }
            }
            self.sends.push(SendAttempt {
                room: room.clone(),
                reference: reference.clone(),
                uncertain: true,
            });
            Some(self.sends.len() - 1)
        } else {
            None
        };
        if let Request::Chat {
            room,
            command: chat::Command::Schedule { .. },
        } = &request
        {
            if self.schedule_uncertain.contains(room) {
                return Err(self.error(ErrorKind::Uncertain));
            }
            if self.schedule_uncertain.len() >= 100 {
                return Err(self.error(ErrorKind::Busy));
            }
            self.schedule_uncertain.push(room.clone());
        }
        if create_room {
            if self.creation_uncertain {
                return Err(self.error(ErrorKind::Uncertain));
            }
            self.creation_uncertain = true;
        }
        if matches!(request, Request::CreateTag { .. }) {
            if self.tag_creation_uncertain {
                return Err(self.error(ErrorKind::Uncertain));
            }
            self.tag_creation_uncertain = true;
        }
        if let Request::JoinCall { room, .. }
        | Request::UpdateCallFlags { room, .. }
        | Request::LeaveCall(room)
        | Request::DeactivateRoom(room) = &request
            && self.active_room.as_ref() != Some(room)
        {
            return Err(self.error(ErrorKind::WrongScope));
        }
        if let Request::ActivateRoom(room) = &request
            && self
                .active_room
                .as_ref()
                .is_some_and(|active| active != room)
        {
            return Err(self.error(ErrorKind::Busy));
        }
        if matches!(request, Request::JoinCall { .. }) {
            if self.room_session.is_none() {
                return Err(self.error(ErrorKind::WrongScope));
            }
            if self.call_remote_pending {
                return Err(self.error(if self.call_generation.is_some() {
                    ErrorKind::Conflict
                } else {
                    ErrorKind::Uncertain
                }));
            }
            self.next_call = self
                .next_call
                .checked_add(1)
                .ok_or_else(|| self.error(ErrorKind::Busy))?;
            self.call_remote_pending = true;
        }
        if matches!(request, Request::UpdateCallFlags { .. }) && self.call_generation.is_none() {
            return Err(self.error(ErrorKind::WrongScope));
        }
        // A session replacement or uncertain flag update must not retain a handle
        // from the previous exchange. Keep the room tombstone until confirmed
        // cleanup so an explicit retry can still close a dropped leave.
        let updating_call = if matches!(request, Request::UpdateCallFlags { .. }) {
            self.call_generation.take()
        } else {
            None
        };
        let leaving_active_room = matches!(
            &request,
            Request::DeactivateRoom(room) | Request::LeaveRoom(room)
                if self.active_room.as_ref() == Some(room)
        );
        if matches!(request, Request::LeaveCall(_)) || leaving_active_room {
            self.call_generation = None;
            self.cancel_prepared_external_meeting();
        }
        if leaving_active_room {
            self.room_session = None;
        }
        let previous_active = if matches!(request, Request::ActivateRoom(_)) {
            Some((
                self.active_room.clone(),
                self.call_generation,
                self.room_session.clone(),
            ))
        } else {
            None
        };
        if let Request::ActivateRoom(room) = &request {
            self.active_room = Some(room.clone());
            self.call_generation = None;
            self.room_session = None;
        }
        let response = if matches!(
            request,
            Request::History { .. }
                | Request::Chat {
                    command: chat::Command::History { .. } | chat::Command::Poll { .. },
                    ..
                }
        ) {
            self.transport.history(context, &route, &query).await
        } else {
            self.transport
                .ocs(
                    context,
                    method,
                    &route,
                    &query,
                    &form,
                    matches!(request, Request::Account),
                )
                .await
        };
        let response = match response {
            Ok(response) => response,
            Err(error) => {
                let error = if mutation {
                    self.mutation_error(error)
                } else {
                    error
                };
                if error.kind != ErrorKind::Uncertain {
                    if let Some((active_room, call_generation, room_session)) = previous_active {
                        self.active_room = active_room;
                        self.call_generation = call_generation;
                        self.room_session = room_session;
                    }
                    if matches!(request, Request::JoinCall { .. }) {
                        self.call_remote_pending = false;
                    }
                    if let Request::Chat {
                        room,
                        command: chat::Command::Schedule { .. },
                    } = &request
                    {
                        self.schedule_uncertain.retain(|r| r != room);
                    }
                    if let Some(index) = send_index {
                        self.sends[index].uncertain = false;
                    }
                    if create_room {
                        self.creation_uncertain = false;
                    }
                    if matches!(request, Request::CreateTag { .. }) {
                        self.tag_creation_uncertain = false;
                    }
                }
                return Err(error);
            }
        };
        let mut decoded = decode(kind, response).map_err(|k| {
            if mutation {
                self.error(ErrorKind::Uncertain)
            } else {
                self.error(k)
            }
        })?;
        // Typed decoding alone does not correlate a server response to this request.
        match (&request, &decoded) {
            (Request::Room(expected), Reply::Room(room)) if room.token != expected.as_str() => {
                return Err(self.error(ErrorKind::WrongScope));
            }
            (
                Request::History { room, .. } | Request::Chat { room, .. },
                Reply::History { messages, .. },
            ) => {
                let mut seen = Vec::with_capacity(messages.len());
                for message in messages {
                    if message.token != room.as_str() {
                        return Err(self.error(ErrorKind::WrongScope));
                    }
                    if seen.contains(&message.id) {
                        return Err(self.error(ErrorKind::Malformed));
                    }
                    seen.push(message.id);
                }
            }
            (_, Reply::Message(message)) if send_index.is_some() => {
                let attempt = &self.sends[send_index.expect("admitted send")];
                if message.token != attempt.room.as_str()
                    || message.reference.as_ref() != Some(&attempt.reference)
                    || message.actor_type.as_deref() != Some("users")
                    || message.actor_id.as_deref() != Some(self.transport.user())
                {
                    return Err(self.error(ErrorKind::Uncertain));
                }
            }
            (
                Request::Chat {
                    command:
                        chat::Command::Edit {
                            message: expected, ..
                        }
                        | chat::Command::Delete(expected),
                    ..
                },
                Reply::Message(message),
            ) if message.update.as_ref().map_or(message.id, |m| m.id) != *expected => {
                return Err(self.error(ErrorKind::Uncertain));
            }
            (Request::Conversation { room, .. }, Reply::Message(message))
                if message.token != room.as_str() =>
            {
                return Err(self.error(ErrorKind::Uncertain));
            }
            (Request::Chat { room, .. }, Reply::Message(message))
                if message.token != room.as_str() =>
            {
                return Err(self.error(if mutation {
                    ErrorKind::Uncertain
                } else {
                    ErrorKind::WrongScope
                }));
            }
            (Request::Conversation { room: expected, .. }, Reply::Room(room))
                if room.token != expected.as_str() =>
            {
                return Err(self.error(if mutation {
                    ErrorKind::Uncertain
                } else {
                    ErrorKind::WrongScope
                }));
            }
            (Request::Chat { room, .. }, Reply::Chat(reply)) => chat::validate(room, reply)
                .map_err(|k| self.error(if mutation { ErrorKind::Uncertain } else { k }))?,
            _ => {}
        }
        if let Reply::Account(ref account) = decoded
            && self
                .expected_user
                .as_ref()
                .is_some_and(|expected| expected.as_str() != account.id)
        {
            return Err(self.error(ErrorKind::WrongScope));
        }
        self.validate(context)
            .map_err(|e| if mutation { self.mutation_error(e) } else { e })?;
        if let Reply::Account(ref account) = decoded {
            self.transport
                .bind_canonical_user(Zeroizing::new(account.id.clone()));
            self.expected_user = Some(Zeroizing::new(account.id.clone()));
        }
        self.observe_rooms(&decoded)
            .map_err(|e| if mutation { self.mutation_error(e) } else { e })?;
        if create_room {
            self.creation_uncertain = false;
        }
        if create_tag {
            self.tag_creation_uncertain = false;
        }
        if matches!(request, Request::LeaveCall(_)) || leaving_active_room {
            self.call_remote_pending = false;
        }
        if leaving_active_room {
            self.active_room = None;
        }
        if let Some(generation) = updating_call {
            self.call_generation = Some(generation);
        }
        if let Request::ActivateRoom(room) = &request {
            self.active_room = Some(room.clone());
            if let Reply::RoomSession(session) = &decoded {
                self.room_session = Some(session.0.clone());
            }
        }
        if let Request::Chat {
            room,
            command: chat::Command::Schedule { .. } | chat::Command::Scheduled,
        } = &request
        {
            self.schedule_uncertain.retain(|r| r != room);
        }
        if let Request::JoinCall { room, .. } = request {
            self.call_generation = Some(self.next_call);
            self.sent_descriptions = 0;
            self.sent_candidates = 0;
            let room_id = self
                .rooms
                .iter()
                .find(|(token, _)| token == &room)
                .map(|(_, id)| *id);
            decoded = Reply::CallJoined(signaling::CallHandle {
                room,
                room_id,
                scope: context.scope,
                origin: self.transport.host(),
                owner: self.owner.clone(),
                generation: self.next_call,
            });
        }
        if let Some(index) = send_index {
            self.sends[index].uncertain = false;
        }
        match &decoded {
            Reply::Rooms(_) => self.creation_uncertain = false,
            Reply::Tags(_) => self.tag_creation_uncertain = false,
            Reply::History { messages, .. } => {
                for message in messages {
                    if message.actor_type.as_deref() == Some("users")
                        && message.actor_id.as_deref() == Some(self.transport.user())
                    {
                        for send in &mut self.sends {
                            if message.reference.as_ref() == Some(&send.reference)
                                && message.token == send.room.as_str()
                            {
                                send.uncertain = false;
                            }
                        }
                    }
                }
            }
            _ => {}
        }
        Ok(decoded)
    }
}
#[derive(Clone, Copy)]
enum Kind {
    Chat(chat::ResponseKind),
    Conversation(conversation::ResponseKind),
    Account,
    Capabilities,
    Rooms,
    Room,
    Participants,
    History,
    Message,
    OptionalMessage,
    Session,
    Tags,
    Tag,
    CallParticipants,
    Settings,
    Ack,
}
type Plan = (
    Method,
    String,
    Vec<(&'static str, String)>,
    Vec<(&'static str, String)>,
    Kind,
    bool,
);
fn bounded(s: &str, max: usize) -> Result<(), ErrorKind> {
    if s.len() > max {
        Err(ErrorKind::TooLarge)
    } else {
        Ok(())
    }
}
fn plan(request: &Request) -> Result<Plan, ErrorKind> {
    let room = |version: &str, token: &RoomToken, suffix: &str| {
        format!(
            "ocs/v2.php/apps/spreed/api/{version}/{}{suffix}",
            token.as_str()
        )
    };
    let tag = |id: &TagId, suffix: &str| {
        format!("ocs/v2.php/apps/spreed/api/v4/tags/{}{suffix}", id.as_str())
    };
    let (method, route, query, form, kind) = match request {
        Request::Chat { room, command } => return chat::plan(room, command),
        Request::Conversation { room, command } => return conversation::plan(room, command),
        Request::Account => (
            Method::GET,
            "ocs/v2.php/cloud/user".into(),
            vec![],
            vec![],
            Kind::Account,
        ),
        Request::Capabilities => (
            Method::GET,
            "ocs/v2.php/cloud/capabilities".into(),
            vec![],
            vec![],
            Kind::Capabilities,
        ),
        Request::Rooms => (
            Method::GET,
            "ocs/v2.php/apps/spreed/api/v4/room".into(),
            vec![("noStatusUpdate", "1".into())],
            vec![],
            Kind::Rooms,
        ),
        Request::Room(t) => (
            Method::GET,
            room("v4/room", t, ""),
            vec![],
            vec![],
            Kind::Room,
        ),
        Request::Participants(t) => (
            Method::GET,
            room("v4/room", t, "/participants"),
            vec![],
            vec![],
            Kind::Participants,
        ),
        Request::History { room: t, before } => {
            let mut q = vec![
                ("lookIntoFuture", "0".into()),
                ("limit", "50".into()),
                ("setReadMarker", "0".into()),
                ("markNotificationsAsRead", "0".into()),
                ("noStatusUpdate", "1".into()),
            ];
            if let Some(id) = before {
                if *id == 0 {
                    return Err(ErrorKind::Malformed);
                }
                q.push(("lastKnownMessageId", id.to_string()));
            }
            (
                Method::GET,
                room("v1/chat", t, ""),
                q,
                vec![],
                Kind::History,
            )
        }
        Request::SendMessage {
            room: t,
            message,
            reference,
        } => {
            bounded(message, 131072)?;
            bounded(reference, 256)?;
            if message.trim().is_empty() || reference.is_empty() {
                return Err(ErrorKind::Malformed);
            }
            (
                Method::POST,
                room("v1/chat", t, ""),
                vec![],
                vec![
                    ("message", message.clone()),
                    ("referenceId", reference.clone()),
                ],
                Kind::Message,
            )
        }
        Request::MarkRead { room: t, message } => {
            if *message == 0 {
                return Err(ErrorKind::Malformed);
            }
            (
                Method::POST,
                room("v1/chat", t, "/read"),
                vec![],
                vec![("lastReadMessage", message.to_string())],
                Kind::Ack,
            )
        }
        Request::CreateGroup { name } => {
            bounded(name, 256)?;
            if name.trim().is_empty() {
                return Err(ErrorKind::Malformed);
            }
            (
                Method::POST,
                "ocs/v2.php/apps/spreed/api/v4/room".into(),
                vec![],
                vec![("roomType", "2".into()), ("roomName", name.clone())],
                Kind::Room,
            )
        }
        Request::FindPeople { search } => {
            bounded(search, 256)?;
            (
                Method::GET,
                "ocs/v2.php/core/autocomplete/get".into(),
                vec![
                    ("search", search.clone()),
                    ("itemType", "call".into()),
                    ("itemId", "new".into()),
                    ("limit", "100".into()),
                    ("shareTypes[]", "0".into()),
                ],
                vec![],
                Kind::Conversation(conversation::ResponseKind::Directory),
            )
        }
        Request::CreateDirect { user } => {
            bounded(user, 512)?;
            if user.trim().is_empty() || user.chars().any(char::is_control) {
                return Err(ErrorKind::Malformed);
            }
            (
                Method::POST,
                "ocs/v2.php/apps/spreed/api/v4/room".into(),
                vec![],
                vec![("roomType", "1".into()), ("invite", user.clone())],
                Kind::Room,
            )
        }
        Request::LeaveRoom(t) => (
            Method::DELETE,
            room("v4/room", t, "/participants/self"),
            vec![],
            vec![],
            Kind::Ack,
        ),
        Request::ActivateRoom(t) => (
            Method::POST,
            room("v4/room", t, "/participants/active"),
            vec![],
            vec![("force", "false".into())],
            Kind::Session,
        ),
        Request::DeactivateRoom(t) => (
            Method::DELETE,
            room("v4/room", t, "/participants/active"),
            vec![],
            vec![],
            Kind::Ack,
        ),
        Request::SetArchived { room: t, archived } => (
            if *archived {
                Method::POST
            } else {
                Method::DELETE
            },
            room("v4/room", t, "/archive"),
            vec![],
            vec![],
            Kind::Ack,
        ),
        Request::SetFavorite { room: t, favorite } => (
            if *favorite {
                Method::POST
            } else {
                Method::DELETE
            },
            room("v4/room", t, "/favorite"),
            vec![],
            vec![],
            Kind::Ack,
        ),
        Request::SetPreference(p) => {
            let (key, value) = match p {
                Preference::ReadReceipts(v) => {
                    ("read_status_privacy", if *v { "0" } else { "1" }.into())
                }
                Preference::TypingStatus(v) => {
                    ("typing_privacy", if *v { "0" } else { "1" }.into())
                }
                Preference::AttachmentFolder(v) => {
                    bounded(v, 16384)?;
                    ("attachment_folder", v.clone())
                }
            };
            (
                Method::POST,
                "ocs/v2.php/apps/spreed/api/v1/settings/user".into(),
                vec![],
                vec![("key", key.into()), ("value", value)],
                Kind::Ack,
            )
        }
        Request::Tags => (
            Method::GET,
            "ocs/v2.php/apps/spreed/api/v4/tags".into(),
            vec![],
            vec![],
            Kind::Tags,
        ),
        Request::CreateTag { name } => {
            bounded(name, 1000)?;
            (
                Method::POST,
                "ocs/v2.php/apps/spreed/api/v4/tags".into(),
                vec![],
                vec![("name", name.clone())],
                Kind::Tag,
            )
        }
        Request::RenameTag { tag: t, name } => {
            bounded(name, 1000)?;
            (
                Method::PUT,
                tag(t, ""),
                vec![],
                vec![("name", name.clone())],
                Kind::Ack,
            )
        }
        Request::DeleteTag(t) => (Method::DELETE, tag(t, ""), vec![], vec![], Kind::Ack),
        Request::CollapseTag { tag: t, collapsed } => (
            Method::PUT,
            tag(t, "/collapsed"),
            vec![],
            vec![("collapsed", collapsed.to_string())],
            Kind::Ack,
        ),
        Request::ReorderTags(tags) => {
            if tags.len() > 102 {
                return Err(ErrorKind::TooLarge);
            }
            (
                Method::PUT,
                "ocs/v2.php/apps/spreed/api/v4/tags/reorder".into(),
                vec![],
                tags.iter()
                    .map(|t| ("orderedIds[]", t.as_str().into()))
                    .collect(),
                Kind::Ack,
            )
        }
        Request::SetRoomTags { room: t, tags } => {
            if tags.len() > 20 {
                return Err(ErrorKind::TooLarge);
            }
            (
                Method::POST,
                room("v4/room", t, "/tags"),
                vec![],
                tags.iter()
                    .map(|t| ("tagIds[]", t.as_str().into()))
                    .collect(),
                Kind::Ack,
            )
        }
        Request::CallParticipants(t) => (
            Method::GET,
            room("v4/call", t, ""),
            vec![],
            vec![],
            Kind::CallParticipants,
        ),
        Request::SignalingSettings(t) => (
            Method::GET,
            "ocs/v2.php/apps/spreed/api/v3/signaling/settings".into(),
            vec![("token", t.as_str().into())],
            vec![],
            Kind::Settings,
        ),
        Request::JoinCall {
            room: t,
            flags,
            silent,
            recording_consent,
        } => {
            let mut form = vec![
                ("flags", flags.bits().to_string()),
                ("silent", silent.to_string()),
            ];
            if *recording_consent {
                form.push(("recordingConsent", "true".into()));
            }
            (
                Method::POST,
                room("v4/call", t, ""),
                vec![],
                form,
                Kind::Ack,
            )
        }
        Request::UpdateCallFlags { room: t, flags } => (
            Method::PUT,
            room("v4/call", t, ""),
            vec![],
            vec![("flags", flags.bits().to_string())],
            Kind::Ack,
        ),
        Request::LeaveCall(t) => (
            Method::DELETE,
            room("v4/call", t, ""),
            vec![],
            vec![],
            Kind::Ack,
        ),
    };
    let mutation = method != Method::GET;
    Ok((method, route, query, form, kind, mutation))
}
fn list<T>(
    v: &serde_json::Value,
    max: usize,
    decode: impl Fn(&serde_json::Value) -> Result<T, ErrorKind>,
) -> Result<Vec<T>, ErrorKind> {
    let values = v.as_array().ok_or(ErrorKind::Malformed)?;
    if values.len() > max {
        return Err(ErrorKind::TooLarge);
    }
    values.iter().map(decode).collect()
}
fn decode(kind: Kind, r: transport::Response) -> Result<Reply, ErrorKind> {
    let v = &r.data;
    Ok(match kind {
        Kind::Chat(kind) => return chat::decode(kind, v),
        Kind::Conversation(kind) => return conversation::decode(kind, v),
        Kind::Account => Reply::Account(Account::decode(v)?),
        Kind::Capabilities => Reply::Capabilities(Capabilities::decode(v)?),
        Kind::Rooms => {
            // This operation promises full reconciliation. A partial response
            // cannot acquire deletion authority merely by fitting the row bound.
            if r.status != 200 {
                return Err(ErrorKind::Malformed);
            }
            Reply::Rooms(list(v, 100, Room::decode)?)
        }
        Kind::Room => Reply::Room(Room::decode(v)?),
        Kind::Participants => Reply::Participants(list(v, 100, Participant::decode)?),
        Kind::History => Reply::History {
            messages: list(v, 50, Message::decode)?,
            last_given: r.last_given,
            common_read: r.common_read,
        },
        Kind::Message => Reply::Message(Message::decode(v)?),
        Kind::OptionalMessage => {
            if v.is_null() {
                Reply::Acknowledged
            } else {
                Reply::Message(Message::decode(v)?)
            }
        }
        Kind::Session => {
            let s = v["sessionId"]
                .as_str()
                .filter(|s| !s.is_empty() && *s != "0" && s.len() <= 512)
                .ok_or(ErrorKind::Malformed)?;
            Reply::RoomSession(RoomSession(Zeroizing::new(s.into())))
        }
        Kind::Tags => Reply::Tags(list(v, 102, Tag::decode)?),
        Kind::Tag => Reply::Tag(Tag::decode(v)?),
        Kind::CallParticipants => Reply::CallParticipants(list(v, 100, CallParticipant::decode)?),
        Kind::Settings => Reply::SignalingSettings(SignalingSettings::decode(v)?),
        Kind::Ack => Reply::Acknowledged,
    })
}
#[cfg(test)]
mod tests;

#[cfg(test)]
mod alpha_tests;
