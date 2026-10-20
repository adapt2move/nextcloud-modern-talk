use super::state::{Advance, index};
use crate::presentation::Desktop;
use gpui::Context;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};
use talk_core::preferences::{
    LoadState, OnboardingResume, SaveState,
    permissions::{PermissionAction, PermissionKind},
};
use talk_core::ui::AccountScope;
use talk_core::ui::presentation::{LoginPhase, UiIntent};

fn now_seconds() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |time| time.as_secs())
}

impl Desktop {
    pub(in crate::presentation) fn onboarding_permission_owner(
        &self,
        kind: PermissionKind,
    ) -> Option<(AccountScope, u64)> {
        (self.onboarding_open
            && self.overlay.is_none()
            && !self.onboarding_ui.ready
            && self.onboarding_ui.kind == kind
            && self.onboarding_ui.scope == self.snapshot.active_account)
            .then(|| {
                self.onboarding_ui
                    .scope
                    .map(|scope| (scope, self.onboarding_ui.entry))
            })
            .flatten()
    }

    pub(in crate::presentation) fn onboarding_permission_completed(
        &mut self,
        owner: (AccountScope, u64),
        kind: PermissionKind,
        action: PermissionAction,
        succeeded: bool,
    ) {
        if self
            .onboarding_ui
            .complete(owner.0, owner.1, kind, action, succeeded)
            && succeeded
            && matches!(
                action,
                PermissionAction::Request | PermissionAction::OpenSystemSettings
            )
        {
            if action == PermissionAction::OpenSystemSettings {
                // The user chose the Settings route; do not race it with a prompt.
                self.onboarding_ui.requested[index(kind)] = true;
            }
            // A bounded, read-only follow-up observes a grant made in Settings.
            // It never reissues a prompt or opens a capture device.
            self.onboarding_ui.rechecks_left = 60;
            self.onboarding_ui.recheck_at = Some(Instant::now() + Duration::from_secs(2));
        }
    }

    pub(in crate::presentation) fn onboarding_checkpoint_ready(&self) -> bool {
        if !cfg!(target_os = "macos") {
            return true;
        }
        self.preferences
            .confirmed
            .onboarding_resume
            .as_ref()
            .is_some_and(|resume| {
                resume.is_current(now_seconds())
                    && self.snapshot.accounts.as_slice().iter().any(|account| {
                        Some(account.scope) == self.onboarding_ui.scope
                            && account.navigation_key.as_ref() == Some(&resume.account)
                    })
            })
    }

    fn arm_onboarding_checkpoint(&mut self, cx: &mut Context<Self>) -> bool {
        if self.onboarding_checkpoint_ready() {
            return true;
        }
        if !self.onboarding_ui.checkpoint_attempted {
            self.onboarding_ui.checkpoint_attempted = true;
            if let Some(account) = self
                .snapshot
                .accounts
                .as_slice()
                .iter()
                .find(|account| Some(account.scope) == self.onboarding_ui.scope)
                .and_then(|account| account.navigation_key.clone())
            {
                let mut values = self.preferences.active.clone();
                values.onboarding_resume = Some(OnboardingResume {
                    account,
                    expires_at_unix_seconds: now_seconds()
                        .saturating_add(OnboardingResume::LIFETIME_SECONDS),
                });
                self.commit_preferences(values, cx);
            }
        }
        false
    }

    pub(in crate::presentation) fn prepare_onboarding_permission(
        &mut self,
        kind: PermissionKind,
        action: PermissionAction,
        cx: &mut Context<Self>,
    ) -> bool {
        if kind == PermissionKind::ScreenSharing
            && action != PermissionAction::Check
            && self.onboarding_permission_owner(kind).is_some()
        {
            return self.arm_onboarding_checkpoint(cx);
        }
        true
    }

    fn clear_onboarding_checkpoint(&mut self, cx: &mut Context<Self>) {
        if self.preferences.confirmed.onboarding_resume.is_none()
            && self.preferences.active.onboarding_resume.is_none()
        {
            self.onboarding_ui.clearing = false;
        } else if !self.onboarding_ui.clearing {
            self.onboarding_ui.clearing = true;
            let mut values = self.preferences.active.clone();
            values.onboarding_resume = None;
            self.commit_preferences(values, cx);
        }
    }

    pub(in crate::presentation) fn reset_onboarding(&mut self, cx: &mut Context<Self>) {
        self.onboarding_ui.initial_resume = None;
        self.onboarding_ui.startup_read = true;
        self.onboarding_ui.scope = None;
        self.onboarding_ui.ready = false;
        self.onboarding_ui.resumed = false;
        self.onboarding_ui.entry = self.onboarding_ui.entry.saturating_add(1);
        self.onboarding_ui.recheck_at = None;
        self.clear_onboarding_checkpoint(cx);
    }

    pub(in crate::presentation) fn finish_onboarding(&mut self, cx: &mut Context<Self>) {
        if self.snapshot.active_account.is_none() {
            return;
        }
        self.onboarding_ui.ready = true;
        self.onboarding_ui.finish_pending = true;
        self.onboarding_ui.entry = self.onboarding_ui.entry.saturating_add(1);
        self.onboarding_ui.recheck_at = None;
        self.clear_onboarding_checkpoint(cx);
        if !self.onboarding_ui.clearing {
            self.onboarding_open = false;
            self.onboarding_ui.finish_pending = false;
        }
    }

