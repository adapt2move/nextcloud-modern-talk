//! Private default-off diagnostic lanes. No public prepared HTTP or cookie access.
#![allow(
    dead_code,
    reason = "Private coordinated API; diagnostic consumer is a separate owner"
)]
use super::*;
use std::sync::atomic::{AtomicBool, Ordering};

pub(super) struct LoopCharge {
    call: CallKey,
    account: Arc<AccountCharge>,
    _requests: OwnedSemaphorePermit,
    terminated: AtomicBool,
    leave_started: AtomicBool,
    reader_cancel: Cancellation,
    owner_cancel: Cancellation,
}
impl LoopCharge {
    fn stop(&self) {
        self.terminated.store(true, Ordering::Release);
        self.reader_cancel.cancel();
    }
    fn error(&self, kind: ErrorKind) -> PortError {
        error(self.call.account.account, kind)
    }
    fn validate(&self, context: &OperationContext, leave: bool) -> Result<(), PortError> {
        validate(
            &self.account.runtime,
            self.call.account,
            &self.owner_cancel,
            context,
        )?;
        if !leave && self.terminated.load(Ordering::Acquire) {
            return Err(self.error(ErrorKind::Uncertain));
        }
        Ok(())
    }
}
impl Drop for LoopCharge {
    fn drop(&mut self) {
        if let Ok(mut entries) = self.account.runtime.0.diagnostic_loops.lock() {
            for entry in entries.iter_mut() {
                if *entry == Some(self.call.account.account) {
                    *entry = None;
                }
            }
        }
    }
}
#[derive(Clone, Copy, Eq, PartialEq)]
enum Direction {
    Read,
    Write,
}
/// Single-flight capability. Only transport constructs it; no Clone or extraction.
pub(crate) struct DiagnosticLane {
    charge: Arc<LoopCharge>,
    direction: Direction,
}
impl Drop for DiagnosticLane {
    fn drop(&mut self) {
        self.charge.stop();
    }
}

