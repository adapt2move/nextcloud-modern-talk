//! Finite setup state. Observations never grant capture authority.
use std::time::Instant;
use talk_core::preferences::{
    OnboardingResume,
    permissions::{PermissionAction, PermissionKind, PermissionObservation, PermissionStatus},
};
use talk_core::ui::AccountScope;

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