    pub(in crate::presentation) fn retry_onboarding_handoff(&mut self, cx: &mut Context<Self>) {
        self.onboarding_ui.checkpoint_attempted = false;
        if self.onboarding_ui.clearing {
            self.onboarding_ui.clearing = false;
            self.clear_onboarding_checkpoint(cx);
        } else {
            self.arm_onboarding_checkpoint(cx);
        }
    }

    pub(in crate::presentation) fn enter_onboarding_step(&mut self, kind: PermissionKind) {
        self.onboarding_ui.enter(kind);
        self.navigation.reset_onboarding();
    }

    fn next_onboarding_step(&mut self) {
        match self.onboarding_ui.kind {
            PermissionKind::Microphone => self.enter_onboarding_step(PermissionKind::Camera),
            PermissionKind::Camera => self.enter_onboarding_step(PermissionKind::ScreenSharing),
            PermissionKind::ScreenSharing => {
                self.onboarding_ui.ready = true;
                self.onboarding_ui.entry = self.onboarding_ui.entry.saturating_add(1);
                self.onboarding_ui.recheck_at = None;
                self.navigation.reset_onboarding();
            }
        }
    }

    pub(in crate::presentation) fn refresh_onboarding(&mut self, cx: &mut Context<Self>) {
        if !self.onboarding_ui.startup_read && self.preferences.load != LoadState::Loading {
            self.onboarding_ui.startup_read = true;
            self.onboarding_ui.initial_resume =
                self.preferences.confirmed.onboarding_resume.clone();
        }
        if let Some(resume) = self.onboarding_ui.initial_resume.clone() {
            if !cfg!(target_os = "macos") || !resume.is_current(now_seconds()) {
                self.onboarding_ui.initial_resume = None;
                self.clear_onboarding_checkpoint(cx);
            } else if self.snapshot.login.attempt.is_none()
                && let Some(scope) = self
                    .snapshot
                    .accounts
                    .as_slice()
                    .iter()
                    .find(|account| account.navigation_key.as_ref() == Some(&resume.account))
                    .map(|account| account.scope)
            {
                if self.snapshot.active_account != Some(scope) {
                    self.submit(UiIntent::SelectAccount(scope));
                    return;
                }
                self.onboarding_ui.initial_resume = None;
                self.onboarding_ui.begin(scope, true);
                self.onboarding_open = true;
                self.overlay = None;
                // A relaunch resumes navigation, never grants permission. Re-read
                // each OS state without Request, capture or preview.
                for kind in [
                    PermissionKind::Microphone,
                    PermissionKind::Camera,
                    PermissionKind::ScreenSharing,
                ] {
                    self.queue_permission_check(kind);
                }
                self.clear_onboarding_checkpoint(cx);
                cx.notify();
            }
        }
        if self.onboarding_ui.clearing
            && self.preferences.confirmed.onboarding_resume.is_none()
            && self.preferences.active.onboarding_resume.is_none()
            && !matches!(self.preferences.save, SaveState::Pending { .. })
        {
            self.onboarding_ui.clearing = false;
            if self.onboarding_ui.finish_pending {
                self.onboarding_open = false;
                self.onboarding_ui.finish_pending = false;
            }
            cx.notify();
        }
        if !self.onboarding_open || self.overlay.is_some() {
            return;
        }
        let Some(scope) = self.snapshot.active_account else {
            if self.onboarding_ui.scope.is_some() {
                self.onboarding_ui.resumed = false;
                self.onboarding_open = false;
            }
            return;
        };
        if self.onboarding_ui.scope.is_some() && self.onboarding_ui.scope != Some(scope) {
            // A different active account cannot inherit this setup or its prompts.
            self.onboarding_ui.resumed = false;
            self.onboarding_open = false;
            return;
        }
        if self.snapshot.login.phase != LoginPhase::Ready && !self.onboarding_ui.resumed {
            return;
        }
        if self.onboarding_ui.scope != Some(scope) {
            self.onboarding_ui.begin(scope, false);
        }
        if self.onboarding_ui.ready {
            return;
        }
        let kind = self.onboarding_ui.kind;
        if !self.onboarding_ui.queued[index(kind)] {
            self.onboarding_ui.queued[index(kind)] = true;
            self.queue_permission_check(kind);
        }
        let idle = self.permission_operation_idle();
        let observation = self.observation(kind);
        match self.onboarding_ui.advance(observation, idle) {
            Advance::Next => {
                self.next_onboarding_step();
                cx.notify();
            }
            Advance::Request
                if self.prepare_onboarding_permission(kind, PermissionAction::Request, cx) =>
            {
                self.onboarding_ui.requested[index(kind)] = true;
                self.permission_action(kind, PermissionAction::Request);
                cx.notify();
            }
            _ => {}
        }
        if kind == PermissionKind::ScreenSharing && observation.can_open_settings {
            // Also arm before the user can follow an OS Settings route.
            self.arm_onboarding_checkpoint(cx);
        }
        if idle
            && !self.onboarding_ui.ready
            && self.onboarding_ui.rechecks_left > 0
            && self
                .onboarding_ui
                .recheck_at
                .is_some_and(|time| Instant::now() >= time)
        {
            self.onboarding_ui.rechecks_left -= 1;
            self.onboarding_ui.recheck_at = Some(Instant::now() + Duration::from_secs(2));
            self.queue_permission_check(kind);
        }
    }
}