pub(crate) enum DiagnosticOperation<'a> {
    Read { route: &'a str },
    Signal { route: &'a str, messages: &'a str },
    Leave { route: &'a str },
}
pub(crate) struct DiagnosticPrepared {
    // Authenticated request is destroyed before the lifetime charge on abandonment.
    request: Option<reqwest::Request>,
    client: Client,
    origin: url::Origin,
    installation: String,
    context: OperationContext,
    sequence: u64,
    revision: u64,
    limit: Duration,
    leave: bool,
    lane: Option<DiagnosticLane>,
}
pub(crate) struct DiagnosticCompletion {
    result: Result<(Response, Vec<CookieMutation>), PortError>,
    context: OperationContext,
    sequence: u64,
    revision: u64,
    leave: bool,
    lane: Option<DiagnosticLane>,
}
impl DiagnosticCompletion {
    /// Owner rejected its selected peer/room/lease. No jar commit; preserve outbound
    /// capability for terminal leave only, after this local operation has settled.
    pub(crate) fn discard(mut self) -> DiagnosticLane {
        let lane = self.lane.take().expect("owned completion lane");
        lane.charge.stop();
        lane
    }
}

impl Transport {
    pub(crate) fn diagnostic_duplex(
        &mut self,
        context: &OperationContext,
        admission: &mut DiagnosticCallAdmission,
    ) -> Result<(DiagnosticLane, DiagnosticLane), PortError> {
        self.validate(context)?;
        let call = admission.call;
        if call.account != self.scope()
            || call.conversation.host != self.host()
            || !call.is_consistent()
        {
            return Err(self.error(ErrorKind::WrongScope));
        }
        if admission.duplex_started {
            return Err(self.error(ErrorKind::Conflict));
        }
        let charge = {
            let mut entries = self
                .runtime
                .0
                .diagnostic_loops
                .lock()
                .map_err(|_| self.error(ErrorKind::Unconfigured))?;
            if entries.contains(&Some(self.scope().account)) {
                return Err(self.error(ErrorKind::Conflict));
            }
            let entry = entries
                .iter_mut()
                .find(|entry| entry.is_none())
                .ok_or_else(|| self.error(ErrorKind::Busy))?;
            // One atomic nonwaiting reservation from the EXISTING eight permits.
            let requests = self
                .runtime
                .0
                .requests
                .clone()
                .try_acquire_many_owned(2)
                .map_err(|_| self.error(ErrorKind::Busy))?;
            self.validate(context)?;
            let charge = Arc::new(LoopCharge {
                call,
                account: self._account_charge.clone(),
                _requests: requests,
                terminated: AtomicBool::new(false),
                leave_started: AtomicBool::new(false),
                reader_cancel: Cancellation::default(),
                owner_cancel: self.cancellation(),
            });
            *entry = Some(self.scope().account);
            charge
        };
        admission.duplex_started = true;
        self.diagnostic_loop = Arc::downgrade(&charge);
        Ok((
            DiagnosticLane {
                charge: charge.clone(),
                direction: Direction::Read,
            },
            DiagnosticLane {
                charge,
                direction: Direction::Write,
            },
        ))
    }
    /// Immediate local closure. An already dispatched outbound write still settles.
    pub(crate) fn diagnostic_stop(&self) {
        if let Some(charge) = self.diagnostic_loop.upgrade() {
            charge.stop();
        }
    }
    pub(crate) fn diagnostic_prepare(
        &mut self,
        context: &OperationContext,
        call: CallKey,
        lane: DiagnosticLane,
        operation: DiagnosticOperation<'_>,
    ) -> Result<DiagnosticPrepared, PortError> {
        self.validate(context)?;
        if !Arc::ptr_eq(&self._account_charge, &lane.charge.account) || lane.charge.call != call {
            return Err(self.error(ErrorKind::WrongScope));
        }
        let (route, direction, method, messages, leave, limit) = match operation {
            DiagnosticOperation::Read { route } => (
                route,
                Direction::Read,
                Method::GET,
                None,
                false,
                Duration::from_secs(35),
            ),
            DiagnosticOperation::Signal { route, messages } => (
                route,
                Direction::Write,
                Method::POST,
                Some(messages),
                false,
                REQUEST_LIMIT,
            ),
            DiagnosticOperation::Leave { route } => (
                route,
                Direction::Write,
                Method::DELETE,
                None,
                true,
                REQUEST_LIMIT,
            ),
        };
        if lane.direction != direction {
            return Err(self.error(ErrorKind::WrongScope));
        }
        lane.charge.validate(context, leave)?;
        let prefix = if leave {
            "ocs/v2.php/apps/spreed/api/v4/call/"
        } else {
            "ocs/v2.php/apps/spreed/api/v3/signaling/"
        };
        let token = route
            .strip_prefix(prefix)
            .ok_or_else(|| self.error(ErrorKind::WrongScope))?;
        if token.is_empty()
            || token.len() > 128
            || !token.bytes().all(|b| b.is_ascii_alphanumeric())
        {
            return Err(self.error(ErrorKind::Malformed));
        }
        if messages.is_some_and(|m| m.len() > 128 * 1024) {
            return Err(self.error(ErrorKind::TooLarge));
        }
        let mut url = self
            .binding
            .origin
            .endpoint(route)
            .map_err(|k| self.error(k))?;
        url.query_pairs_mut().append_pair("format", "json");
        let mut request = self
            .client
            .request(method, url.clone())
            .timeout(limit)
            .header(ACCEPT, "application/json")
            .header(ACCEPT_ENCODING, "identity")
            .header(CACHE_CONTROL, "no-store, no-cache, max-age=0")
            .header("Pragma", "no-cache")
            .header("OCS-APIRequest", "true")
            .basic_auth(
                self.binding.login_name.as_str(),
                Some(self.binding.password.as_str()),
            );
        let now = Instant::now();
        let mut cookies = Zeroizing::new(String::new());
        for cookie in self
            .cookies
            .iter()
            .filter(|c| c.expires.is_none_or(|e| e > now))
        {
            if (!cookie.secure || url.scheme() == "https")
                && cookie_path_matches(url.path(), &cookie.path)
            {
                if !cookies.is_empty() {
                    cookies.push_str("; ");
                }
                cookies.push_str(&cookie.name);
                cookies.push('=');
                cookies.push_str(&cookie.value);
            }
        }
        if !cookies.is_empty() {
            let mut value =
                HeaderValue::from_str(&cookies).map_err(|_| self.error(ErrorKind::Malformed))?;
            value.set_sensitive(true);
            request = request.header(COOKIE, value);
        }
        if let Some(messages) = messages {
            request = request.form(&[("messages", messages)]);
        }
        let request = request
            .build()
            .map_err(|_| self.error(ErrorKind::Malformed))?;
        if request
            .body()
            .and_then(reqwest::Body::as_bytes)
            .is_some_and(|b| b.len() > BODY_LIMIT)
        {
            return Err(self.error(ErrorKind::TooLarge));
        }
        let sequence = self
            .diagnostic_cookies
            .next
            .checked_add(1)
            .ok_or_else(|| self.error(ErrorKind::Uncertain))?;
        if leave {
            if lane.charge.leave_started.swap(true, Ordering::AcqRel) {
                return Err(self.error(ErrorKind::Conflict));
            }
            lane.charge.stop();
        }
        self.diagnostic_cookies.next = sequence;
        Ok(DiagnosticPrepared {
            request: Some(request),
            client: self.client.clone(),
            origin: url.origin(),
            installation: self.binding.origin.0.path().to_owned(),
            context: copy_context(context),
            sequence,
            revision: self.diagnostic_cookies.revision,
            limit,
            leave,
            lane: Some(lane),
        })
    }
    /// Caller has already validated current active room, selected session/SID and
    /// response lease. A failed commit still returns the lane for terminal handling.
    pub(crate) fn diagnostic_commit(
        &mut self,
        context: &OperationContext,
        call: CallKey,
        mut completion: DiagnosticCompletion,
    ) -> (DiagnosticLane, Result<Response, PortError>) {
        let lane = completion.lane.take().expect("owned completion lane");
        let result = (|| {
            self.validate(context)?;
            if !Arc::ptr_eq(&self._account_charge, &lane.charge.account) || lane.charge.call != call
            {
                return Err(self.error(ErrorKind::WrongScope));
            }
            lane.charge
                .validate(&completion.context, completion.leave)?;
            let (response, delta) = completion.result?;
            self.diagnostic_cookies
                .commit(
                    &mut self.cookies,
                    completion.sequence,
                    completion.revision,
                    delta,
                )
                .map_err(|kind| self.error(kind))?;
            Ok(response)
        })();
        if result.is_err() {
            lane.charge.stop();
        }
        (lane, result)
    }
}
impl DiagnosticPrepared {
    /// No owner borrow across await. Authenticated request and reservations are private.
    pub(crate) async fn wait(mut self) -> DiagnosticCompletion {
        let lane = self.lane.as_ref().expect("owned prepared lane");
        let charge = lane.charge.clone();
        let reader = lane.direction == Direction::Read;
        let request = self.request.take().expect("one request");
        let deadline = self
            .context
            .deadline
            .millis
            .checked_sub(charge.account.runtime.now().millis)
            .map(Duration::from_millis)
            .unwrap_or_default()
            .min(self.limit);
        let result = match charge.validate(&self.context, self.leave) {
            Err(error) => Err(error),
            Ok(()) => {
                let work = receive(
                    &self.client,
                    request,
                    &self.origin,
                    &self.installation,
                    &charge,
                );
                let cancellation = async {
                    loop {
                        if charge.owner_cancel.is_cancelled()
                            || self.context.cancellation.is_cancelled()
                            || (reader && charge.reader_cancel.is_cancelled())
                        {
                            break;
                        }
                        tokio::time::sleep(Duration::from_millis(25)).await;
                    }
                };
                let result = tokio::select! {
                    biased;
                    _=cancellation=>Err(charge.error(ErrorKind::Cancelled)),
                    result=tokio::time::timeout(deadline,work)=>result.unwrap_or_else(|_|Err(charge.error(ErrorKind::Deadline))),
                };
                match validate(
                    &charge.account.runtime,
                    charge.call.account,
                    &charge.owner_cancel,
                    &self.context,
                ) {
                    Err(error) => Err(error),
                    Ok(()) => result,
                }
            }
        };
        if result.is_err() {
            charge.stop();
        }
        DiagnosticCompletion {
            result,
            context: self.context,
            sequence: self.sequence,
            revision: self.revision,
            leave: self.leave,
            lane: self.lane.take(),
        }
    }
}
fn copy_context(c: &OperationContext) -> OperationContext {
    OperationContext {
        scope: c.scope,
        deadline: c.deadline,
        cancellation: c.cancellation.clone(),
    }
}
fn error(account: BindingId, kind: ErrorKind) -> PortError {
    PortError {
        kind,
        account: Some(account),
        retry: match kind {
            ErrorKind::Uncertain => Retry::AfterReconciliation,
            ErrorKind::Transport | ErrorKind::Deadline | ErrorKind::ServerUnavailable => {
                Retry::UserAction
            }
            _ => Retry::Never,
        },
    }
}
fn validate(
    runtime: &NetworkRuntime,
    scope: SessionScope,
    cancel: &Cancellation,
    c: &OperationContext,
) -> Result<(), PortError> {
    let kind = if c.scope != scope {
        Some(ErrorKind::WrongScope)
    } else if cancel.is_cancelled() || c.cancellation.is_cancelled() {
        Some(ErrorKind::Cancelled)
    } else {
        let now = runtime.now();
        if c.deadline.epoch != now.epoch || c.deadline.millis <= now.millis {
            Some(ErrorKind::Deadline)
        } else {
            None
        }
    };
    kind.map_or(Ok(()), |kind| Err(error(scope.account, kind)))
}
async fn receive(
    client: &Client,
    request: reqwest::Request,
    origin: &url::Origin,
    installation: &str,
    charge: &LoopCharge,
) -> Result<(Response, Vec<CookieMutation>), PortError> {
    let err = |kind| charge.error(kind);
    let mut response = client.execute(request).await.map_err(|e| {
        err(if e.is_timeout() {
            ErrorKind::Deadline
        } else {
            ErrorKind::Transport
        })
    })?;
    if &response.url().origin() != origin {
        return Err(err(ErrorKind::WrongScope));
    }
    let status = response.status().as_u16();
    if (300..400).contains(&status) {
        return Err(err(ErrorKind::RedirectRejected));
    }
    if response
        .headers()
        .get_all(CONTENT_ENCODING)
        .iter()
        .any(|h| h.as_bytes() != b"identity")
    {
        return Err(err(ErrorKind::Unsupported));
    }
    if response
        .content_length()
        .is_some_and(|n| n > BODY_LIMIT as u64)
    {
        return Err(err(ErrorKind::TooLarge));
    }
    let content_type = response
        .headers()
        .get(CONTENT_TYPE)
        .and_then(|h| h.to_str().ok())
        .unwrap_or("")
        .split(';')
        .next()
        .unwrap_or("");
    if content_type != "application/json" {
        return Err(err(ErrorKind::Malformed));
    }
    let last_given = header_number(response.headers(), "X-Chat-Last-Given").map_err(err)?;
    let common_read = header_number(response.headers(), "X-Chat-Last-Common-Read").map_err(err)?;
    let retry = header_number(response.headers(), "Retry-After")
        .ok()
        .flatten()
        .map(|n| n.clamp(1, 300) as u32)
        .unwrap_or(30);
    let delta = cookie_delta(
        response.headers(),
        installation,
        Instant::now(),
        cookie::time::OffsetDateTime::now_utc().unix_timestamp(),
    )
    .map_err(err)?;
    let mut body = Zeroizing::new(Vec::with_capacity(8192));
    while let Some(chunk) = response
        .chunk()
        .await
        .map_err(|_| err(ErrorKind::Transport))?
    {
        if chunk.len() > BODY_LIMIT - body.len() {
            return Err(err(ErrorKind::TooLarge));
        }
        body.extend_from_slice(&chunk);
    }
    let mut envelope = crate::json::json(&body).map_err(err)?;
    if let Err(kind) = classify_ocs(status, &envelope, false) {
        let mut error = err(kind);
        if kind == ErrorKind::RateLimited {
            error.retry = Retry::AfterDelay {
                millis: retry * 1000,
            };
        }
        return Err(error);
    }
    let data = envelope
        .get_mut("ocs")
        .and_then(|v| v.get_mut("data"))
        .map(serde_json::Value::take)
        .ok_or_else(|| err(ErrorKind::Malformed))?;
    Ok((
        Response {
            status,
            notification_allowed: None,
            data,
            last_given,
            common_read,
        },
        delta,
    ))
}

#[derive(Clone, PartialEq, Eq)]
enum Expiry {
    Session,
    Relative(Instant),
    Absolute(i64),
    Delete,
}
#[derive(Clone, PartialEq, Eq)]
struct CookieMutation {
    name: String,
    path: String,
    value: Zeroizing<String>,
    secure: bool,
    http_only: bool,
    same_site: Option<cookie::SameSite>,
    partitioned: bool,
    expiry: Expiry,
}
#[derive(Default)]
pub(super) struct CookieOrder {
    next: u64,
    revision: u64,
    last_sequence: u64,
    last_delta: Vec<CookieMutation>,
}
impl CookieOrder {
    fn commit(
        &mut self,
        jar: &mut Vec<SessionCookie>,
        sequence: u64,
        revision: u64,
        delta: Vec<CookieMutation>,
    ) -> Result<(), ErrorKind> {
        if revision != self.revision {
            if revision.checked_add(1) == Some(self.revision)
                && !delta.is_empty()
                && delta == self.last_delta
            {
                return Ok(());
            }
            return Err(ErrorKind::Uncertain);
        }
        if delta.is_empty() {
            return Ok(());
        }
        if sequence <= self.last_sequence {
            return Err(ErrorKind::Uncertain);
        }
        let revision = self.revision.checked_add(1).ok_or(ErrorKind::Uncertain)?;
        let now = Instant::now();
        let wall = cookie::time::OffsetDateTime::now_utc().unix_timestamp();
        let mut next: Vec<_> = jar
            .iter()
            .filter(|c| c.expires.is_none_or(|e| e > now))
            .cloned()
            .collect();
        for change in &delta {
            next.retain(|c| c.name != change.name || c.path != change.path);
            let expires = match change.expiry {
                Expiry::Delete => continue,
                Expiry::Session => None,
                Expiry::Relative(at) if at <= now => continue,
                Expiry::Relative(at) => Some(at),
                Expiry::Absolute(at) if at <= wall => continue,
                Expiry::Absolute(at) => {
                    Some(now + Duration::from_secs(at.saturating_sub(wall).min(86400) as u64))
                }
            };
            if next.len() == 8 {
                return Err(ErrorKind::TooLarge);
            }
            next.push(SessionCookie {
                name: change.name.clone(),
                path: change.path.clone(),
                value: change.value.clone(),
                secure: change.secure,
                expires,
            });
        }
        *jar = next;
        self.revision = revision;
        self.last_sequence = sequence;
        self.last_delta = delta;
        Ok(())
    }
}
fn cookie_delta(
    headers: &reqwest::header::HeaderMap,
    installation: &str,
    now: Instant,
    wall: i64,
) -> Result<Vec<CookieMutation>, ErrorKind> {
    let mut delta = Vec::new();
    for value in headers.get_all(SET_COOKIE) {
        if delta.len() == 8 || value.as_bytes().len() > 4096 {
            return Err(ErrorKind::TooLarge);
        }
        let raw = value.to_str().map_err(|_| ErrorKind::Malformed)?;
        // Explicit finite attribute vocabulary and no duplicate ambiguity. Values
        // are parsed by the pinned cookie crate, never manually interpreted as auth.
        let mut attributes = Vec::new();
        for attribute in raw.split(';').skip(1) {
            let name = attribute
                .trim()
                .split('=')
                .next()
                .unwrap_or("")
                .trim()
                .to_ascii_lowercase();
            if matches!(name.as_str(), "secure" | "httponly" | "partitioned")
                && attribute.contains('=')
            {
                return Err(ErrorKind::Malformed);
            }
            if !matches!(
                name.as_str(),
                "path"
                    | "secure"
                    | "httponly"
                    | "samesite"
                    | "partitioned"
                    | "max-age"
                    | "expires"
                    | "domain"
            ) {
                return Err(ErrorKind::Unsupported);
            }
            if attributes.contains(&name) {
                return Err(ErrorKind::Malformed);
            }
            attributes.push(name);
        }
        let cookie = cookie::Cookie::parse(raw).map_err(|_| ErrorKind::Malformed)?;
        if cookie.domain().is_some() {
            return Err(ErrorKind::Unsupported);
        }
        let path = cookie.path().unwrap_or(installation);
        if !path.starts_with('/') || !cookie_path_matches(installation, path) {
            return Err(ErrorKind::WrongScope);
        }
        if cookie.name().is_empty()
            || !cookie
                .name()
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || b"_-".contains(&b))
            || cookie
                .value()
                .bytes()
                .any(|b| !(0x21..=0x7e).contains(&b) || b == b';' || b == b',')
        {
            return Err(ErrorKind::Malformed);
        }
        let expiry = if let Some(age) = cookie.max_age() {
            if age.whole_seconds() <= 0 {
                Expiry::Delete
            } else {
                Expiry::Relative(now + Duration::from_secs(age.whole_seconds().min(86400) as u64))
            }
        } else if let Some(at) = cookie.expires_datetime() {
            if at.unix_timestamp() <= wall {
                Expiry::Delete
            } else {
                Expiry::Absolute(at.unix_timestamp())
            }
        } else {
            Expiry::Session
        };
        if attributes.iter().any(|a| a == "max-age") && cookie.max_age().is_none()
            || attributes.iter().any(|a| a == "expires") && cookie.expires_datetime().is_none()
            || attributes.iter().any(|a| a == "samesite") && cookie.same_site().is_none()
        {
            return Err(ErrorKind::Malformed);
        }
        let value = if expiry == Expiry::Delete {
            String::new()
        } else {
            cookie.value().to_owned()
        };
        let mutation = CookieMutation {
            name: cookie.name().to_owned(),
            path: path.to_owned(),
            value: Zeroizing::new(value),
            secure: cookie.secure().unwrap_or(false),
            http_only: cookie.http_only().unwrap_or(false),
            same_site: cookie.same_site(),
            partitioned: cookie.partitioned().unwrap_or(false),
            expiry,
        };
        if delta
            .iter()
            .any(|c: &CookieMutation| c.name == mutation.name && c.path == mutation.path)
        {
            return Err(ErrorKind::Malformed);
        }
        delta.push(mutation);
    }
    delta.sort_unstable_by(|a, b| a.name.cmp(&b.name).then(a.path.cmp(&b.path)));
    Ok(delta)
}

#[cfg(test)]
mod tests;
