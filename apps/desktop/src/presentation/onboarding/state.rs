//! Finite setup state. Observations never grant capture authority.
use std::time::Instant;
use talk_core::preferences::{
    OnboardingResume,
    permissions::{PermissionAction, PermissionKind, PermissionObservation, PermissionStatus},
};
use talk_core::ui::AccountScope;
use talk_core::ui::presentation::{LoginFailure, LoginPhase, StorageStatus};

/// A semantic step changes focus once, after its controls have been rendered.
/// Snapshot polling, permission observations and locale changes do not re-enter it.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum FocusStep {
    Server,
    Validating,
    Browser,
    Storage { pending: bool },
    Permission(PermissionKind),
    Done,
    Failed(LoginFailure),
}
impl FocusStep {
    pub fn control(self) -> Option<&'static str> {
        Some(match self {
            Self::Server => return None,
            Self::Validating => "cancel-validating",
            Self::Browser => "reopen-browser",
            Self::Storage { pending: true } => "cancel-storage",
            Self::Storage { pending: false } => "retry-native",
            Self::Permission(_) => "permission-setup-next",
            Self::Done => "onboarding-done",
            Self::Failed(LoginFailure::AccountExists) => "login-open-accounts",
            Self::Failed(_) => "retry-login",
        })
    }
}

