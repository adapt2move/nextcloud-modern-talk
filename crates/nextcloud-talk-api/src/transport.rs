//! Origin-bound HTTP owner. Never exported to core or the renderer.
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use crate::error::{ErrorKind, PortError, Retry};
#[cfg(any(test, feature = "backend-test"))]
use crate::scope::CallKey;
use crate::scope::Cancellation;
use crate::scope::Moment;
use crate::scope::OperationContext;
use crate::scope::{BindingId, ClockId, OriginId, SessionScope};
use reqwest::header::{
    ACCEPT, ACCEPT_ENCODING, CACHE_CONTROL, CONTENT_ENCODING, CONTENT_TYPE, COOKIE, HeaderValue,
    SET_COOKIE,
};
use reqwest::{Client, Method};
use tokio::sync::{OwnedSemaphorePermit, Semaphore};
use url::Url;
use zeroize::Zeroizing;

use crate::json;
mod files;
mod meeting;
pub(crate) use meeting::MeetingLifetime;

#[cfg(any(test, feature = "backend-test"))]
mod duplex;
#[cfg(any(test, feature = "backend-test"))]
#[allow(
    unused_imports,
    reason = "Private API consumed by the coordinated diagnostic owner"
)]
pub(crate) use duplex::{
    DiagnosticCompletion, DiagnosticLane, DiagnosticOperation, DiagnosticPrepared,
};

pub(crate) const BODY_LIMIT: usize = crate::MAX_RESPONSE_BYTES;
const REQUEST_LIMIT: Duration = Duration::from_secs(3);

/// One application-wide owner, shared by every account transport. No work queue.
#[derive(Clone)]
pub struct NetworkRuntime(Arc<Runtime>);
struct Runtime {
    requests: Arc<Semaphore>,
    accounts: Arc<Semaphore>,
    meetings: Arc<Semaphore>,
    #[cfg(any(test, feature = "backend-test"))]
    diagnostic_responses: Arc<Semaphore>,
    #[cfg(any(test, feature = "backend-test"))]
    diagnostic_calls: Mutex<[Option<CallKey>; 8]>,
    #[cfg(any(test, feature = "backend-test"))]
    diagnostic_loops: Mutex<[Option<BindingId>; 2]>,
    start: Instant,
    epoch: ClockId,
    bindings: Mutex<Vec<BindingId>>,
}
impl NetworkRuntime {
    pub fn new(epoch: ClockId) -> Self {
        Self(Arc::new(Runtime {
            requests: Arc::new(Semaphore::new(8)),
            accounts: Arc::new(Semaphore::new(8)),
            meetings: Arc::new(Semaphore::new(2)),
            #[cfg(any(test, feature = "backend-test"))]
            diagnostic_responses: Arc::new(Semaphore::new(8)),
            #[cfg(any(test, feature = "backend-test"))]
            diagnostic_calls: Mutex::new([None; 8]),
            #[cfg(any(test, feature = "backend-test"))]
            diagnostic_loops: Mutex::new([None; 2]),
            start: Instant::now(),
            epoch,
            bindings: Mutex::new(Vec::new()),
        }))
    }
    /// Ordinary request timing only; this clock does not qualify suspend-safe media leases.
    pub fn now(&self) -> Moment {
        Moment {
            epoch: self.0.epoch,
            millis: self.0.start.elapsed().as_millis().min(u64::MAX as u128) as u64,
        }
    }
}

/// Created by the identity owner after its own pending/ledger/host checks.
/// No Debug/Clone/Display and no credential getter; never a core/UI value.
pub(crate) struct AuthenticatedBinding {
    pub(crate) scope: SessionScope,
    pub(crate) host: OriginId,
    pub(crate) origin: ServerOrigin,
    /// Exact Login Flow loginName, used only for app-password Basic authentication.
    pub(crate) login_name: Zeroizing<String>,
    /// Canonical OCS user id, used only for identity comparisons/self attribution.
    pub(crate) user: Zeroizing<String>,
    pub(crate) password: Zeroizing<String>,
    pub(crate) cancellation: Cancellation,
}

