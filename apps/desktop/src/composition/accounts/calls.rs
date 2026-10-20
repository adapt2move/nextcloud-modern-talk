//! UI correlation only. The authenticated account worker grants every call and
//! device transition; a row or renderer cannot manufacture admission.
use super::*;
use std::sync::{Arc, Mutex};
use talk_core::calls::{Command, DeviceState, Entry, Gate, MediaWishes, Phase, Snapshot};
use talk_core::ids::CallScope;
use talk_core::preferences::notifications::{
    NativeCallAction, NativeCallActions, NotificationError, NotificationEvent, NotificationKind,
};
use talk_core::ui::MediaIntent;
use talk_core::ui::presentation::{CallStage, CallView, MediaState, ParticipantView, RequiredGate};

pub(super) struct CallEvents {
    poll_at: Instant,
    source: Option<(AccountScope, Instant)>,
    event: Option<NotificationEvent>,
    notified: Option<CallScope>,
}
impl CallEvents {
    pub(super) fn new() -> Self {
        Self {
            poll_at: Instant::now(),
            source: None,
            event: None,
            notified: None,
        }
    }
}
struct ActionLease {
    call: CallScope,
    expires: Instant,
    cancellation: talk_core::lifecycle::Cancellation,
}
struct ActionState {
    lease: Option<ActionLease>,
    pending: Option<(CallScope, NativeCallAction)>,
}
pub(super) struct CallActions(Mutex<ActionState>);
impl CallActions {
    pub(super) fn new() -> Self {
        Self(Mutex::new(ActionState {
            lease: None,
            pending: None,
        }))
    }
    pub(super) fn clear(&self) {
        if let Ok(mut state) = self.0.lock() {
            state.lease = None;
            state.pending = None;
        }
    }
    fn admit(&self, call: CallScope, cancellation: talk_core::lifecycle::Cancellation) {
        if let Ok(mut state) = self.0.lock() {
            state.lease = Some(ActionLease {
                call,
                expires: Instant::now() + Duration::from_secs(7),
                cancellation,
            });
        }
    }
    fn take(&self) -> Option<(CallScope, NativeCallAction)> {
        self.0.lock().ok()?.pending.take()
    }
}
impl crate::settings::notifications::NotificationCallDispatcher for CallActions {
    fn available_actions(&self, call: CallScope) -> NativeCallActions {
        let valid = self.0.lock().is_ok_and(|s| {
            s.pending.is_none()
                && s.lease.as_ref().is_some_and(|l| {
                    l.call == call && l.expires > Instant::now() && !l.cancellation.is_cancelled()
                })
        });
        NativeCallActions {
            accept: valid,
            decline: valid,
        }
    }
    fn dispatch(&self, call: CallScope, action: NativeCallAction) -> Result<(), NotificationError> {
        let mut state = self.0.lock().map_err(|_| NotificationError::Closed)?;
        if state.pending.is_some() {
            return Err(NotificationError::Busy);
        }
        if !state.lease.as_ref().is_some_and(|l| {
            l.call == call && l.expires > Instant::now() && !l.cancellation.is_cancelled()
        }) {
            return Err(NotificationError::Stale);
        }
        state.lease = None;
        state.pending = Some((call, action));
        Ok(())
    }
}

