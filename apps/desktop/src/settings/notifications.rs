//! Bounded notification owner: one worker, one slot per priority, 16 correlated completions.
//! Retains at most 8 current account source observations and 64 accepted event identities;
//! queued ordinary alerts expire after 5 seconds, calls after 45 seconds.
//! One separate authorization slot uses this same worker, without an idle timer.
//! removed sources are retired and old identities evicted in order. Participation bursts coalesce
//! per call for 5 seconds. Call cancellation stops queued/running helpers, but cannot
//! withdraw a banner already submitted through these one-shot native helper routes.
//! Sources and the current direct invitation are admitted composition observations, not login.
//! All categories use ordinary OS notification interruption, including sound.
//! No direct audio player, critical urgency, time-sensitive entitlement, or CallKit UI.
//! macOS actions exist only while the bounded helper and admitted dispatcher are live.
use std::{
    collections::VecDeque,
    path::{Path, PathBuf},
    process::{Command, Stdio},
    sync::{Arc, Condvar, Mutex, MutexGuard},
    time::{Duration, Instant},
};
use talk_core::{
    ids::{AccountScope, CallScope},
    preferences::{
        Locale, NotificationPreferences, RequestId,
        notifications::{
            self, BannerBackend, DeliveryOutcome, NativeCallAction, NativeCallActionCompletion,
            NativeCallActions, NativeNotificationCapabilities, NotificationAccessAction,
            NotificationAccessOperation, NotificationAccessSnapshot, NotificationActivation,
            NotificationAuthorization, NotificationCategory, NotificationCompletion,
            NotificationError, NotificationEvent, NotificationKind, NotificationSources,
            NotificationState, SoundBackend, SourceAvailability,
        },
    },
};

mod access;
mod activation;
mod macos;

#[cfg(test)]
mod tests;

