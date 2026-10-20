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
    event: Option<NotificationEvent>,
    notified: Option<CallScope>,
}
impl CallEvents {
    pub(super) fn new() -> Self {
        Self {
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
    update_sealed: bool,
}
pub(super) struct CallActions(Mutex<ActionState>);
impl CallActions {
    pub(super) fn new() -> Self {
        Self(Mutex::new(ActionState {
            lease: None,
            pending: None,
            update_sealed: false,
        }))
    }
    pub(super) fn clear(&self) {
        if let Ok(mut state) = self.0.lock() {
            state.lease = None;
            state.pending = None;
        }
    }
    pub(super) fn update_sealed(&self) -> bool {
        self.0.lock().map_or(true, |state| state.update_sealed)
    }
    pub(super) fn seal_for_update(
        &self,
    ) -> Result<(), talk_core::preferences::updates::UpdateError> {
        use talk_core::preferences::updates::UpdateError;
        let mut state = self.0.lock().map_err(|_| UpdateError::Unavailable)?;
        if state.update_sealed {
            return Err(UpdateError::Busy);
        }
        if state.pending.is_some()
            || state.lease.as_ref().is_some_and(|lease| {
                lease.expires > Instant::now() && !lease.cancellation.is_cancelled()
            })
        {
            return Err(UpdateError::ActiveCall);
        }
        state.lease = None;
        state.update_sealed = true;
        Ok(())
    }
    pub(super) fn release_update_seal(&self) {
        if let Ok(mut state) = self.0.lock() {
            state.update_sealed = false;
        }
    }
    fn clear_account(&self, scope: AccountScope) {
        if let Ok(mut state) = self.0.lock() {
            if state
                .lease
                .as_ref()
                .is_some_and(|l| l.call.account == scope)
            {
                state.lease = None;
            }
            if state.pending.is_some_and(|(call, _)| call.account == scope) {
                state.pending = None;
            }
        }
    }
    fn admit(&self, call: CallScope, cancellation: talk_core::lifecycle::Cancellation) {
        if let Ok(mut state) = self.0.lock()
            && !state.update_sealed
        {
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
            !s.update_sealed
                && s.pending.is_none()
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
        if state.update_sealed {
            return Err(NotificationError::Closed);
        }
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
    pub fn incoming_source_available(&self, scope: AccountScope) -> bool {
        self.current(scope).is_ok()
            && self
                .background
                .source(scope, background::Observation::Calls, Instant::now())
    }
    pub fn ringing_call(&self) -> Option<CallScope> {
        self.snapshot
            .call
            .as_ref()
            .filter(|c| {
                c.stage == CallStage::Incoming
                    && self.incoming_source_available(c.scope.account)
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
        if self.call_actions.update_sealed() {
            return;
        }
        let cancellation = match self
            .connection
            .as_ref()
            .map_err(|e| *e)
            .and_then(|c| c.account_cancellation(scope))
        {
            Ok(cancellation) => cancellation,
            Err(_) => {
                self.invalidate_incoming(scope);
                return;
            }
        };
        let Ok(RuntimeReply::Protocol(ProtocolReply::IncomingCalls(value))) = result else {
            self.invalidate_incoming(scope);
            return;
        };
        self.apply_incoming(scope, value, cancellation);
    }
    fn apply_incoming(
        &mut self,
        scope: AccountScope,
        value: talk_core::calls::IncomingCalls,
        cancellation: talk_core::lifecycle::Cancellation,
    ) {
        if cancellation.is_cancelled() {
            self.invalidate_incoming(scope);
            return;
        }
        if value.scope != scope || value.call.is_some() != value.title.is_some() {
            self.invalidate_incoming(scope);
            return;
        }
        self.background
            .observed(scope, background::Observation::Calls, true);
        if let (Some(call), Some(title)) = (value.call, value.title) {
            if call.call.account != scope
                || !call.call.is_consistent()
                || call.entry != Entry::IncomingDirect
                || call.phase != Phase::Ringing
            {
                self.invalidate_incoming(scope);
                return;
            }
            if self
                .snapshot
                .call
                .as_ref()
                .is_some_and(|v| !can_offer(v, call.call))
            {
                return;
            }
            if self
                .call_ui
                .as_ref()
                .is_some_and(|ui| ui.scope != call.call)
            {
                self.call_actions.clear();
                self.call_events.event = None;
            }
            self.call_actions.admit(call.call, cancellation);
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
    fn invalidate_incoming(&mut self, scope: AccountScope) {
        self.background
            .observed(scope, background::Observation::Calls, false);
        self.call_actions.clear_account(scope);
        if self.call_events.event.as_ref().is_some_and(
            |e| matches!(e.kind, NotificationKind::IncomingCall { call } if call.account == scope),
        ) {
            self.call_events.event = None;
        }
        if let Some(call) = self
            .snapshot
            .call
            .as_mut()
            .filter(|c| c.scope.account == scope && c.stage == CallStage::Incoming)
        {
            call.accept = Decision::Unknown;
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
                UiIntent::DeclineDirect(call) => {
                    if self.ringing_call() != Some(*call) {
                        return Err(SubmitError::StaleScope);
                    }
                    self.request(
                        call.account,
                        ProtocolRequest::Meeting(Command::Stop(*call)),
                        Purpose::Call(*call),
                    )?;
                    self.call_actions.clear();
                    self.call_events.event = None;
                    if let Some(view) = &mut self.snapshot.call {
                        view.stage = CallStage::Ended;
                        view.accept = Decision::Denied;
                    }
                    self.touch();
                    return Ok(());
                }
                UiIntent::Legacy(UserIntent::Leave(call)) => {
                    if self.call_ui.as_ref().is_none_or(|ui| ui.scope != *call) {
                        return Err(SubmitError::StaleScope);
                    }
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
            if self.ringing_call() != Some(scope) {
                return Err(SubmitError::StaleScope);
            }
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
        self.request(
            scope.account,
            ProtocolRequest::Meeting(Command::Start {
                call: scope,
                entry,
                wishes,
            }),
            Purpose::Call(scope),
        )?;
        self.call_actions.clear();
        self.call_events.event = None;
        if entry == Entry::IncomingDirect {
            self.select(Some(scope.account));
        }
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
        if self.call_actions.update_sealed() {
            return;
        }
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
        if let Some(call) = self.snapshot.call.as_ref()
            && call.stage == CallStage::Incoming
            && !self.incoming_source_available(call.scope.account)
        {
            self.invalidate_incoming(call.scope.account);
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
// One visible/current invitation. Other accounts remain observed and may offer
// their invitation after this one ends; they never replace an active call.
fn can_offer(current: &CallView, incoming: CallScope) -> bool {
    matches!(current.stage, CallStage::Ended | CallStage::Failed)
        || current.stage == CallStage::Incoming && current.scope.account == incoming.account
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
    fn invitation(call: CallScope) -> talk_core::calls::IncomingCalls {
        talk_core::calls::IncomingCalls {
            scope: call.account,
            call: Some(Snapshot {
                call,
                entry: Entry::IncomingDirect,
                phase: Phase::Ringing,
                participants: Page::empty(),
                microphone: DeviceState::Off,
                camera: DeviceState::Off,
                screen: DeviceState::Off,
                peers: 0,
                recording: talk_core::policy::RecordingState::Unknown,
                counters: Default::default(),
                revision: 1,
            }),
            title: Some(text("Incoming fixture")),
            eligible: true,
        }
    }
    #[test]
    fn inactive_account_can_offer_without_selecting_or_capturing_and_other_errors_cannot_revoke_it()
    {
        let mut model = super::super::tests::model();
        let call = call();
        let active = AccountScope {
            account: AccountId::from_local(2),
            runtime: RuntimeGeneration::from_local(2),
        };
        model.snapshot.active_account = Some(active);
        model
            .background
            .sync(&[active, call.account], Instant::now());
        model.apply_incoming(
            call.account,
            invitation(call),
            talk_core::lifecycle::Cancellation::default(),
        );
        let view = model.snapshot.call.as_ref().unwrap().clone();
        assert_eq!(view.scope, call);
        assert_eq!(view.stage, CallStage::Incoming);
        assert_eq!(view.microphone, MediaState::Off);
        assert_eq!(view.camera, MediaState::Off);
        assert_eq!(
            model.snapshot.active_account,
            Some(active),
            "ringing never navigates"
        );
        assert!(model.call_actions.available_actions(call).accept);
        model.complete_incoming(active, Err(IdentityError::Request(ErrorKind::Deadline)));
        assert_eq!(model.snapshot.call, Some(view));
        assert!(model.call_actions.available_actions(call).accept);
        let stale = AccountScope {
            runtime: RuntimeGeneration::from_local(1),
            ..call.account
        };
        model.complete_incoming(stale, Err(IdentityError::Request(ErrorKind::WrongScope)));
        assert!(model.call_actions.available_actions(call).accept);
        model.invalidate_incoming(call.account);
        assert!(!model.call_actions.available_actions(call).accept);
    }
    #[test]
    fn another_account_invitation_never_takes_the_current_call_and_cancelled_observations_do_not_ring()
     {
        let mut model = super::super::tests::model();
        let first = call();
        let account = AccountId::from_local(2);
        let second = CallScope {
            account: AccountScope {
                account,
                runtime: RuntimeGeneration::from_local(2),
            },
            conversation: ConversationId {
                account,
                ..first.conversation
            },
            ..first
        };
        model
            .background
            .sync(&[first.account, second.account], Instant::now());
        model.apply_incoming(
            first.account,
            invitation(first),
            talk_core::lifecycle::Cancellation::default(),
        );
        model.apply_incoming(
            second.account,
            invitation(second),
            talk_core::lifecycle::Cancellation::default(),
        );
        assert_eq!(model.snapshot.call.as_ref().unwrap().scope, first);
        model.snapshot.call.as_mut().unwrap().stage = CallStage::Active;
        model.apply_incoming(
            second.account,
            invitation(second),
            talk_core::lifecycle::Cancellation::default(),
        );
        assert_eq!(model.snapshot.call.as_ref().unwrap().scope, first);
        assert_eq!(
            model.snapshot.call.as_ref().unwrap().stage,
            CallStage::Active
        );
        model.invalidate_incoming(second.account);
        assert_eq!(
            model.snapshot.call.as_ref().unwrap().stage,
            CallStage::Active
        );
        model.snapshot.call = None;
        model.call_ui = None;
        model.call_actions.clear();
        let cancelled = talk_core::lifecycle::Cancellation::default();
        cancelled.cancel();
        model.apply_incoming(second.account, invitation(second), cancelled);
        assert!(model.snapshot.call.is_none());
        assert!(!model.call_actions.available_actions(second).decline);
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
    #[test]
    fn update_seal_refuses_ringing_or_queued_action_and_cannot_be_cleared_by_call_cleanup() {
        use talk_core::preferences::updates::UpdateError;
        let actions = CallActions::new();
        let call = call();
        actions.admit(call, talk_core::lifecycle::Cancellation::default());
        assert_eq!(actions.seal_for_update(), Err(UpdateError::ActiveCall));
        assert_eq!(actions.dispatch(call, NativeCallAction::Accept), Ok(()));
        assert_eq!(actions.seal_for_update(), Err(UpdateError::ActiveCall));
        assert_eq!(actions.take(), Some((call, NativeCallAction::Accept)));
        assert_eq!(actions.seal_for_update(), Ok(()));
        actions.admit(call, talk_core::lifecycle::Cancellation::default());
        assert!(!actions.available_actions(call).accept);
        assert_eq!(
            actions.dispatch(call, NativeCallAction::Accept),
            Err(NotificationError::Closed)
        );
        actions.clear();
        assert!(actions.update_sealed());
        actions.release_update_seal();
        actions.admit(call, talk_core::lifecycle::Cancellation::default());
        assert!(actions.available_actions(call).accept);
    }
}