pub struct ServerOrigin(Url);
impl ServerOrigin {
    #[allow(
        dead_code,
        reason = "identity owner will use the production constructor"
    )]
    pub fn https(input: &str) -> Result<Self, ErrorKind> {
        Self::parse(input, false)
    }

    fn parse(input: &str, loopback: bool) -> Result<Self, ErrorKind> {
        // Reject ambiguous normalization before URL parsing can discard dot segments/userinfo.
        if input.len() > 2048
            || input.trim() != input
            || input.contains(['\\', '?', '#', '@'])
            || input.chars().any(char::is_control)
        {
            return Err(ErrorKind::Malformed);
        }
        let mut url = Url::parse(input).map_err(|_| ErrorKind::Malformed)?;
        let host = url.host_str().ok_or(ErrorKind::Malformed)?;
        let literal_loopback = host == "127.0.0.1" || host == "[::1]";
        if url.scheme() != "https" && !(loopback && url.scheme() == "http" && literal_loopback) {
            return Err(ErrorKind::Unsupported);
        }
        if !url.username().is_empty()
            || url.password().is_some()
            || host.ends_with('.')
            || url.port() == Some(0)
        {
            return Err(ErrorKind::Malformed);
        }
        let raw_path = input
            .split_once("://")
            .ok_or(ErrorKind::Malformed)?
            .1
            .split_once('/')
            .map_or("", |(_, path)| path);
        // A deliberately conservative installation-base grammar; no encoded traversal.
        if raw_path.split('/').any(|s| matches!(s, "." | ".."))
            || raw_path.contains('%')
            || raw_path.contains("//")
            || !raw_path
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || b"/-_.~".contains(&b))
        {
            return Err(ErrorKind::Malformed);
        }
        if !url.path().ends_with('/') {
            url.set_path(&format!("{}/", url.path()));
        }
        Ok(Self(url))
    }

    #[cfg(any(test, feature = "backend-test"))]
    pub fn loopback(input: &str) -> Result<Self, ErrorKind> {
        let value = Self::parse(input, true)?;
        if !matches!(value.0.host_str(), Some("127.0.0.1" | "[::1]")) {
            return Err(ErrorKind::Unsupported);
        }
        Ok(value)
    }

    fn endpoint(&self, route: &str) -> Result<Url, ErrorKind> {
        // Only reviewed private route constructors call this method.
        if route.starts_with('/')
            || route.contains(['?', '#', '%', '\\'])
            || route.split('/').any(|v| matches!(v, "." | ".."))
        {
            return Err(ErrorKind::Malformed);
        }
        let url = self.0.join(route).map_err(|_| ErrorKind::Malformed)?;
        if url.origin() != self.0.origin() || !url.path().starts_with(self.0.path()) {
            return Err(ErrorKind::WrongScope);
        }
        Ok(url)
    }
}

pub(crate) struct Response {
    pub status: u16,
    pub notification_allowed: Option<bool>,
    pub data: serde_json::Value,
    pub last_given: Option<u64>,
    pub common_read: Option<u64>,
}

struct RequestOptions {
    account_probe: bool,
    limit: Duration,
    empty_history: bool,
}

#[derive(Clone, Eq, PartialEq)]
struct SessionCookie {
    name: String,
    value: Zeroizing<String>,
    path: String,
    secure: bool,
    expires: Option<Instant>,
}

pub(crate) struct Transport {
    binding: AuthenticatedBinding,
    client: Client,
    runtime: NetworkRuntime,
    _account_charge: Arc<AccountCharge>,
    #[cfg(any(test, feature = "backend-test"))]
    diagnostic_loop: std::sync::Weak<duplex::LoopCharge>,
    #[cfg(any(test, feature = "backend-test"))]
    diagnostic_cookies: duplex::CookieOrder,
    cookies: Vec<SessionCookie>,
    shared_cookies: Option<Arc<Mutex<meeting::SharedCookies>>>,
    cancel_account_on_drop: bool,
    meeting_active: Arc<std::sync::atomic::AtomicBool>,
    meeting_lifetime: std::sync::Weak<MeetingLifetime>,
}