pub(super) fn index(kind: PermissionKind) -> usize {
    match kind {
        PermissionKind::Microphone => 0,
        PermissionKind::Camera => 1,
        PermissionKind::ScreenSharing => 2,
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum Advance {
    Wait,
    Request,
    Next,
}

pub(in crate::presentation) struct OnboardingUi {
    pub(super) focused_step: Option<FocusStep>,
    pub scope: Option<AccountScope>,
    pub kind: PermissionKind,
    pub ready: bool,
    pub resumed: bool,
    pub entry: u64,
    pub(super) checked: [bool; 3],
    pub(super) queued: [bool; 3],
    pub(super) requested: [bool; 3],
    pub(super) initial_resume: Option<OnboardingResume>,
    pub(super) startup_read: bool,
    pub(super) checkpoint_attempted: bool,
    pub(super) clearing: bool,
    pub(super) finish_pending: bool,
    pub(super) recheck_at: Option<Instant>,
    pub(super) rechecks_left: u8,
}
impl OnboardingUi {
    pub fn new() -> Self {
        Self {
            focused_step: None,
            scope: None,
            kind: PermissionKind::Microphone,
            ready: false,
            resumed: false,
            entry: 0,
            checked: [false; 3],
            queued: [false; 3],
            requested: [false; 3],
            initial_resume: None,
            startup_read: false,
            checkpoint_attempted: false,
            clearing: false,
            finish_pending: false,
            recheck_at: None,
            rechecks_left: 0,
        }
    }

    pub(super) fn focus_step(&self, phase: LoginPhase, storage: StorageStatus) -> FocusStep {
        if self.resumed || phase == LoginPhase::Ready {
            return if self.ready {
                FocusStep::Done
            } else {
                FocusStep::Permission(self.kind)
            };
        }
        match phase {
            LoginPhase::EnterServer => FocusStep::Server,
            LoginPhase::Validating => FocusStep::Validating,
            LoginPhase::BrowserWaiting => FocusStep::Browser,
            LoginPhase::ChooseStorage => FocusStep::Storage {
                pending: matches!(
                    storage,
                    StorageStatus::Saving
                        | StorageStatus::CleanupPending
                        | StorageStatus::RecoveryRequired
                ),
            },
            LoginPhase::Failed(failure) => FocusStep::Failed(failure),
            LoginPhase::Ready => FocusStep::Done,
        }
    }

    pub(super) fn pending_focus(
        &self,
        phase: LoginPhase,
        storage: StorageStatus,
    ) -> Option<FocusStep> {
        let step = self.focus_step(phase, storage);
        (self.focused_step != Some(step)).then_some(step)
    }

    pub(super) fn begin(&mut self, scope: AccountScope, resumed: bool) {
        self.scope = Some(scope);
        self.kind = PermissionKind::Microphone;
        self.ready = resumed;
        self.resumed = resumed;
        self.entry = self.entry.saturating_add(1);
        self.checked = [false; 3];
        self.queued = [false; 3];
        self.requested = [false; 3];
        self.checkpoint_attempted = false;
        self.finish_pending = false;
        self.recheck_at = None;
        self.rechecks_left = 0;
    }

    pub(super) fn enter(&mut self, kind: PermissionKind) {
        self.kind = kind;
        self.ready = false;
        self.entry = self.entry.saturating_add(1);
        self.checked[index(kind)] = false;
        self.queued[index(kind)] = false;
        self.recheck_at = None;
        self.rechecks_left = 0;
        // Returning to a step does not re-arm a native prompt.
    }

    pub(super) fn complete(
        &mut self,
        scope: AccountScope,
        entry: u64,
        kind: PermissionKind,
        action: PermissionAction,
        succeeded: bool,
    ) -> bool {
        if self.scope != Some(scope) || self.entry != entry || self.kind != kind || self.ready {
            return false;
        }
        if action != PermissionAction::OpenSystemSettings {
            self.checked[index(kind)] = succeeded;
        }
        true
    }

    pub(super) fn advance(&self, observation: PermissionObservation, idle: bool) -> Advance {
        if self.ready || !idle || !self.checked[index(self.kind)] || observation.kind != self.kind {
            return Advance::Wait;
        }
        if observation.status == PermissionStatus::Granted {
            Advance::Next
        } else if observation.can_request
            && !self.requested[index(self.kind)]
            && matches!(
                observation.status,
                PermissionStatus::NotDetermined | PermissionStatus::Unknown
            )
        {
            Advance::Request
        } else {
            Advance::Wait
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use talk_core::ui::{AccountId, RuntimeGeneration};

    fn scope(value: u64) -> AccountScope {
        AccountScope {
            account: AccountId::from_local(value.into()),
            runtime: RuntimeGeneration::from_local(1),
        }
    }
    #[test]
    fn asynchronous_login_and_permission_steps_enter_focus_once() {
        let mut ui = OnboardingUi::new();
        for (phase, storage, expected) in [
            (
                LoginPhase::EnterServer,
                StorageStatus::NotSaved,
                FocusStep::Server,
            ),
            (
                LoginPhase::Validating,
                StorageStatus::NotSaved,
                FocusStep::Validating,
            ),
            (
                LoginPhase::BrowserWaiting,
                StorageStatus::NotSaved,
                FocusStep::Browser,
            ),
            (
                LoginPhase::ChooseStorage,
                StorageStatus::Saving,
                FocusStep::Storage { pending: true },
            ),
            (
                LoginPhase::ChooseStorage,
                StorageStatus::NotSaved,
                FocusStep::Storage { pending: false },
            ),
            (
                LoginPhase::Failed(LoginFailure::Network),
                StorageStatus::NotSaved,
                FocusStep::Failed(LoginFailure::Network),
            ),
            (
                LoginPhase::EnterServer,
                StorageStatus::NotSaved,
                FocusStep::Server,
            ),
        ] {
            assert_eq!(ui.pending_focus(phase, storage), Some(expected));
            ui.focused_step = Some(expected);
            assert_eq!(
                ui.pending_focus(phase, storage),
                None,
                "snapshot polling must not steal focus"
            );
        }
        ui.begin(scope(1), false);
        assert_eq!(
            ui.pending_focus(LoginPhase::Ready, StorageStatus::NotSaved),
            Some(FocusStep::Permission(PermissionKind::Microphone))
        );
        ui.focused_step = Some(FocusStep::Permission(PermissionKind::Microphone));
        assert_eq!(
            ui.pending_focus(LoginPhase::Ready, StorageStatus::NotSaved),
            None
        );
        ui.enter(PermissionKind::Camera);
        assert_eq!(
            ui.pending_focus(LoginPhase::Ready, StorageStatus::NotSaved),
            Some(FocusStep::Permission(PermissionKind::Camera))
        );
        ui.ready = true;
        assert_eq!(
            ui.pending_focus(LoginPhase::Ready, StorageStatus::NotSaved),
            Some(FocusStep::Done)
        );
        ui.begin(scope(1), true);
        assert_eq!(
            ui.pending_focus(LoginPhase::EnterServer, StorageStatus::NotSaved),
            Some(FocusStep::Done)
        );
    }
    #[test]
    fn one_request_per_step_and_only_observed_grants_advance() {
        let mut ui = OnboardingUi::new();
        ui.begin(scope(1), false);
        let mut observation = PermissionObservation::unknown(PermissionKind::Microphone);
        observation.status = PermissionStatus::NotDetermined;
        observation.can_request = true;
        assert_eq!(ui.advance(observation, true), Advance::Wait);
        assert!(ui.complete(scope(1), ui.entry, ui.kind, PermissionAction::Check, true));
        assert_eq!(ui.advance(observation, true), Advance::Request);
        ui.requested[0] = true;
        assert_eq!(ui.advance(observation, true), Advance::Wait);
        observation.status = PermissionStatus::Denied;
        assert_eq!(ui.advance(observation, true), Advance::Wait);
        observation.status = PermissionStatus::Granted;
        assert_eq!(ui.advance(observation, false), Advance::Wait);
        assert_eq!(ui.advance(observation, true), Advance::Next);
        ui.enter(PermissionKind::Camera);
        ui.enter(PermissionKind::Microphone);
        ui.checked[0] = true;
        observation.status = PermissionStatus::NotDetermined;
        assert_eq!(ui.advance(observation, true), Advance::Wait);
    }
    #[test]
    fn stale_failed_and_resumed_completions_never_advance_or_prompt() {
        let mut ui = OnboardingUi::new();
        ui.begin(scope(1), false);
        let old = ui.entry;
        ui.enter(PermissionKind::Camera);
        assert!(!ui.complete(
            scope(1),
            old,
            PermissionKind::Microphone,
            PermissionAction::Request,
            true
        ));
        assert!(!ui.complete(scope(2), ui.entry, ui.kind, PermissionAction::Check, true));
        ui.complete(scope(1), ui.entry, ui.kind, PermissionAction::Check, false);
        let mut observation = PermissionObservation::unknown(PermissionKind::Camera);
        observation.can_request = true;
        assert_eq!(ui.advance(observation, true), Advance::Wait);
        ui.begin(scope(1), true);
        assert_eq!(ui.advance(observation, true), Advance::Wait);
    }
}
