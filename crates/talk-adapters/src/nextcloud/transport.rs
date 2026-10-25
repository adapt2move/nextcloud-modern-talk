//! Application correlation/projection adapter. The API crate owns all HTTP mechanics.
use super::projection::Project;
use api::models::{ParticipantFlags, Preference, RoomToken, TagId};
use nextcloud_talk_api as api;
use reqwest::Method;
use serde_json::{Value, json};
#[cfg(any(test, feature = "backend-test"))]
use talk_core::ids::CallScope;
use talk_core::ids::{AccountScope, ClockEpoch, HostId};
use talk_core::lifecycle::Cancellation;
use talk_core::policy::Moment;
use talk_core::ports::{ErrorKind, OperationContext, PortError, Retry};
use zeroize::Zeroizing;
#[derive(Clone)]
pub struct NetworkRuntime(api::NetworkRuntime);
impl NetworkRuntime {
    pub fn new(epoch: ClockEpoch) -> Self {
        Self(api::NetworkRuntime::new(api::scope::ClockId::from_key(
            epoch.as_local(),
        )))
    }
    pub fn now(&self) -> Moment {
        moment(self.0.now())
    }
}
pub(crate) struct ServerOrigin(api::ServerOrigin);
impl ServerOrigin {
    pub(crate) fn https(s: &str) -> Result<Self, ErrorKind> {
        api::ServerOrigin::https(s).map(Self).map_err(kind)
    }
    #[cfg(any(test, feature = "backend-test"))]
    pub(crate) fn loopback(s: &str) -> Result<Self, ErrorKind> {
        api::ServerOrigin::loopback(s).map(Self).map_err(kind)
    }
}
pub(crate) struct AuthenticatedBinding {
    pub(crate) scope: AccountScope,
    pub(crate) host: HostId,
    pub(crate) origin: ServerOrigin,
    /// Exact Login Flow loginName, used only for app-password Basic authentication.
    pub(crate) login_name: Zeroizing<String>,
    /// Canonical OCS user id, used only for identity comparisons/self attribution.
    pub(crate) user: Zeroizing<String>,
    pub(crate) password: Zeroizing<String>,
    pub(crate) cancellation: Cancellation,
}