// The reservation follows prepared authenticated I/O after the client retires.
pub(crate) struct AccountCharge {
    runtime: NetworkRuntime,
    account: BindingId,
    _slot: OwnedSemaphorePermit,
}
impl Drop for AccountCharge {
    fn drop(&mut self) {
        if let Ok(mut bindings) = self.runtime.0.bindings.lock() {
            bindings.retain(|account| *account != self.account);
        }
    }
}

/// One irreversible synthetic-call admission. Only this module can construct it.
/// The diagnostic state retains it; dropping it never removes its runtime tombstone.
#[cfg(any(test, feature = "backend-test"))]
#[allow(
    dead_code,
    reason = "Consumed by the coordinated diagnostic signaling state"
)]
pub(crate) struct DiagnosticCallAdmission {
    call: CallKey,
    duplex_started: bool,
}
#[cfg(any(test, feature = "backend-test"))]
#[allow(
    dead_code,
    reason = "Consumed by the coordinated diagnostic signaling state"
)]
impl DiagnosticCallAdmission {
    pub(crate) fn scope(&self) -> CallKey {
        self.call
    }
}

impl Transport {
    pub(crate) fn hpb_binding(&self) -> Result<crate::hpb::AccountBinding, PortError> {
        let backend = self
            .binding
            .origin
            .endpoint("ocs/v2.php/apps/spreed/api/v3/signaling/backend")
            .map_err(|e| self.error(e))?;
        Ok(crate::hpb::AccountBinding {
            scope: self.scope(),
            backend: backend.to_string(),
            origin: self.binding.origin.0.origin().ascii_serialization(),
            user: self.binding.user.clone(),
            cancellation: self.binding.cancellation.clone(),
            runtime: self.runtime.clone(),
            _charge: self._account_charge.clone(),
        })
    }
    /// Equality-only lifetime ledger, separate from HTTP and response permits.
    /// The caller checks its locally admitted room before invoking this hook.
    #[cfg(any(test, feature = "backend-test"))]
    #[allow(
        dead_code,
        reason = "Consumed by the coordinated diagnostic signaling state"
    )]
    pub(crate) fn diagnostic_call_admission(
        &self,
        context: &OperationContext,
        call: CallKey,
    ) -> Result<DiagnosticCallAdmission, PortError> {
        self.validate(context)?;
        if call.account != self.scope()
            || !call.is_consistent()
            || call.conversation.host != self.host()
        {
            return Err(self.error(ErrorKind::WrongScope));
        }
        let mut calls = self
            .runtime
            .0
            .diagnostic_calls
            .lock()
            .map_err(|_| self.error(ErrorKind::Unconfigured))?;
        // Duplicate remains Conflict even at capacity. Opaque generations are
        // compared in full and are never sorted, advanced or reused here.
        if calls.contains(&Some(call)) {
            return Err(self.error(ErrorKind::Conflict));
        }
        let slot = calls
            .iter_mut()
            .find(|slot| slot.is_none())
            .ok_or_else(|| self.error(ErrorKind::Busy))?;
        *slot = Some(call);
        Ok(DiagnosticCallAdmission {
            call,
            duplex_started: false,
        })
    }
    /// Default-off typed response lifetime reservation, independent of HTTP slots.
    /// The owned permit must survive client retirement until every guarded page drops.
    #[cfg(any(test, feature = "backend-test"))]
    #[allow(
        dead_code,
        reason = "Media mailbox consumes this coordinated private hook"
    )]
    pub(crate) fn diagnostic_response_slot(&self) -> Result<OwnedSemaphorePermit, PortError> {
        self.runtime
            .0
            .diagnostic_responses
            .clone()
            .try_acquire_owned()
            .map_err(|_| self.error(ErrorKind::Busy))
    }
    pub(crate) fn new(
        binding: AuthenticatedBinding,
        runtime: NetworkRuntime,
    ) -> Result<Self, PortError> {
        let error = |kind| PortError {
            kind,
            account: Some(binding.scope.account),
            retry: Retry::Never,
        };
        if binding.cancellation.is_cancelled() {
            return Err(error(ErrorKind::Cancelled));
        }
        if binding.login_name.is_empty()
            || binding.login_name.len() > 256
            || binding.login_name.contains(':')
            || binding.login_name.chars().any(char::is_control)
            || binding.user.is_empty()
            || binding.user.len() > 256
            || binding.user.contains(':')
            || binding.user.chars().any(char::is_control)
            || binding.password.is_empty()
            || binding.password.len() > 8192
            || binding.password.chars().any(char::is_control)
        {
            return Err(error(ErrorKind::Malformed));
        }
        let slot = runtime
            .0
            .accounts
            .clone()
            .try_acquire_owned()
            .map_err(|_| error(ErrorKind::Busy))?;
        let client = Client::builder()
            .https_only(binding.origin.0.scheme() == "https")
            .redirect(reqwest::redirect::Policy::none())
            .retry(reqwest::retry::never())
            .referer(false)
            .no_proxy()
            .http1_only()
            .http1_max_headers(64)
            .no_gzip()
            .no_brotli()
            .no_deflate()
            .no_zstd()
            .connect_timeout(REQUEST_LIMIT)
            .read_timeout(Duration::from_secs(35))
            .timeout(REQUEST_LIMIT)
            .pool_max_idle_per_host(1)
            .pool_idle_timeout(Duration::from_secs(30))
            .user_agent("nextcloud-modern-talk/0.0.0")
            .build()
            .map_err(|_| error(ErrorKind::Transport))?;
        {
            let mut bindings = runtime
                .0
                .bindings
                .lock()
                .map_err(|_| error(ErrorKind::Unconfigured))?;
            if bindings.contains(&binding.scope.account) {
                return Err(error(ErrorKind::Conflict));
            }
            bindings.push(binding.scope.account);
        }
        let account_charge = Arc::new(AccountCharge {
            runtime: runtime.clone(),
            account: binding.scope.account,
            _slot: slot,
        });
        Ok(Self {
            binding,
            client,
            runtime,
            _account_charge: account_charge,
            #[cfg(any(test, feature = "backend-test"))]
            diagnostic_loop: std::sync::Weak::new(),
            #[cfg(any(test, feature = "backend-test"))]
            diagnostic_cookies: duplex::CookieOrder::default(),
            cookies: Vec::new(),
            shared_cookies: None,
            cancel_account_on_drop: true,
            meeting_active: Arc::new(std::sync::atomic::AtomicBool::new(false)),
            meeting_lifetime: std::sync::Weak::new(),
        })
    }

    pub(crate) fn public_room_link(
        &self,
        token: &crate::models::RoomToken,
    ) -> Result<String, PortError> {
        self.binding
            .origin
            .endpoint(&format!("index.php/call/{}", token.as_str()))
            .map(|u| u.to_string())
            .map_err(|k| self.error(k))
    }
    pub(crate) fn scope(&self) -> SessionScope {
        self.binding.scope
    }
    pub(crate) fn host(&self) -> OriginId {
        self.binding.host
    }
    pub(crate) fn user(&self) -> &str {
        &self.binding.user
    }
    /// Only the consuming fresh-login qualification chain calls this after its
    /// authoritative probe. No live room/send/session owner is exposed beforehand.
    pub(crate) fn bind_canonical_user(&mut self, user: Zeroizing<String>) {
        self.binding.user = user;
    }
    pub(crate) fn cancellation(&self) -> Cancellation {
        self.binding.cancellation.clone()
    }
    pub(crate) fn now(&self) -> Moment {
        self.runtime.now()
    }
    pub(crate) fn error(&self, kind: ErrorKind) -> PortError {
        PortError {
            kind,
            account: Some(self.scope().account),
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
    pub(crate) fn validate(&self, context: &OperationContext) -> Result<(), PortError> {
        if context.scope != self.scope() {
            return Err(self.error(ErrorKind::WrongScope));
        }
        if self.binding.cancellation.is_cancelled() || context.cancellation.is_cancelled() {
            return Err(self.error(ErrorKind::Cancelled));
        }
        let now = self.runtime.now();
        if context.deadline.epoch != now.epoch || context.deadline.millis <= now.millis {
            return Err(self.error(ErrorKind::Deadline));
        }
        Ok(())
    }

    pub(crate) async fn ocs(
        &mut self,
        context: &OperationContext,
        method: Method,
        route: &str,
        query: &[(&str, String)],
        form: &[(&str, String)],
        account_probe: bool,
    ) -> Result<Response, PortError> {
        self.perform(
            context,
            method,
            route,
            query,
            form,
            RequestOptions {
                account_probe,
                limit: REQUEST_LIMIT,
                empty_history: false,
            },
        )
        .await
    }
    pub(crate) async fn history(
        &mut self,
        context: &OperationContext,
        route: &str,
        query: &[(&str, String)],
    ) -> Result<Response, PortError> {
        self.perform(
            context,
            Method::GET,
            route,
            query,
            &[],
            RequestOptions {
                account_probe: false,
                limit: REQUEST_LIMIT,
                empty_history: true,
            },
        )
        .await
    }
    pub(crate) async fn signaling_poll(
        &mut self,
        context: &OperationContext,
        route: &str,
    ) -> Result<Response, PortError> {
        self.perform(
            context,
            Method::GET,
            route,
            &[],
            &[],
            RequestOptions {
                account_probe: false,
                limit: Duration::from_secs(35),
                empty_history: false,
            },
        )
        .await
    }
    async fn perform(
        &mut self,
        context: &OperationContext,
        method: Method,
        route: &str,
        query: &[(&str, String)],
        form: &[(&str, String)],
        options: RequestOptions,
    ) -> Result<Response, PortError> {
        self.validate(context)?;
        let shared_cookies = self.begin_shared_cookies()?;
        if options.account_probe
            && (method != Method::GET
                || route != "ocs/v2.php/cloud/user"
                || !query.is_empty()
                || !form.is_empty())
        {
            return Err(self.error(ErrorKind::WrongScope));
        }
        #[cfg(any(test, feature = "backend-test"))]
        if self.diagnostic_loop.upgrade().is_some() {
            return Err(self.error(ErrorKind::Busy));
        }
        let _permit = self
            .runtime
            .0
            .requests
            .clone()
            .try_acquire_owned()
            .map_err(|_| self.error(ErrorKind::Busy))?;
        let mut url = self
            .binding
            .origin
            .endpoint(route)
            .map_err(|kind| self.error(kind))?;
        {
            let mut pairs = url.query_pairs_mut();
            pairs.append_pair("format", "json");
            for (key, value) in query {
                pairs.append_pair(key, value);
            }
        }
        let mut request = self
            .client
            .request(method.clone(), url.clone())
            .timeout(options.limit)
            .header(ACCEPT, "application/json")
            .header(ACCEPT_ENCODING, "identity")
            .header(CACHE_CONTROL, "no-store, no-cache, max-age=0")
            .header("Pragma", "no-cache")
            .header("OCS-APIRequest", "true")
            .basic_auth(
                self.binding.login_name.as_str(),
                Some(self.binding.password.as_str()),
            );
        self.cookies
            .retain(|cookie| cookie.expires.is_none_or(|time| time > Instant::now()));
        let mut cookie_header = Zeroizing::new(String::new());
        for cookie in &self.cookies {
            if (!cookie.secure || url.scheme() == "https")
                && cookie_path_matches(url.path(), &cookie.path)
            {
                if !cookie_header.is_empty() {
                    cookie_header.push_str("; ");
                }
                cookie_header.push_str(&cookie.name);
                cookie_header.push('=');
                cookie_header.push_str(&cookie.value);
            }
        }
        if !cookie_header.is_empty() {
            let mut value = HeaderValue::from_str(&cookie_header)
                .map_err(|_| self.error(ErrorKind::Malformed))?;
            value.set_sensitive(true);
            request = request.header(COOKIE, value);
        }
        if !form.is_empty() {
            request = request.form(form);
        }
        let request = request
            .build()
            .map_err(|_| self.error(ErrorKind::Malformed))?;
        if request
            .body()
            .and_then(reqwest::Body::as_bytes)
            .is_some_and(|body| body.len() > BODY_LIMIT)
        {
            return Err(self.error(ErrorKind::TooLarge));
        }
        let deadline = Duration::from_millis(
            context
                .deadline
                .millis
                .checked_sub(self.now().millis)
                .ok_or_else(|| self.error(ErrorKind::Deadline))?,
        )
        .min(options.limit);
        let cancel = self.binding.cancellation.clone();
        let work = async {
            let mut response = self.client.execute(request).await.map_err(|e| {
                self.error(if e.is_timeout() {
                    ErrorKind::Deadline
                } else {
                    ErrorKind::Transport
                })
            })?;
            if response.url().origin() != self.binding.origin.0.origin() {
                return Err(self.error(ErrorKind::WrongScope));
            }
            let status = response.status().as_u16();
            // Talk chat explicitly uses a bodyless 304 for an exhausted history page.
            // No other operation interprets 304 as success or follows its Location.
            if status == 304 && options.empty_history {
                return Ok(Response {
                    status,
                    notification_allowed: None,
                    data: serde_json::Value::Array(Vec::new()),
                    last_given: None,
                    common_read: header_number(response.headers(), "X-Chat-Last-Common-Read")
                        .map_err(|k| self.error(k))?,
                });
            }
            // Reject before reading, following or copying any redirect location.
            if (300..400).contains(&status) {
                return Err(self.error(ErrorKind::RedirectRejected));
            }
            if response
                .headers()
                .get_all(CONTENT_ENCODING)
                .iter()
                .any(|h| h.as_bytes() != b"identity")
            {
                return Err(self.error(ErrorKind::Unsupported));
            }
            if response
                .content_length()
                .is_some_and(|n| n > BODY_LIMIT as u64)
            {
                return Err(self.error(ErrorKind::TooLarge));
            }
            let content_type = response
                .headers()
                .get(CONTENT_TYPE)
                .and_then(|h| h.to_str().ok())
                .unwrap_or("")
                .split(';')
                .next()
                .unwrap_or("");
            if status != 304 && content_type != "application/json" {
                return Err(self.error(ErrorKind::Malformed));
            }
            let last_given = header_number(response.headers(), "X-Chat-Last-Given")
                .map_err(|k| self.error(k))?;
            let common_read = header_read_marker(response.headers(), "X-Chat-Last-Common-Read")
                .map_err(|k| self.error(k))?;
            let retry_seconds = header_number(response.headers(), "Retry-After")
                .ok()
                .flatten()
                .map(|n| n.clamp(1, 300) as u32)
                .unwrap_or(30);
            let mut body = Zeroizing::new(Vec::with_capacity(8192));
            while let Some(chunk) = response
                .chunk()
                .await
                .map_err(|_| self.error(ErrorKind::Transport))?
            {
                if chunk.len() > BODY_LIMIT - body.len() {
                    return Err(self.error(ErrorKind::TooLarge));
                }
                body.extend_from_slice(&chunk);
            }
            let envelope = json::json(&body).map_err(|k| self.error(k))?;
            if let Err(kind) = classify_ocs(status, &envelope, options.account_probe) {
                let mut error = self.error(kind);
                if kind == ErrorKind::RateLimited {
                    error.retry = Retry::AfterDelay {
                        millis: retry_seconds * 1000,
                    };
                }
                return Err(error);
            }
            let mut envelope = envelope;
            let data = envelope
                .get_mut("ocs")
                .and_then(|v| v.get_mut("data"))
                .map(serde_json::Value::take)
                .ok_or_else(|| self.error(ErrorKind::Malformed))?;
            // Commit only a complete, successfully decoded OCS response. A
            // partial body or rejected response cannot advance shared cookies.
            self.update_cookies(response.headers())?;
            Ok(Response {
                status,
                notification_allowed: match response
                    .headers()
                    .get("X-Nextcloud-User-Status")
                    .and_then(|h| h.to_str().ok())
                {
                    Some("online" | "away" | "offline" | "invisible") => Some(true),
                    Some("dnd") => Some(false),
                    _ => None,
                },
                data,
                last_given,
                common_read,
            })
        };
        // The existing core cancellation primitive is atomic, not waker based. Poll only
        // while an admitted request exists; at most 8 timers, no idle task or detached job.
        let cancellation = async {
            loop {
                if cancel.is_cancelled() || context.cancellation.is_cancelled() {
                    break;
                }
                tokio::time::sleep(Duration::from_millis(25)).await;
            }
        };
        let result = tokio::select! {
            biased;
            _ = cancellation => Err(PortError { kind: ErrorKind::Cancelled, account: Some(context.scope.account), retry: Retry::Never }),
            result = tokio::time::timeout(deadline, work) => result.unwrap_or(Err(PortError { kind: ErrorKind::Deadline, account: Some(context.scope.account), retry: Retry::UserAction })),
        };
        self.validate(context)?;
        if result.is_ok() {
            self.commit_shared_cookies(shared_cookies)?;
        }
        result
    }

    fn update_cookies(&mut self, headers: &reqwest::header::HeaderMap) -> Result<(), PortError> {
        for value in headers.get_all(SET_COOKIE) {
            if value.as_bytes().len() > 4096 {
                return Err(self.error(ErrorKind::TooLarge));
            }
            let raw = value
                .to_str()
                .map_err(|_| self.error(ErrorKind::Malformed))?;
            let cookie =
                cookie::Cookie::parse(raw).map_err(|_| self.error(ErrorKind::Malformed))?;
            // No domain cookies, path escape, or secret expansion; accept host-only sessions.
            if cookie.domain().is_some() {
                return Err(self.error(ErrorKind::Unsupported));
            }
            let path = cookie.path().unwrap_or(self.binding.origin.0.path());
            if !path.starts_with('/') || !cookie_path_matches(self.binding.origin.0.path(), path) {
                return Err(self.error(ErrorKind::WrongScope));
            }
            if !cookie
                .name()
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || b"_-".contains(&b))
                || cookie.name().is_empty()
                || cookie
                    .value()
                    .bytes()
                    .any(|b| !(0x21..=0x7e).contains(&b) || b == b';' || b == b',')
            {
                return Err(self.error(ErrorKind::Malformed));
            }
            self.cookies
                .retain(|c| !(c.name == cookie.name() && c.path == path));
            if cookie.max_age().is_some_and(|d| d.whole_seconds() <= 0)
                || cookie.expires_datetime().is_some_and(|d| {
                    d.unix_timestamp() <= cookie::time::OffsetDateTime::now_utc().unix_timestamp()
                })
            {
                continue;
            }
            if self.cookies.len() == 8 {
                return Err(self.error(ErrorKind::TooLarge));
            }
            let expires = cookie.max_age().map(|d| {
                Instant::now() + Duration::from_secs(d.whole_seconds().clamp(0, 86400) as u64)
            });
            self.cookies.push(SessionCookie {
                name: cookie.name().to_owned(),
                value: Zeroizing::new(cookie.value().to_owned()),
                path: path.to_owned(),
                secure: cookie.secure().unwrap_or(false),
                expires,
            });
        }
        Ok(())
    }
}