pub(super) struct CallUi {
    scope: CallScope,
    title: Text<256>,
    direct: bool,
    wishes: MediaWishes,
}
impl Accounts {
    pub fn call_dispatcher(
        &self,
    ) -> Arc<dyn crate::settings::notifications::NotificationCallDispatcher> {
        self.call_actions.clone()
    }
    pub fn incoming_source_available(&self) -> bool {
        self.call_events.source.is_some_and(|(scope, at)| {
            Some(scope) == self.snapshot.active_account
                && at.elapsed() < Duration::from_secs(12)
                && self.current(scope).is_ok()
        })
    }
    pub fn ringing_call(&self) -> Option<CallScope> {
        self.snapshot
            .call
            .as_ref()
            .filter(|c| {
                c.stage == CallStage::Incoming
                    && self.incoming_source_available()
                    && self.current(c.scope.account).is_ok()
            })
            .map(|c| c.scope)
    }
    pub fn incoming_notification(&mut self) -> Option<NotificationEvent> {
        let event = self.call_events.event.as_ref()?.clone();
        if let NotificationKind::IncomingCall { call } = event.kind
            && self.ringing_call() == Some(call)
        {
            return Some(event);
        }
        self.call_events.event = None;
        None
    }
    pub fn settle_incoming_notification(&mut self, kind: NotificationKind) {
        if self
            .call_events
            .event
            .as_ref()
            .is_some_and(|event| event.kind == kind)
        {
            self.call_events.event = None;
        }
    }
    pub(super) fn complete_incoming(
        &mut self,
        scope: AccountScope,
        result: Result<RuntimeReply, IdentityError>,
    ) {
        if self.current(scope).is_err() {
            self.call_actions.clear();
            self.call_events.source = None;
            return;
        }
        let Ok(RuntimeReply::Protocol(ProtocolReply::IncomingCalls(value))) = result else {
            self.call_actions.clear();
            self.call_events.source = None;
            return;
        };
        if value.scope != scope {
            return;
        }
        self.call_events.source = Some((scope, Instant::now()));
        if let (Some(call), Some(title)) = (value.call, value.title) {
            if call.call.account != scope
                || call.entry != Entry::IncomingDirect
                || call.phase != Phase::Ringing
            {
                return;
            }
            if self.snapshot.call.as_ref().is_some_and(|v| {
                !matches!(
                    v.stage,
                    CallStage::Incoming | CallStage::Ended | CallStage::Failed
                )
            }) {
                return;
            }
            if let Ok(cancellation) = self
                .connection
                .as_ref()
                .map_err(|e| *e)
                .and_then(|c| c.account_cancellation(scope))
            {
                self.call_actions.admit(call.call, cancellation);
            }
            self.snapshot.call = Some(project(&call, &title, true));
            self.call_ui = Some(CallUi {
                scope: call.call,
                title: title.clone(),
                direct: true,
                wishes: MediaWishes::default(),
            });
            self.snapshot.conversations =
                page(self.snapshot.conversations.as_slice().iter().map(|row| {
                    let mut row = row.clone();
                    if row.context.conversation == call.call.conversation {
                        row.call_scope = Some(call.call);
                    }
                    row
                }));
            if self.call_events.notified != Some(call.call) {
                self.call_events.notified = Some(call.call);
                self.call_events.event = Some(NotificationEvent {
                    request: talk_core::preferences::RequestId(0),
                    kind: NotificationKind::IncomingCall { call: call.call },
                    title: Text::new("Eingehender Anruf").expect("bounded literal"),
                    body: text(title.as_str()),
                    minimized: false,
                    eligible: value.eligible,
                });
            }
            self.touch();
        } else if self
            .snapshot
            .call
            .as_ref()
            .is_some_and(|c| c.scope.account == scope && c.stage == CallStage::Incoming)
        {
            self.call_actions.clear();
            self.call_events.event = None;
            self.call_ui = None;
            if let Some(call) = &mut self.snapshot.call {
                call.stage = CallStage::Ended;
                call.accept = Decision::Denied;
            }
            self.touch();
        }
    }
    pub(super) fn submit_call(&mut self, intent: &UiIntent) -> Option<Result<(), SubmitError>> {
        let result = (|| {
            let command = match intent {
                UiIntent::StartDirect { call, media } => {
                    let row = self
                        .snapshot
                        .conversations
                        .as_slice()
                        .iter()
                        .find(|r| r.call_scope == Some(*call))
                        .ok_or(SubmitError::StaleScope)?;
                    let direct = row.kind == ConversationKind::Direct;
                    self.start_call(
                        *call,
                        if direct {
                            Entry::OutgoingDirect
                        } else {
                            Entry::GroupAudio
                        },
                        wishes(*media),
                        direct,
                    )?;
                    return Ok(());
                }
                UiIntent::JoinConference {
                    call,
                    microphone,
                    camera,
                } => {
                    self.start_call(
                        *call,
                        Entry::PreparedConference,
                        MediaWishes {
                            microphone: *microphone,
                            camera: *camera,
                            screen: false,
                        },
                        false,
                    )?;
                    return Ok(());
                }
                UiIntent::Legacy(UserIntent::AcceptDirect { call, media }) => {
                    self.start_call(*call, Entry::IncomingDirect, wishes(*media), true)?;
                    return Ok(());
                }
                UiIntent::Legacy(UserIntent::JoinPrepared { call, media }) => {
                    self.start_call(*call, Entry::PreparedConference, wishes(*media), false)?;
                    return Ok(());
                }
                UiIntent::SetMicrophone { call, enabled } => {
                    let ui = self
                        .call_ui
                        .as_mut()
                        .filter(|u| u.scope == *call)
                        .ok_or(SubmitError::StaleScope)?;
                    ui.wishes.microphone = *enabled;
                    Command::Change {
                        call: *call,
                        wishes: ui.wishes,
                    }
                }
                UiIntent::SetCamera { call, enabled } => {
                    let ui = self
                        .call_ui
                        .as_mut()
                        .filter(|u| u.scope == *call)
                        .ok_or(SubmitError::StaleScope)?;
                    ui.wishes.camera = *enabled;
                    Command::Change {
                        call: *call,
                        wishes: ui.wishes,
                    }
                }
                UiIntent::ShareScreen(_) => {
                    return Err(SubmitError::Unavailable(UnavailableReason::Unsupported));
                }
                UiIntent::StopScreen(call) => {
                    let ui = self
                        .call_ui
                        .as_mut()
                        .filter(|u| u.scope == *call)
                        .ok_or(SubmitError::StaleScope)?;
                    ui.wishes.screen = false;
                    Command::Change {
                        call: *call,
                        wishes: ui.wishes,
                    }
                }
                UiIntent::ReconnectCall(call) => {
                    let ui = self
                        .call_ui
                        .as_mut()
                        .filter(|u| u.scope == *call)
                        .ok_or(SubmitError::StaleScope)?;
                    ui.wishes = MediaWishes::default();
                    Command::Reconnect(*call)
                }
                UiIntent::DeclineDirect(call) | UiIntent::Legacy(UserIntent::Leave(call)) => {
                    self.media.close_call(*call);
                    self.call_actions.clear();
                    self.call_events.event = None;
                    Command::Stop(*call)
                }
                UiIntent::ResolveRequiredGate {
                    call,
                    gate: RequiredGate::RecordingConsent,
                } => Command::Consent(*call),
                UiIntent::ResolveRequiredGate { .. } => {
                    return Err(SubmitError::Unavailable(
                        UnavailableReason::PermissionDenied,
                    ));
                }
                _ => return Ok(()),
            };
            let scope = command.scope();
            self.request(
                scope.account,
                ProtocolRequest::Meeting(command),
                Purpose::Call(scope),
            )?;
            Ok(())
        })();
        matches!(
            intent,
            UiIntent::StartDirect { .. }
                | UiIntent::JoinConference { .. }
                | UiIntent::DeclineDirect(_)
                | UiIntent::SetMicrophone { .. }
                | UiIntent::SetCamera { .. }
                | UiIntent::ShareScreen(_)
                | UiIntent::StopScreen(_)
                | UiIntent::ReconnectCall(_)
                | UiIntent::ResolveRequiredGate { .. }
                | UiIntent::Legacy(
                    UserIntent::AcceptDirect { .. }
                        | UserIntent::JoinPrepared { .. }
                        | UserIntent::Leave(_)
                )
        )
        .then_some(result)
    }
    fn start_call(
        &mut self,
        scope: CallScope,
        entry: Entry,
        wishes: MediaWishes,
        direct: bool,
    ) -> Result<(), SubmitError> {
        self.current(scope.account)?;
        if self.call_ui.as_ref().is_some_and(|ui| ui.scope != scope)
            && self
                .snapshot
                .call
                .as_ref()
                .is_some_and(|c| !matches!(c.stage, CallStage::Ended | CallStage::Failed))
        {
            return Err(SubmitError::Busy);
        }
        let title = if entry == Entry::IncomingDirect {
            self.call_ui
                .as_ref()
                .filter(|u| u.scope == scope)
                .ok_or(SubmitError::StaleScope)?
                .title
                .clone()
        } else {
            let row = self
                .snapshot
                .conversations
                .as_slice()
                .iter()
                .find(|r| r.call_scope == Some(scope))
                .ok_or(SubmitError::StaleScope)?;
            if row.call != Decision::Allowed {
                return Err(SubmitError::Unavailable(
                    UnavailableReason::PermissionDenied,
                ));
            }
            row.title.clone()
        };
        self.call_actions.clear();
        self.call_events.event = None;
        self.request(
            scope.account,
            ProtocolRequest::Meeting(Command::Start {
                call: scope,
                entry,
                wishes,
            }),
            Purpose::Call(scope),
        )?;
        self.call_ui = Some(CallUi {
            scope,
            title: title.clone(),
            direct,
            wishes,
        });
        self.snapshot.call = Some(CallView {
            scope,
            title,
            direct,
            stage: if direct {
                CallStage::Outgoing
            } else {
                CallStage::Connecting
            },
            accept: Decision::Unknown,
            microphone: MediaState::Off,
            camera: MediaState::Off,
            share: MediaState::Unavailable,
            participants: Page::empty(),
        });
        self.call_poll_at = Instant::now();
        self.touch();
        Ok(())
    }
    pub(super) fn tick_calls(&mut self) {
        if let Some((call, action)) = self.call_actions.take() {
            let intent = match action {
                NativeCallAction::Accept => UiIntent::Legacy(UserIntent::AcceptDirect {
                    call,
                    media: MediaIntent::MicrophoneOnCameraOff,
                }),
                NativeCallAction::Decline => UiIntent::DeclineDirect(call),
            };
            let _ = self.submit_call(&intent);
        }
        if let Some(scope) = self.snapshot.active_account {
            let free = self.snapshot.call.as_ref().is_none_or(|c| {
                matches!(
                    c.stage,
                    CallStage::Incoming | CallStage::Ended | CallStage::Failed
                )
            });
            if free
                && Instant::now() >= self.call_events.poll_at
                && !self
                    .pending
                    .iter()
                    .any(|p| matches!(p.purpose, Purpose::IncomingCalls(_)))
                && self
                    .request(
                        scope,
                        ProtocolRequest::PollIncomingCalls,
                        Purpose::IncomingCalls(scope),
                    )
                    .is_ok()
            {
                self.call_events.poll_at = Instant::now() + Duration::from_secs(5);
            }
        }
        let Some(ui) = &self.call_ui else {
            return;
        };
        let scope = ui.scope;
        if self.current(scope.account).is_err() {
            self.media.close_call(scope);
            if let Some(view) = &mut self.snapshot.call {
                view.stage = CallStage::Ended;
                view.microphone = MediaState::Off;
                view.camera = MediaState::Off;
                view.share = MediaState::Off;
            }
            self.call_ui = None;
            self.touch();
            return;
        }
        if self.snapshot.call.as_ref().is_some_and(|c| {
            matches!(
                c.stage,
                CallStage::Ended | CallStage::Failed | CallStage::Incoming
            )
        }) {
            return;
        }
        if Instant::now() < self.call_poll_at
            || self
                .pending
                .iter()
                .any(|p| matches!(p.purpose, Purpose::Call(_)))
        {
            return;
        }
        if self
            .request(
                scope.account,
                ProtocolRequest::Meeting(Command::Observe(scope)),
                Purpose::Call(scope),
            )
            .is_ok()
        {
            self.call_poll_at = Instant::now() + Duration::from_millis(250);
        }
    }
    pub(super) fn complete_call(
        &mut self,
        requested: CallScope,
        result: Result<RuntimeReply, IdentityError>,
    ) {
        let Some(ui) = self.call_ui.as_mut().filter(|u| u.scope == requested) else {
            return;
        };
        match result {
            Ok(RuntimeReply::Protocol(ProtocolReply::Meeting(value)))
                if value.call.account == requested.account
                    && value.call.conversation == requested.conversation =>
            {
                ui.scope = value.call;
                if matches!(
                    value.phase,
                    Phase::Reconnecting | Phase::Ended | Phase::Waiting(Gate::PolicyStale)
                ) {
                    ui.wishes = MediaWishes::default();
                }
                self.snapshot.call = Some(project(&value, &ui.title, ui.direct));
                self.snapshot.conversations =
                    page(self.snapshot.conversations.as_slice().iter().map(|row| {
                        let mut row = row.clone();
                        if row.call_scope == Some(requested) {
                            row.call_scope = Some(value.call);
                        }
                        row
                    }));
            }
            Err(IdentityError::Request(ErrorKind::Busy)) => {}
            _ => {
                self.media.close_call(requested);
                if let Some(call) = &mut self.snapshot.call {
                    call.stage = CallStage::Failed;
                    call.microphone = MediaState::Off;
                    call.camera = MediaState::Off;
                    call.share = MediaState::Unavailable;
                }
            }
        }
    }
}
fn wishes(value: MediaIntent) -> MediaWishes {
    MediaWishes {
        microphone: value != MediaIntent::MicrophoneOffCameraOff,
        camera: value == MediaIntent::MicrophoneOnCameraOn,
        screen: false,
    }
}
fn device(state: DeviceState) -> MediaState {
    match state {
        DeviceState::Off => MediaState::Off,
        DeviceState::Requested => MediaState::Requested,
        DeviceState::Active => MediaState::Active,
        DeviceState::Unavailable => MediaState::Unavailable,
    }
}
fn project(value: &Snapshot, title: &Text<256>, direct: bool) -> CallView {
    let stage = match value.phase {
        Phase::Ringing => CallStage::Incoming,
        Phase::Preparing | Phase::Connecting => CallStage::Connecting,
        Phase::Active => CallStage::Active,
        Phase::Reconnecting => CallStage::Reconnecting,
        Phase::Ending | Phase::Ended => CallStage::Ended,
        Phase::Waiting(gate) => match gate {
            Gate::Lobby => CallStage::Required(RequiredGate::Lobby),
            Gate::RecordingConsent => CallStage::Required(RequiredGate::RecordingConsent),
            Gate::RequiredEncryption | Gate::CallForbidden => {
                CallStage::Required(RequiredGate::SecurityPolicy)
            }
            Gate::PolicyStale | Gate::MembershipLost => CallStage::Reconnecting,
            _ => CallStage::Failed,
        },
    };
    CallView {
        scope: value.call,
        title: title.clone(),
        direct,
        stage,
        accept: if stage == CallStage::Incoming {
            Decision::Allowed
        } else {
            Decision::Denied
        },
        microphone: device(value.microphone),
        camera: device(value.camera),
        share: device(value.screen),
        participants: page(
            value
                .participants
                .as_slice()
                .iter()
                .map(|p| ParticipantView {
                    local_id: p.local,
                    name: text(p.name.as_str()),
                    initials: initials(p.name.as_str()),
                    role_label: text("Teilnehmend"),
                    presence: Presence::Unknown,
                    self_user: false,
                }),
        ),
    }
}