pub(super) struct Response {
    pub data: Value,
    pub last_given: Option<u64>,
    pub common_read: Option<u64>,
}
pub(crate) struct Transport {
    api: api::Client,
    network: NetworkRuntime,
    scope: AccountScope,
    host: HostId,
    user: Zeroizing<String>,
    cancellation: Cancellation,
}
struct CancellationBridge(Cancellation);
impl api::scope::CancellationSignal for CancellationBridge {
    fn cancel(&self) {
        self.0.cancel();
    }
    fn is_cancelled(&self) -> bool {
        self.0.is_cancelled()
    }
}
fn cancellation(value: &Cancellation) -> api::scope::Cancellation {
    api::scope::Cancellation::from_signal(CancellationBridge(value.clone()))
}
fn scope(s: AccountScope) -> api::scope::SessionScope {
    api::scope::SessionScope {
        account: api::scope::BindingId::from_key(s.account.as_local()),
        runtime: api::scope::LeaseId::from_key(s.runtime.as_local()),
    }
}
fn context(c: &OperationContext) -> api::scope::OperationContext {
    api::scope::OperationContext {
        scope: scope(c.scope),
        deadline: api::scope::Moment {
            epoch: api::scope::ClockId::from_key(c.deadline.epoch.as_local()),
            millis: c.deadline.millis,
        },
        cancellation: cancellation(&c.cancellation),
    }
}
fn moment(m: api::scope::Moment) -> Moment {
    Moment {
        epoch: ClockEpoch::from_local(m.epoch.key()),
        millis: m.millis,
    }
}
#[cfg(any(test, feature = "backend-test"))]
fn call(c: CallScope) -> api::scope::CallKey {
    api::scope::CallKey {
        account: scope(c.account),
        conversation: api::scope::RoomKey {
            account: api::scope::BindingId::from_key(c.conversation.account.as_local()),
            host: api::scope::OriginId::from_key(c.conversation.host.as_local()),
            local: c.conversation.local,
        },
        call: api::scope::NegotiationId::from_key(c.call.as_local()),
    }
}
pub(super) fn kind(k: api::ErrorKind) -> ErrorKind {
    match k {
        api::ErrorKind::Unconfigured => ErrorKind::Unconfigured,
        api::ErrorKind::Unsupported => ErrorKind::Unsupported,
        api::ErrorKind::Cancelled => ErrorKind::Cancelled,
        api::ErrorKind::Deadline => ErrorKind::Deadline,
        api::ErrorKind::Busy => ErrorKind::Busy,
        api::ErrorKind::PermissionDenied => ErrorKind::PermissionDenied,
        api::ErrorKind::ReauthenticationRequired => ErrorKind::ReauthenticationRequired,
        api::ErrorKind::Malformed => ErrorKind::Malformed,
        api::ErrorKind::Uncertain => ErrorKind::Uncertain,
        api::ErrorKind::WrongScope => ErrorKind::WrongScope,
        api::ErrorKind::Transport => ErrorKind::Transport,
        api::ErrorKind::RedirectRejected => ErrorKind::RedirectRejected,
        api::ErrorKind::TooLarge => ErrorKind::TooLarge,
        api::ErrorKind::NotFound => ErrorKind::NotFound,
        api::ErrorKind::Conflict => ErrorKind::Conflict,
        api::ErrorKind::RateLimited => ErrorKind::RateLimited,
        api::ErrorKind::ServerUnavailable => ErrorKind::ServerUnavailable,
        api::ErrorKind::ServerAccessDenied => ErrorKind::ServerAccessDenied,
    }
}
fn error(e: api::Error) -> PortError {
    PortError {
        kind: kind(e.kind),
        account: e
            .account
            .map(|a| talk_core::ids::AccountId::from_local(a.key())),
        retry: match e.retry {
            api::Retry::Never => Retry::Never,
            api::Retry::UserAction => Retry::UserAction,
            api::Retry::AfterReconciliation => Retry::AfterReconciliation,
            api::Retry::AfterDelay { millis } => Retry::AfterDelay { millis },
        },
    }
}
impl Transport {
    pub(super) fn new(b: AuthenticatedBinding, r: NetworkRuntime) -> Result<Self, PortError> {
        let expected = Some(b.user.clone());
        Self::construct(b, r, expected)
    }
    pub(super) fn new_login(
        b: AuthenticatedBinding,
        r: NetworkRuntime,
        expected: Option<Zeroizing<String>>,
    ) -> Result<Self, PortError> {
        Self::construct(b, r, expected)
    }
    fn construct(
        b: AuthenticatedBinding,
        r: NetworkRuntime,
        expected: Option<Zeroizing<String>>,
    ) -> Result<Self, PortError> {
        let network = r.clone();
        let client = api::Client::new(
            api::AppCredential::new(b.origin.0, b.login_name, b.password, expected),
            r.0,
            scope(b.scope),
            api::scope::OriginId::from_key(b.host.as_local()),
            cancellation(&b.cancellation),
        )
        .map_err(error)?;
        Ok(Self {
            api: client,
            network,
            scope: b.scope,
            host: b.host,
            user: b.user,
            cancellation: b.cancellation,
        })
    }
    pub(super) async fn call_invitations(
        &mut self,
        c: &OperationContext,
    ) -> Result<Vec<api::meeting::CallInvitation>, PortError> {
        self.api.call_invitations(&context(c)).await.map_err(error)
    }
    pub(super) async fn invitation_current(
        &mut self,
        c: &OperationContext,
        invitation: &api::meeting::CallInvitation,
    ) -> Result<bool, PortError> {
        self.api
            .invitation_current(&context(c), invitation)
            .await
            .map_err(error)
    }
    pub(super) async fn decline_invitation(
        &mut self,
        c: &OperationContext,
        invitation: api::meeting::CallInvitation,
    ) -> Result<(), PortError> {
        self.api
            .decline_invitation(&context(c), invitation)
            .await
            .map_err(error)
    }
    pub(super) fn network(&self) -> NetworkRuntime {
        self.network.clone()
    }
    pub(super) fn api_now(&self) -> api::scope::Moment {
        self.api.now()
    }
    pub(super) fn own_session(
        &self,
        c: &OperationContext,
        room: &RoomToken,
    ) -> Result<api::signaling::PeerSession, PortError> {
        self.api.own_room_session(&context(c), room).map_err(error)
    }
    pub(super) async fn finish_meeting(
        &mut self,
        c: &OperationContext,
        room: RoomToken,
    ) -> Result<(), PortError> {
        self.api
            .finish_meeting(&context(c), room)
            .await
            .map_err(error)
    }
    pub(super) async fn begin_meeting(
        &mut self,
        c: &OperationContext,
        room: RoomToken,
        flags: ParticipantFlags,
        recording_consent: bool,
        settings: &api::models::SignalingSettings,
    ) -> Result<
        (
            api::meeting::MeetingReader,
            api::meeting::MeetingWriter,
            api::meeting::MeetingStop,
        ),
        PortError,
    > {
        if settings.mode == api::models::SignalingMode::External {
            self.api
                .prepare_external_meeting(&context(c), &room, settings)
                .await
                .map_err(error)?;
        }
        let reply = self
            .api
            .request(
                &context(c),
                api::Request::JoinCall {
                    room,
                    flags,
                    silent: false,
                    recording_consent,
                },
            )
            .await
            .map_err(|e| {
                self.api.cancel_prepared_external_meeting();
                error(e)
            })?;
        let api::Reply::CallJoined(handle) = reply else {
            return Err(self.error(ErrorKind::Malformed));
        };
        self.api.open_meeting(&context(c), handle).map_err(error)
    }
    pub(super) fn scope(&self) -> AccountScope {
        self.scope
    }
    pub(super) fn host(&self) -> HostId {
        self.host
    }
    pub(super) fn user(&self) -> &str {
        &self.user
    }
    pub(super) fn bind_canonical_user(&mut self, user: Zeroizing<String>) {
        self.user = user
    }
    pub(super) fn cancellation(&self) -> Cancellation {
        self.cancellation.clone()
    }
    pub(super) fn now(&self) -> Moment {
        moment(self.api.now())
    }
    pub(super) fn error(&self, kind: ErrorKind) -> PortError {
        PortError {
            kind,
            account: Some(self.scope.account),
            retry: match kind {
                ErrorKind::Uncertain => Retry::AfterReconciliation,
                ErrorKind::ReauthenticationRequired
                | ErrorKind::Transport
                | ErrorKind::ServerUnavailable
                | ErrorKind::Deadline => Retry::UserAction,
                _ => Retry::Never,
            },
        }
    }
    pub(super) fn validate(&self, c: &OperationContext) -> Result<(), PortError> {
        self.api.validate(&context(c)).map_err(error)
    }
    pub(super) async fn upload_file(
        &mut self,
        c: &OperationContext,
        path: api::files::FilePath,
        file: tokio::fs::File,
        size: u64,
    ) -> Result<(), PortError> {
        self.api
            .upload_file(&context(c), path, file, size)
            .await
            .map_err(error)
    }
    pub(super) async fn download_file(
        &mut self,
        c: &OperationContext,
        path: api::files::FilePath,
        file: &mut tokio::fs::File,
        progress: &(dyn Fn(u64, Option<u64>) + Send + Sync),
    ) -> Result<u64, PortError> {
        self.api
            .download_file_progress(&context(c), path, file, progress)
            .await
            .map_err(error)
    }
    pub(super) fn public_room_link(&self, token: &str) -> Result<String, PortError> {
        let token = RoomToken::parse(token).map_err(|k| self.error(kind(k)))?;
        self.api.public_room_link(&token).map_err(error)
    }
    pub(super) async fn image(
        &mut self,
        c: &OperationContext,
        request: api::content::ImageRequest,
    ) -> Result<Vec<u8>, PortError> {
        self.api.image(&context(c), request).await.map_err(error)
    }
    pub(super) async fn video(
        &mut self,
        c: &OperationContext,
        path: api::files::FilePath,
        format: api::content::VideoFormat,
    ) -> Result<Vec<u8>, PortError> {
        self.api
            .video(&context(c), path, format)
            .await
            .map_err(error)
    }
    #[cfg(any(test, target_os = "macos"))]
    pub(super) async fn preview_pdf(
        &mut self,
        c: &OperationContext,
        path: api::files::FilePath,
    ) -> Result<Vec<u8>, PortError> {
        self.api.preview_pdf(&context(c), path).await.map_err(error)
    }
    pub(super) async fn poll(
        &mut self,
        c: &OperationContext,
        room: &api::models::RoomToken,
        command: api::poll::Command,
    ) -> Result<api::poll::Poll, PortError> {
        self.api
            .poll(&context(c), room, command)
            .await
            .map_err(error)
    }
    pub(super) fn file_link(&self, file: u64) -> Result<String, PortError> {
        self.api.file_link(file).map_err(error)
    }
    pub(super) async fn api_request(
        &mut self,
        c: &OperationContext,
        request: api::Request,
    ) -> Result<api::Reply, PortError> {
        self.api.request(&context(c), request).await.map_err(error)
    }
    pub(super) async fn own_user_status(
        &mut self,
        c: &OperationContext,
    ) -> Result<api::presence::UserStatus, PortError> {
        self.api.own_user_status(&context(c)).await.map_err(error)
    }
    pub(super) async fn message_notifications(
        &mut self,
        c: &OperationContext,
    ) -> Result<api::notifications::MessageNotifications, PortError> {
        self.api
            .message_notifications(&context(c))
            .await
            .map_err(error)
    }
    pub(super) async fn poll_live_events(
        &mut self,
        c: &OperationContext,
    ) -> Result<talk_core::protocol::AccountLiveEvents, PortError> {
        self.validate(c)?;
        let batch = self
            .api
            .poll_live_events(&context(c))
            .await
            .map_err(error)?;
        self.validate(c)?;
        super::live::project(self.scope, batch).map_err(|kind| self.error(kind))
    }
    pub(super) async fn history(
        &mut self,
        c: &OperationContext,
        route: &str,
        query: &[(&str, String)],
    ) -> Result<Response, PortError> {
        let room = route
            .strip_prefix("ocs/v2.php/apps/spreed/api/v1/chat/")
            .ok_or_else(|| self.error(ErrorKind::WrongScope))?;
        let room = RoomToken::parse(room).map_err(|k| self.error(kind(k)))?;
        let before = query
            .iter()
            .find(|(k, _)| *k == "lastKnownMessageId")
            .map(|(_, v)| {
                v.parse::<u64>()
                    .map_err(|_| self.error(ErrorKind::Malformed))
            })
            .transpose()?;
        self.api
            .request(&context(c), api::Request::History { room, before })
            .await
            .map_err(error)
            .and_then(|reply| project(reply).map_err(|kind| self.error(kind)))
    }
    pub(super) async fn ocs(
        &mut self,
        c: &OperationContext,
        method: Method,
        route: &str,
        query: &[(&str, String)],
        form: &[(&str, String)],
        account_probe: bool,
    ) -> Result<Response, PortError> {
        if account_probe
            && (method != Method::GET
                || route != "ocs/v2.php/cloud/user"
                || !query.is_empty()
                || !form.is_empty())
        {
            return Err(self.error(ErrorKind::WrongScope));
        }
        #[cfg(any(test, feature = "backend-test"))]
        if let Some(token) = route.strip_prefix("ocs/v2.php/apps/spreed/api/v3/signaling/")
            && token != "settings"
            && method == Method::POST
        {
            let room = RoomToken::parse(token).map_err(|k| self.error(kind(k)))?;
            let messages = value(form, "messages").map_err(|k| self.error(k))?;
            self.api
                .diagnostic_send(&context(c), &room, messages)
                .await
                .map_err(error)?;
            return Ok(Response {
                data: json!([]),
                last_given: None,
                common_read: None,
            });
        }
        let request = request(method, route, query, form).map_err(|k| self.error(k))?;
        self.api
            .request(&context(c), request)
            .await
            .map_err(error)
            .and_then(|reply| project(reply).map_err(|kind| self.error(kind)))
    }
}
fn value<'a>(pairs: &'a [(&str, String)], key: &str) -> Result<&'a str, ErrorKind> {
    pairs
        .iter()
        .find(|(k, _)| *k == key)
        .map(|(_, v)| v.as_str())
        .ok_or(ErrorKind::Malformed)
}
fn request(
    method: Method,
    route: &str,
    query: &[(&str, String)],
    form: &[(&str, String)],
) -> Result<api::Request, ErrorKind> {
    use api::Request as R;
    let get = method == Method::GET;
    let post = method == Method::POST;
    let delete = method == Method::DELETE;
    let v = |k| value(form, k).map(str::to_owned);
    match route {
        "ocs/v2.php/cloud/user" if get => return Ok(R::Account),
        "ocs/v2.php/cloud/capabilities" if get => return Ok(R::Capabilities),
        "ocs/v2.php/apps/spreed/api/v4/room" if get => return Ok(R::Rooms),
        "ocs/v2.php/apps/spreed/api/v4/room" if post => {
            return Ok(R::CreateGroup {
                name: v("roomName")?,
            });
        }
        "ocs/v2.php/apps/spreed/api/v1/settings/user" if post => {
            return Ok(R::SetPreference(match value(form, "key")? {
                "read_status_privacy" => Preference::ReadReceipts(value(form, "value")? == "0"),
                "typing_privacy" => Preference::TypingStatus(value(form, "value")? == "0"),
                "attachment_folder" => Preference::AttachmentFolder(v("value")?),
                _ => return Err(ErrorKind::Unsupported),
            }));
        }
        "ocs/v2.php/apps/spreed/api/v4/tags" if get => return Ok(R::Tags),
        "ocs/v2.php/apps/spreed/api/v4/tags" if post => {
            return Ok(R::CreateTag { name: v("name")? });
        }
        "ocs/v2.php/apps/spreed/api/v4/tags/reorder" => {
            return Ok(R::ReorderTags(
                form.iter()
                    .map(|(_, v)| TagId::parse(v).map_err(kind))
                    .collect::<Result<_, _>>()?,
            ));
        }
        "ocs/v2.php/apps/spreed/api/v3/signaling/settings" if get => {
            return Ok(R::SignalingSettings(
                RoomToken::parse(value(query, "token")?).map_err(kind)?,
            ));
        }
        _ => {}
    }
    if let Some(rest) = route.strip_prefix("ocs/v2.php/apps/spreed/api/v4/tags/") {
        let (id, suffix) = rest.split_once('/').unwrap_or((rest, ""));
        let tag = TagId::parse(id).map_err(kind)?;
        return Ok(if delete {
            R::DeleteTag(tag)
        } else if suffix == "collapsed" {
            R::CollapseTag {
                tag,
                collapsed: value(form, "collapsed")? == "true",
            }
        } else {
            R::RenameTag {
                tag,
                name: v("name")?,
            }
        });
    }
    for prefix in ["v4/room", "v1/chat", "v4/call"] {
        if let Some(rest) = route.strip_prefix(&format!("ocs/v2.php/apps/spreed/api/{prefix}/")) {
            let (token, suffix) = rest.split_once('/').unwrap_or((rest, ""));
            let room = RoomToken::parse(token).map_err(kind)?;
            return Ok(match (prefix, suffix) {
                ("v4/room", "") if get => R::Room(room),
                ("v4/room", "participants") if get => R::Participants(room),
                ("v4/room", "participants/self") if delete => R::LeaveRoom(room),
                ("v4/room", "participants/active") if post => R::ActivateRoom(room),
                ("v4/room", "participants/active") if delete => R::DeactivateRoom(room),
                ("v4/room", "archive") => R::SetArchived {
                    room,
                    archived: post,
                },
                ("v4/room", "favorite") => R::SetFavorite {
                    room,
                    favorite: post,
                },
                ("v4/room", "tags") => R::SetRoomTags {
                    room,
                    tags: form
                        .iter()
                        .map(|(_, v)| TagId::parse(v).map_err(kind))
                        .collect::<Result<_, _>>()?,
                },
                ("v1/chat", "") if post => R::SendMessage {
                    room,
                    message: v("message")?,
                    reference: v("referenceId")?,
                },
                ("v1/chat", "read") => R::MarkRead {
                    room,
                    message: value(form, "lastReadMessage")?
                        .parse()
                        .map_err(|_| ErrorKind::Malformed)?,
                },
                ("v4/call", "") if get => R::CallParticipants(room),
                ("v4/call", "") if delete => R::LeaveCall(room),
                ("v4/call", "") if post => {
                    let flags: u8 = value(form, "flags")?
                        .parse()
                        .map_err(|_| ErrorKind::Malformed)?;
                    if flags & !7 != 0 || flags & 1 == 0 {
                        return Err(ErrorKind::Unsupported);
                    }
                    R::JoinCall {
                        room,
                        flags: ParticipantFlags::new(flags & 2 != 0, flags & 4 != 0),
                        silent: value(form, "silent")? == "true",
                        recording_consent: false,
                    }
                }
                _ => return Err(ErrorKind::Unsupported),
            });
        }
    }
    Err(ErrorKind::Unsupported)
}
fn project(reply: api::Reply) -> Result<Response, ErrorKind> {
    use api::Reply as R;
    let mut last_given = None;
    let mut common_read = None;
    let data = match reply {
        R::Chat(_) | R::Conversation(_) => return Err(ErrorKind::Malformed),
        R::Account(v) => v.project(),
        R::Capabilities(v) => v.project(),
        R::Room(v) => v.project(),
        R::Message(v) => v.project(),
        R::Tag(v) => v.project(),
        R::SignalingSettings(v) => v.project(),
        R::Rooms(v) => Value::Array(v.into_iter().map(Project::project).collect()),
        R::Participants(v) => Value::Array(v.into_iter().map(Project::project).collect()),
        R::Tags(v) => Value::Array(v.into_iter().map(Project::project).collect()),
        R::CallParticipants(v) => Value::Array(v.into_iter().map(Project::project).collect()),
        R::History {
            messages,
            last_given: lg,
            common_read: cr,
        } => {
            last_given = lg;
            common_read = cr;
            Value::Array(messages.into_iter().map(Project::project).collect())
        }
        R::RoomSession(v) => json!({"sessionId":v.into_secret().as_str()}),
        R::Acknowledged | R::CallJoined(_) => json!([]),
    };
    Ok(Response {
        data,
        last_given,
        common_read,
    })
}
#[cfg(any(test, feature = "backend-test"))]
pub(super) use api::diagnostics::{
    Completion as DiagnosticCompletion, Lane as DiagnosticLane, Prepared as DiagnosticPrepared,
};
#[cfg(any(test, feature = "backend-test"))]
pub(super) struct DiagnosticCallAdmission {
    inner: api::diagnostics::Admission,
    call: CallScope,
}
#[cfg(any(test, feature = "backend-test"))]
impl DiagnosticCallAdmission {
    pub(super) fn scope(&self) -> CallScope {
        self.call
    }
}
#[cfg(any(test, feature = "backend-test"))]
pub(super) enum DiagnosticOperation<'a> {
    Read { route: &'a str },
    Signal { route: &'a str, messages: &'a str },
    Leave { route: &'a str },
}
#[cfg(any(test, feature = "backend-test"))]
impl Transport {
    pub(super) fn diagnostic_call_admission(
        &self,
        c: &OperationContext,
        scope: CallScope,
    ) -> Result<DiagnosticCallAdmission, PortError> {
        self.api
            .diagnostic_admit(&context(c), call(scope))
            .map(|inner| DiagnosticCallAdmission { inner, call: scope })
            .map_err(error)
    }
    pub(super) fn diagnostic_response_slot(
        &self,
    ) -> Result<tokio::sync::OwnedSemaphorePermit, PortError> {
        self.api.diagnostic_response_slot().map_err(error)
    }
    pub(super) fn diagnostic_duplex(
        &mut self,
        c: &OperationContext,
        token: &mut DiagnosticCallAdmission,
    ) -> Result<(DiagnosticLane, DiagnosticLane), PortError> {
        self.api
            .diagnostic_duplex(&context(c), &mut token.inner)
            .map_err(error)
    }
    pub(super) fn diagnostic_stop(&self) {
        self.api.diagnostic_stop()
    }
    pub(super) async fn diagnostic_poll(
        &mut self,
        c: &OperationContext,
        route: &str,
    ) -> Result<Response, PortError> {
        let token = self.diagnostic_token(route, false)?;
        self.api
            .diagnostic_poll(&context(c), &token)
            .await
            .map(|data| Response {
                data,
                last_given: None,
                common_read: None,
            })
            .map_err(error)
    }
    fn diagnostic_token(&self, route: &str, leave: bool) -> Result<RoomToken, PortError> {
        let prefix = if leave {
            "ocs/v2.php/apps/spreed/api/v4/call/"
        } else {
            "ocs/v2.php/apps/spreed/api/v3/signaling/"
        };
        let token = route
            .strip_prefix(prefix)
            .ok_or_else(|| self.error(ErrorKind::WrongScope))?;
        RoomToken::parse(token).map_err(|k| self.error(kind(k)))
    }
    pub(super) fn diagnostic_prepare(
        &mut self,
        c: &OperationContext,
        scope: CallScope,
        lane: DiagnosticLane,
        operation: DiagnosticOperation<'_>,
    ) -> Result<DiagnosticPrepared, PortError> {
        let (route, leave) = match operation {
            DiagnosticOperation::Read { route } | DiagnosticOperation::Signal { route, .. } => {
                (route, false)
            }
            DiagnosticOperation::Leave { route } => (route, true),
        };
        let token = self.diagnostic_token(route, leave)?;
        let operation = match operation {
            DiagnosticOperation::Read { .. } => api::diagnostics::Operation::Read(&token),
            DiagnosticOperation::Signal { messages, .. } => {
                api::diagnostics::Operation::Signal(&token, messages)
            }
            DiagnosticOperation::Leave { .. } => api::diagnostics::Operation::Leave(&token),
        };
        self.api
            .diagnostic_prepare(&context(c), call(scope), lane, operation)
            .map_err(error)
    }
    pub(super) fn diagnostic_commit(
        &mut self,
        c: &OperationContext,
        scope: CallScope,
        completion: DiagnosticCompletion,
    ) -> (DiagnosticLane, Result<Response, PortError>) {
        let (lane, r) = self
            .api
            .diagnostic_commit(&context(c), call(scope), completion);
        (
            lane,
            r.map(|data| Response {
                data,
                last_given: None,
                common_read: None,
            })
            .map_err(error),
        )
    }
}