impl Drop for Transport {
    fn drop(&mut self) {
        if self.cancel_account_on_drop {
            self.binding.cancellation.cancel();
            self.close_meeting();
        }
        #[cfg(any(test, feature = "backend-test"))]
        self.diagnostic_stop();
    }
}

/// NC34.0.5/NC35.0.2 cloud/user rejects unauthenticated callers with
/// HTTP401 + OCS failure/997. Other evidence cannot close an account.
fn classify_ocs(
    status: u16,
    envelope: &serde_json::Value,
    account_probe: bool,
) -> Result<(), ErrorKind> {
    let ocs = envelope.get("ocs").ok_or(ErrorKind::Malformed)?;
    let meta = ocs.get("meta").ok_or(ErrorKind::Malformed)?;
    let code = crate::models::integer(meta.get("statuscode")).ok_or(ErrorKind::Malformed)?;
    let state = meta.get("status").and_then(serde_json::Value::as_str);
    if account_probe && status == 401 {
        let complete = meta
            .get("message")
            .is_some_and(serde_json::Value::is_string)
            && ocs
                .get("data")
                .is_some_and(|data| data.is_array() || data.is_object());
        return Err(
            if state == Some("failure")
                && meta.get("statuscode").and_then(serde_json::Value::as_u64) == Some(997)
                && complete
            {
                ErrorKind::ReauthenticationRequired
            } else {
                ErrorKind::Malformed
            },
        );
    }
    // Scheduled edits and replicated history clearing use documented HTTP/OCS 202.
    // This does not change the authoritative account rejection classifier.
    if !account_probe && status == 202 && code == 202 && state == Some("ok") {
        return Ok(());
    }
    if (200..300).contains(&status) && state == Some("ok") && matches!(code, 100 | 200 | 201 | 204)
    {
        return Ok(());
    }
    let effective = if status >= 400 {
        u64::from(status)
    } else {
        code
    };
    Err(match effective {
        // A v1-style HTTP200 auth code is not authoritative on the pinned v2 route.
        401 | 997 if account_probe => ErrorKind::Malformed,
        401 | 997 | 403 | 412 => ErrorKind::PermissionDenied,
        404 => ErrorKind::NotFound,
        409 => ErrorKind::Conflict,
        413 => ErrorKind::TooLarge,
        429 => ErrorKind::RateLimited,
        500..=599 => ErrorKind::ServerUnavailable,
        _ => ErrorKind::Malformed,
    })
}