#[cfg(test)]
mod action_tests {
    use super::*;
    use crate::settings::notifications::NotificationCallDispatcher;
    use talk_core::ids::*;
    fn call() -> CallScope {
        let account = AccountId::from_local(1);
        CallScope {
            account: AccountScope {
                account,
                runtime: RuntimeGeneration::from_local(2),
            },
            conversation: ConversationId {
                account,
                host: HostId::from_local(3),
                local: 4,
            },
            call: CallGeneration::from_local(5),
        }
    }
    #[test]
    fn native_action_requires_current_invitation_and_accepts_only_once() {
        let actions = CallActions::new();
        let call = call();
        assert_eq!(
            actions.dispatch(call, NativeCallAction::Accept),
            Err(NotificationError::Stale)
        );
        actions.admit(call, talk_core::lifecycle::Cancellation::default());
        let mut old = call;
        old.call = CallGeneration::from_local(6);
        assert_eq!(
            actions.dispatch(old, NativeCallAction::Decline),
            Err(NotificationError::Stale)
        );
        assert!(actions.available_actions(call).accept);
        assert_eq!(actions.dispatch(call, NativeCallAction::Accept), Ok(()));
        assert_eq!(
            actions.dispatch(call, NativeCallAction::Accept),
            Err(NotificationError::Busy)
        );
        assert_eq!(actions.take(), Some((call, NativeCallAction::Accept)));
        assert!(!actions.available_actions(call).accept);
    }
    #[test]
    fn cancelled_account_and_expired_observation_disable_native_actions() {
        let actions = CallActions::new();
        let call = call();
        let cancellation = talk_core::lifecycle::Cancellation::default();
        actions.admit(call, cancellation.clone());
        cancellation.cancel();
        assert!(!actions.available_actions(call).decline);
        assert_eq!(
            actions.dispatch(call, NativeCallAction::Decline),
            Err(NotificationError::Stale)
        );
        actions.admit(call, talk_core::lifecycle::Cancellation::default());
        actions.0.lock().unwrap().lease.as_mut().unwrap().expires = Instant::now();
        assert_eq!(
            actions.dispatch(call, NativeCallAction::Accept),
            Err(NotificationError::Stale)
        );
    }
}