const COMPLETIONS: usize = 16;
const SOURCES: usize = 8;
const RECENT: usize = 64;
const JOIN_WINDOW: Duration = Duration::from_secs(5);
struct Running {
    event: NotificationEvent,
    cancelled: bool,
}
struct State {
    policy: NotificationPreferences,
    locale: Locale,
    actions: VecDeque<NativeCallActionCompletion>,
    activations: VecDeque<NotificationActivation>,
    activation_hosts: usize,
    capabilities: NativeNotificationCapabilities,
    pending: [Option<NotificationEvent>; 3],
    pending_at: [Option<Instant>; 3],
    running: Option<Running>,
    sources: Vec<NotificationSources>,
    recent: VecDeque<NotificationKind>,
    joins: VecDeque<(CallScope, Instant)>,
    completions: VecDeque<NotificationCompletion>,
    last_request: RequestId,
    access: NotificationAccessSnapshot,
    access_pending: Option<access::Command>,
    last_access_request: RequestId,
    stopped: bool,
    result: NotificationState,
}
impl State {
    fn new(policy: NotificationPreferences) -> Self {
        Self {
            policy,
            locale: Locale::English,
            actions: VecDeque::new(),
            activations: VecDeque::new(),
            activation_hosts: 0,
            capabilities: NativeNotificationCapabilities {
                banners: BannerBackend::Unavailable,
                sounds: SoundBackend::Unavailable,
                native_call_ui: false,
                native_call_actions: false,
            },
            pending: [None, None, None],
            pending_at: [None, None, None],
            running: None,
            sources: Vec::new(),
            recent: VecDeque::new(),
            joins: VecDeque::new(),
            completions: VecDeque::new(),
            last_request: RequestId(0),
            access: NotificationAccessSnapshot::unknown(),
            access_pending: None,
            last_access_request: RequestId(0),
            stopped: false,
            result: NotificationState::Idle,
        }
    }
    fn source(
        &self,
        scope: Option<AccountScope>,
        category: NotificationCategory,
    ) -> SourceAvailability {
        let Some(scope) = scope else {
            return SourceAvailability::Unavailable;
        };
        self.sources
            .iter()
            .find(|s| s.scope == scope)
            .map_or(SourceAvailability::Unknown, |s| s.availability(category))
    }
    fn eligible_source(
        &self,
        event: &NotificationEvent,
    ) -> Result<SourceAvailability, NotificationError> {
        if !event.kind.is_consistent() {
            return Err(NotificationError::Stale);
        }
        // Only explicit non-Calendar tests bypass unavailable production sources.
        if matches!(event.kind, NotificationKind::Test { category, .. } if category != NotificationCategory::Calendar)
        {
            return Ok(SourceAvailability::Available);
        }
        let source = self.source(event.kind.scope(), event.kind.category());
        if source != SourceAvailability::Available {
            return Err(NotificationError::SourceUnavailable);
        }
        if let NotificationKind::IncomingCall { call } = event.kind {
            let current = self
                .sources
                .iter()
                .find(|s| s.scope == call.account)
                .and_then(|s| s.ringing);
            if current != Some(call) {
                return Err(NotificationError::Stale);
            }
        }
        Ok(source)
    }
    fn finish(
        &mut self,
        event: &NotificationEvent,
        banner: DeliveryOutcome,
        sound: DeliveryOutcome,
    ) {
        for lane in 0..3 {
            if self.pending[lane].is_none() {
                self.pending_at[lane] = None;
            }
        }
        // Admission reserves one completion for every accepted pending/running event.
        self.completions.push_back(NotificationCompletion {
            request: event.request,
            category: event.kind.category(),
            banner,
            sound,
        });
        self.result = NotificationState::Finished {
            request: event.request,
            banner,
            sound,
        };
    }
    fn admit(&mut self, event: NotificationEvent, now: Instant) -> Result<(), NotificationError> {
        if self.stopped {
            return Err(NotificationError::Closed);
        }
        if event.request <= self.last_request {
            return Err(NotificationError::Stale);
        }
        self.eligible_source(&event)?;
        // An explicit first-authorization sheet cannot turn queued old events
        // into fresh alerts after a long decision. Composition settles rejection.
        if matches!(
            self.access.operation,
            NotificationAccessOperation::Pending {
                action: NotificationAccessAction::Request,
                ..
            }
        ) {
            return Err(NotificationError::PermissionDenied);
        }
        if !matches!(event.kind, NotificationKind::Test { .. }) && self.recent.contains(&event.kind)
        {
            return Err(NotificationError::Duplicate);
        }
        self.joins
            .retain(|(_, time)| now.saturating_duration_since(*time) < JOIN_WINDOW);
        if let NotificationKind::ConferenceParticipantJoined {
            call,
            is_self: false,
            ..
        } = event.kind
            && self.joins.iter().any(|(prior, _)| *prior == call)
        {
            return Err(NotificationError::Coalesced);
        }
        let lane = lane(event.kind.category());
        let outstanding = self.completions.len()
            + self.pending.iter().filter(|p| p.is_some()).count()
            + usize::from(self.running.is_some());
        // Ordinary/important work cannot consume the final completion reserved for a call.
        let capacity = if lane == 2 {
            COMPLETIONS
        } else {
            COMPLETIONS - 1
        };
        if self.pending[lane].is_some() || outstanding >= capacity {
            return Err(NotificationError::Busy);
        }
        if !matches!(event.kind, NotificationKind::Test { .. }) {
            if self.recent.len() == RECENT {
                self.recent.pop_front();
            }
            self.recent.push_back(event.kind);
        }
        if let NotificationKind::ConferenceParticipantJoined {
            call,
            is_self: false,
            ..
        } = event.kind
        {
            if self.joins.len() == RECENT {
                self.joins.pop_front();
            }
            self.joins.push_back((call, now));
        }
        if lane == 2
            && let Some(running) = &mut self.running
            && lane_of(&running.event) < 2
        {
            running.cancelled = true;
        }
        self.last_request = event.request;
        self.result = NotificationState::Pending(event.request);
        self.pending[lane] = Some(event);
        self.pending_at[lane] = Some(now);
        Ok(())
    }
    fn enter_foreground(&mut self) -> bool {
        let mut changed = false;
        for lane in 0..3 {
            if self.pending[lane]
                .as_ref()
                .is_some_and(|e| matches!(e.kind, NotificationKind::Message { .. }))
                && let Some(event) = self.pending[lane].take()
            {
                self.finish(
                    &event,
                    DeliveryOutcome::Cancelled,
                    DeliveryOutcome::Cancelled,
                );
                changed = true;
            }
        }
        if let Some(running) = &mut self.running
            && !running.cancelled
            && matches!(running.event.kind, NotificationKind::Message { .. })
        {
            running.cancelled = true;
            changed = true;
        }
        changed
    }
    fn next(&mut self) -> Option<NotificationEvent> {
        for lane in (0..3).rev() {
            let Some(event) = self.pending[lane].take() else {
                continue;
            };
            let lifetime = Duration::from_secs(if lane == 2 { 45 } else { 5 });
            if self.pending_at[lane]
                .take()
                .is_none_or(|at| at.elapsed() >= lifetime)
            {
                self.finish(
                    &event,
                    DeliveryOutcome::Cancelled,
                    DeliveryOutcome::Cancelled,
                );
                continue;
            }
            self.running = Some(Running {
                event: event.clone(),
                cancelled: false,
            });
            return Some(event);
        }
        None
    }
    fn cancel_obsolete(&mut self) {
        for lane in 0..3 {
            if let Some(event) = self.pending[lane].take() {
                if self.eligible_source(&event).is_err() {
                    self.finish(
                        &event,
                        DeliveryOutcome::Cancelled,
                        DeliveryOutcome::Cancelled,
                    );
                } else {
                    self.pending[lane] = Some(event);
                }
            }
        }
        let obsolete = self
            .running
            .as_ref()
            .is_some_and(|r| self.eligible_source(&r.event).is_err());
        if obsolete && let Some(running) = &mut self.running {
            running.cancelled = true;
        }
    }
    fn retire_source(&mut self, scope: AccountScope) -> Result<(), NotificationError> {
        if self.stopped {
            return Err(NotificationError::Closed);
        }
        let index = self
            .sources
            .iter()
            .position(|source| source.scope == scope)
            .ok_or(NotificationError::Stale)?;
        self.sources.remove(index);
        self.cancel_obsolete();
        Ok(())
    }
    fn update_sources(
        &mut self,
        expected: Option<AccountScope>,
        values: NotificationSources,
    ) -> Result<(), NotificationError> {
        if self.stopped {
            return Err(NotificationError::Closed);
        }
        if values
            .ringing
            .is_some_and(|call| !call.is_consistent() || call.account != values.scope)
        {
            return Err(NotificationError::Stale);
        }
        let has_source_capacity = self.sources.len() < SOURCES;
        match self
            .sources
            .iter_mut()
            .find(|s| s.scope.account == values.scope.account)
        {
            Some(current) => {
                if expected != Some(current.scope)
                    || current.scope == values.scope && values.revision <= current.revision
                {
                    return Err(NotificationError::Stale);
                }
                *current = values;
            }
            None if expected.is_none() && has_source_capacity => self.sources.push(values),
            None if expected.is_some() => return Err(NotificationError::Stale),
            None => return Err(NotificationError::Busy),
        }
        self.cancel_obsolete();
        Ok(())
    }
}
fn lane(category: NotificationCategory) -> usize {
    match category {
        NotificationCategory::Messages | NotificationCategory::Calendar => 0,
        NotificationCategory::Important => 1,
        NotificationCategory::IncomingCalls => 2,
    }
}
fn lane_of(event: &NotificationEvent) -> usize {
    lane(event.kind.category())
}
struct Shared {
    state: Mutex<State>,
    wake: Condvar,
}
impl Shared {
    fn lock(&self) -> MutexGuard<'_, State> {
        self.state.lock().unwrap_or_else(|e| e.into_inner())
    }
}
/// Implement only in the admitted account/call owner. Both methods recheck the
/// current account runtime, invitation and configured media action support.
/// Ringing and this trait never grant capture authority by themselves.
pub trait NotificationCallDispatcher: Send + Sync {
    fn available_actions(&self, call: CallScope) -> NativeCallActions;
    fn dispatch(&self, call: CallScope, action: NativeCallAction) -> Result<(), NotificationError>;
}
pub struct NotificationRuntime {
    shared: Arc<Shared>,
}
impl NotificationRuntime {
    pub fn start(policy: NotificationPreferences) -> Result<Self, NotificationError> {
        Self::start_with_dispatcher(policy, None)
    }
    pub fn start_with_dispatcher(
        policy: NotificationPreferences,
        dispatcher: Option<Arc<dyn NotificationCallDispatcher>>,
    ) -> Result<Self, NotificationError> {
        let shared = Arc::new(Shared {
            state: Mutex::new(State::new(policy)),
            wake: Condvar::new(),
        });
        let worker = Arc::clone(&shared);
        std::thread::Builder::new()
            .name("talk-notifications".into())
            .spawn(move || run(worker, dispatcher))
            .map_err(|_| NotificationError::Unavailable)?;
        Ok(Self { shared })
    }
    pub fn update_locale(&self, locale: Locale) {
        self.shared.lock().locale = locale;
    }
    pub fn update_visibility(&self, background: bool) {
        if !background && self.shared.lock().enter_foreground() {
            self.shared.wake.notify_one();
        }
    }
    pub fn take_action_completion(&self) -> Option<NativeCallActionCompletion> {
        self.shared.lock().actions.pop_front()
    }
    pub fn take_message_activation(&self) -> Option<NotificationActivation> {
        self.shared.lock().activations.pop_front()
    }
    pub fn update_policy(&self, policy: NotificationPreferences) {
        self.shared.lock().policy = policy;
        self.shared.wake.notify_one();
    }
    pub fn snapshot(&self) -> NotificationState {
        self.shared.lock().result
    }
    pub fn access_snapshot(&self) -> NotificationAccessSnapshot {
        self.shared.lock().access
    }
    pub fn submit_access(
        &self,
        request: RequestId,
        action: NotificationAccessAction,
    ) -> Result<(), NotificationError> {
        access::admit(&mut self.shared.lock(), request, action, Instant::now())?;
        self.shared.wake.notify_one();
        Ok(())
    }
    pub fn take_completion(&self) -> Option<NotificationCompletion> {
        self.shared.lock().completions.pop_front()
    }
    pub fn source_availability(
        &self,
        scope: Option<AccountScope>,
        category: NotificationCategory,
    ) -> SourceAvailability {
        self.shared.lock().source(scope, category)
    }
    /// Only the admitted account owner calls this with fresh, increasing source observations.
    /// Exact expected scope prevents a late previous-runtime completion reopening admission.
    pub fn update_sources(
        &self,
        expected_scope: Option<AccountScope>,
        values: NotificationSources,
    ) -> Result<(), NotificationError> {
        self.shared.lock().update_sources(expected_scope, values)?;
        self.shared.wake.notify_one();
        Ok(())
    }
    pub fn retire_source(&self, scope: AccountScope) -> Result<(), NotificationError> {
        self.shared.lock().retire_source(scope)?;
        self.shared.wake.notify_one();
        Ok(())
    }
    pub fn native_capabilities(&self) -> NativeNotificationCapabilities {
        self.shared.lock().capabilities
    }
    pub fn submit(&self, event: NotificationEvent) -> Result<(), NotificationError> {
        if event.title.as_str().chars().any(char::is_control)
            || event
                .body
                .as_str()
                .chars()
                .any(|c| c.is_control() && c != '\n')
        {
            return Err(NotificationError::InvalidContent);
        }
        self.shared.lock().admit(event, Instant::now())?;
        self.shared.wake.notify_one();
        Ok(())
    }
    #[allow(
        dead_code,
        reason = "Admitted current-call lifecycle composition is not integrated yet."
    )]
    pub fn end_call(&self, call: CallScope) -> Result<(), NotificationError> {
        let mut state = self.shared.lock();
        if state.stopped {
            return Err(NotificationError::Closed);
        }
        let source = state
            .sources
            .iter_mut()
            .find(|s| s.scope == call.account)
            .ok_or(NotificationError::Stale)?;
        if source.ringing != Some(call) {
            return Err(NotificationError::Stale);
        }
        source.ringing = None;
        state.cancel_obsolete();
        self.shared.wake.notify_one();
        Ok(())
    }
    pub fn request_shutdown(&self) {
        let mut state = self.shared.lock();
        state.stopped = true;
        access::cancel(&mut state);
        for lane in 0..3 {
            if let Some(event) = state.pending[lane].take() {
                state.finish(
                    &event,
                    DeliveryOutcome::Cancelled,
                    DeliveryOutcome::Cancelled,
                );
            }
        }
        if let Some(running) = &mut state.running {
            running.cancelled = true;
        }
        self.shared.wake.notify_one();
    }
}
impl Drop for NotificationRuntime {
    fn drop(&mut self) {
        self.request_shutdown();
    }
}
fn native_capabilities() -> NativeNotificationCapabilities {
    let mac = cfg!(target_os = "macos") && super::native_process::mac_helper().is_ok();
    NativeNotificationCapabilities {
        banners: if cfg!(target_os = "linux") {
            BannerBackend::Freedesktop
        } else if mac {
            BannerBackend::UserNotifications
        } else {
            BannerBackend::Unavailable
        },
        sounds: if cfg!(target_os = "linux") || mac {
            SoundBackend::SystemNotification
        } else {
            SoundBackend::Unavailable
        },
        native_call_ui: false,
        // This describes the helper route only; actual actions require the dispatcher.
        native_call_actions: mac,
    }
}
fn run(shared: Arc<Shared>, dispatcher: Option<Arc<dyn NotificationCallDispatcher>>) {
    let mut capabilities = native_capabilities();
    capabilities.native_call_actions &= dispatcher.is_some();
    shared.lock().capabilities = capabilities;
    enum Work {
        Event(NotificationEvent),
        Access(access::Command),
    }
    loop {
        let work = {
            let mut state = shared.lock();
            while state.pending.iter().all(Option::is_none)
                && state.access_pending.is_none()
                && !state.stopped
            {
                state = shared.wake.wait(state).unwrap_or_else(|e| e.into_inner());
            }
            if state.stopped {
                state.result = NotificationState::Stopped;
                return;
            }
            if state.pending[2].is_some() {
                state.next().map(Work::Event)
            } else if let Some(command) = state.access_pending.take() {
                Some(Work::Access(command))
            } else {
                state.next().map(Work::Event)
            }
        };
        let event = match work {
            Some(Work::Event(event)) => event,
            Some(Work::Access(command)) => {
                let result = access::perform(&shared, command);
                access::complete(&mut shared.lock(), command, result);
                continue;
            }
            None => continue,
        };
        let (banner, sound) = deliver_pair(&shared, &event, dispatcher.as_deref());
        let mut state = shared.lock();
        state.running = None;
        state.finish(&event, banner, sound);
    }
}
#[derive(Clone, Copy)]
enum Effect {
    Banner,
    Sound,
}
fn check(
    shared: &Shared,
    event: &NotificationEvent,
    effect: Effect,
) -> Result<bool, NotificationError> {
    let state = shared.lock();
    if state.stopped
        || state
            .running
            .as_ref()
            .is_none_or(|r| r.event.request != event.request || r.cancelled)
    {
        return Err(NotificationError::Cancelled);
    }
    let source = state.eligible_source(event)?;
    let plan = notifications::plan(state.policy, event, source);
    Ok(match effect {
        Effect::Banner => plan.banner,
        Effect::Sound => plan.sound,
    })
}
fn outcome(result: Result<(), NotificationError>) -> DeliveryOutcome {
    match result {
        Ok(()) => DeliveryOutcome::SubmittedToSystem,
        Err(
            NotificationError::Cancelled
            | NotificationError::Stale
            | NotificationError::SourceUnavailable,
        ) => DeliveryOutcome::Cancelled,
        Err(error) => DeliveryOutcome::Failed(error),
    }
}
fn deliver_pair(
    shared: &Arc<Shared>,
    event: &NotificationEvent,
    dispatcher: Option<&dyn NotificationCallDispatcher>,
) -> (DeliveryOutcome, DeliveryOutcome) {
    let banner = check(shared, event, Effect::Banner);
    let sound = check(shared, event, Effect::Sound);
    let (banner, sound) = match (banner, sound) {
        (Ok(banner), Ok(sound)) => (banner, sound),
        (Err(error), _) | (_, Err(error)) => return (outcome(Err(error)), outcome(Err(error))),
    };
    if !banner && !sound {
        return (DeliveryOutcome::Suppressed, DeliveryOutcome::Suppressed);
    }
    if banner
        && matches!(event.kind, NotificationKind::MessageReminder { .. })
        && let Some(delivery) = activation::deliver(shared, event, sound)
    {
        return delivery;
    }
    if cfg!(target_os = "macos") {
        return macos::deliver(shared, event, banner, sound, dispatcher);
    }
    if cfg!(target_os = "linux") {
        // Freedesktop has no portable per-notification banner suppression for a
        // sound-only notice. Do not bypass Focus with a direct audio subprocess.
        if !banner {
            return (
                DeliveryOutcome::Suppressed,
                DeliveryOutcome::Failed(NotificationError::Unavailable),
            );
        }
        let sound_file = sound
            .then(|| linux_sound_file(event.kind))
            .flatten()
            .filter(|_| linux_sound_supported(shared, event));
        let result = execute(
            shared,
            event,
            Effect::Banner,
            &mut linux_command(event, sound_file.as_deref()),
        );
        let banner_result = outcome(result);
        let sound_result = if !sound {
            DeliveryOutcome::Suppressed
        } else if sound_file.is_none() {
            DeliveryOutcome::Failed(NotificationError::Unavailable)
        } else {
            banner_result
        };
        return (banner_result, sound_result);
    }
    (
        if banner {
            DeliveryOutcome::Failed(NotificationError::Unavailable)
        } else {
            DeliveryOutcome::Suppressed
        },
        if sound {
            DeliveryOutcome::Failed(NotificationError::Unavailable)
        } else {
            DeliveryOutcome::Suppressed
        },
    )
}
fn linux_sound_supported(shared: &Shared, event: &NotificationEvent) -> bool {
    linux_daemon_supports(shared, event, "sound")
}
fn linux_daemon_supports(shared: &Shared, event: &NotificationEvent, capability: &str) -> bool {
    let result = super::native_process::capture(
        Command::new("gdbus").args([
            "call",
            "--session",
            "--dest",
            "org.freedesktop.Notifications",
            "--object-path",
            "/org/freedesktop/Notifications",
            "--method",
            "org.freedesktop.Notifications.GetCapabilities",
        ]),
        Duration::from_secs(2),
        || {
            !check(
                shared,
                event,
                if capability == "sound" {
                    Effect::Sound
                } else {
                    Effect::Banner
                },
            )
            .unwrap_or(false)
        },
    );
    result.is_ok_and(|output| output.split(['\'', '"']).any(|part| part == capability))
}
fn execute(
    shared: &Shared,
    event: &NotificationEvent,
    effect: Effect,
    command: &mut Command,
) -> Result<(), NotificationError> {
    if !check(shared, event, effect)? {
        return Err(NotificationError::Cancelled);
    }
    let mut child = command
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .map_err(|error| match error.kind() {
            std::io::ErrorKind::NotFound => NotificationError::Unavailable,
            std::io::ErrorKind::PermissionDenied => NotificationError::PermissionDenied,
            _ => NotificationError::Failed,
        })?;
    let deadline = Instant::now() + Duration::from_secs(3);
    loop {
        match child.try_wait() {
            Ok(Some(status)) => {
                return if status.success() {
                    Ok(())
                } else {
                    Err(NotificationError::Failed)
                };
            }
            Err(_) => {
                let _ = child.kill();
                let _ = child.wait();
                return Err(NotificationError::Failed);
            }
            Ok(None) => {}
        }
        let permitted = check(shared, event, effect).unwrap_or(false);
        if !permitted || Instant::now() >= deadline {
            let _ = child.kill();
            let _ = child.wait();
            return Err(if !permitted {
                NotificationError::Cancelled
            } else {
                NotificationError::TimedOut
            });
        }
        std::thread::sleep(Duration::from_millis(20));
    }
}
fn linux_category(kind: NotificationKind) -> &'static str {
    match kind {
        NotificationKind::Message { .. }
        | NotificationKind::ServerTest { .. }
        | NotificationKind::Test {
            category: NotificationCategory::Messages,
            ..
        } => "im.received",
        NotificationKind::IncomingCall { .. }
        | NotificationKind::Test {
            category: NotificationCategory::IncomingCalls,
            ..
        } => "call.incoming",
        NotificationKind::ConferenceParticipantJoined { .. } => {
            "x-nextcloud-modern-talk.conference.joined"
        }
        NotificationKind::MessageReminder { .. }
        | NotificationKind::Test {
            category: NotificationCategory::Important,
            ..
        } => "x-nextcloud-modern-talk.message.reminder",
        NotificationKind::Calendar { .. }
        | NotificationKind::Test {
            category: NotificationCategory::Calendar,
            ..
        } => "x-nextcloud-modern-talk.calendar.reminder",
    }
}
#[cfg(test)]
fn linux_banner_command(event: &NotificationEvent) -> Command {
    linux_command(event, None)
}
fn linux_sound_file(kind: NotificationKind) -> Option<PathBuf> {
    let executable = std::env::current_exe().ok()?;
    let binary_dir = executable.parent()?;
    // Both the tar and Debian packages retain this layout. Missing resources
    // are unavailable, including an unpackaged executable; never play directly.
    if binary_dir.file_name()? != "bin" {
        return None;
    }
    let file = if kind.category() == NotificationCategory::IncomingCalls {
        "call-ringtone.wav"
    } else {
        "message-notification.wav"
    };
    let path = binary_dir
        .parent()?
        .join("share/nextcloud-modern-talk/assets/sounds")
        .join(file);
    path.is_file().then_some(path)
}
fn linux_command(event: &NotificationEvent, sound_file: Option<&Path>) -> Command {
    linux_command_with_action(event, sound_file, None)
}
fn linux_command_with_action(
    event: &NotificationEvent,
    sound_file: Option<&Path>,
    action: Option<&str>,
) -> Command {
    let body = event
        .body
        .as_str()
        .replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;");
    let mut command = Command::new("notify-send");
    command.args([
        "--app-name=NextCloud Modern Talk",
        "--urgency=normal",
        "--expire-time=5000",
        "--category",
        linux_category(event.kind),
        if sound_file.is_some() {
            "--hint=boolean:suppress-sound:false"
        } else {
            "--hint=boolean:suppress-sound:true"
        },
    ]);
    if let Some(path) = sound_file {
        // The notification daemon owns playback and Focus/DND suppression.
        // Its advertised "sound" capability covers this standard file hint.
        let mut hint = std::ffi::OsString::from("--hint=string:sound-file:");
        hint.push(path);
        command.arg(hint);
    }
    if let Some(action) = action {
        command.arg(format!("--action=default={action}"));
    }
    command.args(["--", event.title.as_str(), &body]);
    command
}