fn cookie_path_matches(request: &str, cookie: &str) -> bool {
    request == cookie
        || request
            .strip_prefix(cookie)
            .is_some_and(|rest| cookie.ends_with('/') || rest.starts_with('/'))
}
// Talk uses -1/-2 for unavailable/unread markers. They convey no receipt.
fn header_read_marker(
    headers: &reqwest::header::HeaderMap,
    name: &str,
) -> Result<Option<u64>, ErrorKind> {
    match headers.get(name).and_then(|v| v.to_str().ok()) {
        Some("-1" | "-2") => Ok(None),
        _ => header_number(headers, name),
    }
}
fn header_number(
    headers: &reqwest::header::HeaderMap,
    name: &str,
) -> Result<Option<u64>, ErrorKind> {
    headers
        .get(name)
        .map(|v| {
            v.to_str()
                .ok()
                .and_then(|s| s.parse().ok())
                .ok_or(ErrorKind::Malformed)
        })
        .transpose()
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn unread_common_marker_is_unavailable_without_relaxing_message_cursors() {
        let mut headers = reqwest::header::HeaderMap::new();
        for sentinel in ["-1", "-2"] {
            headers.insert(
                "x-chat-last-common-read",
                HeaderValue::from_static(sentinel),
            );
            assert_eq!(
                header_read_marker(&headers, "x-chat-last-common-read"),
                Ok(None)
            );
            assert_eq!(
                header_number(&headers, "x-chat-last-common-read"),
                Err(ErrorKind::Malformed)
            );
        }
        headers.insert("x-chat-last-common-read", HeaderValue::from_static("-3"));
        assert_eq!(
            header_read_marker(&headers, "x-chat-last-common-read"),
            Err(ErrorKind::Malformed)
        );
        headers.insert("x-chat-last-common-read", HeaderValue::from_static("7"));
        assert_eq!(
            header_read_marker(&headers, "x-chat-last-common-read"),
            Ok(Some(7))
        );
    }
    #[test]
    fn origin_and_routes_fail_closed() {
        let origin = ServerOrigin::https("https://EXAMPLE.org:443/cloud").unwrap();
        assert_eq!(
            origin.endpoint("ocs/v2.php/cloud/user").unwrap().as_str(),
            "https://example.org/cloud/ocs/v2.php/cloud/user"
        );
        for input in [
            "http://example.org",
            "https://user:secret@example.org",
            "https://example.org/a/../b",
            "https://example.org/%2e%2e",
            "https://example.org/?token=secret",
            "https://example.org/#x",
            "https://example.org/\\other",
            " https://example.org",
            "https://example.org.",
        ] {
            assert!(ServerOrigin::https(input).is_err(), "{input}");
        }
        for route in ["/evil", "https://evil.test/path", "../evil", "%2fetc"] {
            assert!(origin.endpoint(route).is_err());
        }
        assert!(ServerOrigin::loopback("http://localhost").is_err());
        assert!(ServerOrigin::loopback("http://127.0.0.2").is_err());
        assert!(ServerOrigin::loopback("http://127.0.0.1:42").is_ok());
        assert!(ServerOrigin::loopback("http://[::1]:42").is_ok());
    }
}
