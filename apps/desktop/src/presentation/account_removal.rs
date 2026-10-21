//! Account removal is a confirmed typed intent; only runtime snapshots remove rows.
use super::{ButtonStyle, Desktop, LocalAction, Overlay, controls::Availability};
use gpui::{Context, Div, FocusHandle, FontWeight, Window, div, prelude::*, px};
use talk_core::preferences::navigation::NavigationAccountKey;
use talk_core::ui::presentation::{
    AccountRemovalFailure, AccountRemovalResult, PresentationSnapshot, RequestId,
    SavedAccountPhase, UiIntent,
};
use talk_core::ui::{AccountId, AccountScope};

#[derive(Clone)]
struct Target {
    account: AccountId,
    scope: Option<AccountScope>,
    label: String,
}

pub(super) struct AccountRemovalUi {
    confirmation: Option<Target>,
    pending: Option<(RequestId, Target)>,
    failure: Option<(Target, &'static str)>,
    request: u64,
    return_focus: Option<FocusHandle>,
    pub focus: FocusHandle,
    pub focus_cancel: bool,
}
impl AccountRemovalUi {
    pub fn new(cx: &gpui::App) -> Self {
        Self {
            confirmation: None,
            pending: None,
            failure: None,
            request: 0,
            return_focus: None,
            focus: cx.focus_handle(),
            focus_cancel: false,
        }
    }
    pub fn confirming(&self) -> bool {
        self.confirmation.is_some()
    }
    pub fn busy(&self) -> bool {
        self.pending.is_some()
    }
    pub fn dismiss_confirmation(&mut self) {
        self.confirmation = None;
        self.return_focus = None;
        self.focus_cancel = false;
    }
    fn complete(&mut self, result: AccountRemovalResult) -> bool {
        if !self.pending.as_ref().is_some_and(|(request, target)| {
            *request == result.request && target.account == result.account
        }) {
            return false;
        }
        if let Some((_, target)) = self.pending.take() {
            self.failure = result
                .result
                .err()
                .map(|failure| (target, removal_message(failure)));
        }
        true
    }
}

fn removal_message(error: AccountRemovalFailure) -> &'static str {
    match error {
        AccountRemovalFailure::NativeLocked if cfg!(target_os = "macos") => {
            "Der macOS-Schlüsselbund ist gesperrt. Entsperre den Anmeldeschlüsselbund in der Schlüsselbundverwaltung und versuche das Entfernen erneut."
        }
        AccountRemovalFailure::NativeDenied if cfg!(target_os = "macos") => {
            "macOS hat den Zugriff auf den Schlüsselbund verweigert. Erlaube dieser App den Zugriff auf ihre gespeicherte Anmeldung und versuche das Entfernen erneut."
        }
        AccountRemovalFailure::LocalDenied => {
            "Die lokale Anmeldedatei konnte wegen fehlender Schreibrechte nicht gelöscht werden. Prüfe die Zugriffsrechte des App-Datenordners und versuche es erneut."
        }
        AccountRemovalFailure::LocalFile => {
            "Die lokale Anmeldedatei konnte nicht sicher entfernt werden. Bitte versuche es erneut. Der Fehler steht im lokalen Diagnoseprotokoll."
        }
        AccountRemovalFailure::NativeLocked => {
            "Der Schlüsselspeicher ist gesperrt. Entsperre ihn und versuche das Entfernen erneut."
        }
        AccountRemovalFailure::NativeDenied => {
            "Der Zugriff auf den Schlüsselspeicher wurde verweigert. Erlaube der App den Zugriff und versuche das Entfernen erneut."
        }
        AccountRemovalFailure::NativeCancelled => {
            "Die Freigabe für den Schlüsselspeicher wurde abgebrochen. Versuche das Entfernen erneut und bestätige die Systemabfrage."
        }
        AccountRemovalFailure::NativeUnavailable => {
            "Der Schlüsselspeicher des Betriebssystems ist nicht verfügbar. Stelle den Zugriff wieder her und versuche das Entfernen erneut."
        }
        AccountRemovalFailure::NativeFailed => {
            "Die gespeicherte Anmeldung konnte nicht sicher entfernt werden. Bitte versuche es erneut. Der Speicherfehler steht im lokalen Diagnoseprotokoll."
        }
        AccountRemovalFailure::LocalState => {
            "Der lokale Kontostatus konnte nicht gespeichert oder gelesen werden. Prüfe freien Speicher und Schreibrechte des App-Datenordners und versuche es erneut."
        }
        AccountRemovalFailure::SuppressionUnconfirmed => {
            "Die Verbindung wurde getrennt, aber die Abmeldung konnte nicht dauerhaft gespeichert werden. Bitte versuche es erneut; nach einem Neustart könnte sich das Konto sonst wieder verbinden."
        }
        AccountRemovalFailure::CleanupPending => {
            "Ein früherer Speichervorgang für diese Anmeldung ist noch nicht abgeschlossen. Das Konto bleibt abgemeldet. Warte kurz und versuche das Entfernen erneut."
        }
        AccountRemovalFailure::TimedOut => {
            "Das Entfernen dauert länger als erwartet. Es wurde noch nicht als abgeschlossen bestätigt. Warte kurz und versuche es erneut."
        }
        AccountRemovalFailure::Busy => {
            "Der Kontospeicher wird gerade verwendet. Warte kurz und versuche das Entfernen erneut."
        }
        AccountRemovalFailure::Cancelled => {
            "Das Entfernen wurde unterbrochen. Bitte versuche es erneut."
        }
        AccountRemovalFailure::Changed => "Das Konto hat sich geändert. Bitte wähle es erneut aus.",
        AccountRemovalFailure::Unavailable => {
            "Die Kontoverwaltung ist nicht verfügbar. Bitte starte die App neu und versuche das Entfernen erneut."
        }
    }
}

fn target_is_current(
    snapshot: &PresentationSnapshot,
    account: AccountId,
    scope: Option<AccountScope>,
) -> bool {
    target_matches(
        account,
        scope,
        snapshot
            .accounts
            .as_slice()
            .iter()
            .find(|a| a.scope.account == account)
            .map(|a| a.scope),
        snapshot
            .saved_accounts
            .as_slice()
            .iter()
            .find(|a| a.id == account)
            .map(|a| a.phase),
    )
}
fn target_matches(
    account: AccountId,
    scope: Option<AccountScope>,
    live: Option<AccountScope>,
    saved: Option<SavedAccountPhase>,
) -> bool {
    match scope {
        Some(scope) => scope.account == account && live == Some(scope),
        None => {
            live.is_none()
                && matches!(
                    saved,
                    Some(
                        SavedAccountPhase::Dormant
                            | SavedAccountPhase::ReauthenticationRequired
                            | SavedAccountPhase::RecoveryRequired
                    )
                )
        }
    }
}

fn retry_scope(snapshot: &PresentationSnapshot, target: &Target) -> Option<Option<AccountScope>> {
    if target_is_current(snapshot, target.account, target.scope) {
        Some(target.scope)
    } else if target.scope.is_some() && target_is_current(snapshot, target.account, None) {
        // A failed logout may leave a dormant cleanup record. A new explicit
        // confirmation may forget it, but never adopts another live generation.
        Some(None)
    } else {
        None
    }
}

fn removal_clears_resume(
    account: AccountId,
    scope: Option<AccountScope>,
    checkpoint: &NavigationAccountKey,
    live: Option<(AccountScope, &NavigationAccountKey)>,
    saved: Option<(AccountId, &NavigationAccountKey)>,
) -> bool {
    // Account IDs are process-local. Only an authoritative matching durable key
    // can retire a checkpoint, including when the removal targets a dormant row.
    match scope {
        Some(scope) => live.is_some_and(|(current, key)| {
            scope.account == account && current == scope && key == checkpoint
        }),
        None => saved.is_some_and(|(current, key)| current == account && key == checkpoint),
    }
}

impl Desktop {
    pub(super) fn request_account_removal(
        &mut self,
        account: AccountId,
        scope: Option<AccountScope>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.account_removal.busy()
            || !(self.overlay == Some(Overlay::Accounts)
                || (self.overlay == Some(Overlay::Settings)
                    && self.settings_tab == super::preferences::SettingsTab::Accounts))
            || !target_is_current(&self.snapshot, account, scope)
        {
            return;
        }
        let label = self
            .snapshot
            .accounts
            .as_slice()
            .iter()
            .find(|a| a.scope.account == account)
            .map(|a| self.account_name(a))
            .or_else(|| {
                self.snapshot
                    .saved_accounts
                    .as_slice()
                    .iter()
                    .find(|a| a.id == account)
                    .and_then(|a| a.label.as_ref())
                    .map(|v| v.as_str().to_owned())
            })
            .unwrap_or_else(|| self.l("Gespeichertes Konto").to_owned());
        self.account_removal.confirmation = Some(Target {
            account,
            scope,
            label,
        });
        self.account_removal.return_focus = window.focused(cx);
        self.account_removal.focus_cancel = true;
        self.hover_hints.cancel();
        cx.notify();
    }
    pub(super) fn cancel_account_removal(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.account_removal.confirmation = None;
        self.account_removal.focus_cancel = false;
        if let Some(focus) = self.account_removal.return_focus.take() {
            focus.focus(window);
        }
        cx.notify();
    }
    pub(super) fn confirm_account_removal(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(target) = self.account_removal.confirmation.take() else {
            return;
        };
        self.cancel_account_removal(window, cx);
        if self.account_removal.busy() {
            return;
        }
        if !target_is_current(&self.snapshot, target.account, target.scope) {
            self.account_removal.failure = Some((
                target,
                "Das Konto hat sich geändert. Bitte wähle es erneut aus.",
            ));
            return;
        }
        let Some(next) = self.account_removal.request.checked_add(1) else {
            self.account_removal.failure = Some((
                target,
                "Bitte starte die App neu, bevor du weitere Konten entfernst.",
            ));
            return;
        };
        if !self.save_draft(cx) {
            return;
        }
        if let Some(result) = self.snapshot.account_removal
            && self
                .port
                .try_submit(UiIntent::AcknowledgeAccountRemoval {
                    request: result.request,
                    account: result.account,
                })
                .is_err()
        {
            self.account_removal.failure = Some((target, "Bitte warte auf den laufenden Vorgang."));
            return;
        }
        let clear_resume = self
            .preferences
            .active
            .onboarding_resume
            .as_ref()
            .is_some_and(|resume| {
                removal_clears_resume(
                    target.account,
                    target.scope,
                    &resume.account,
                    self.snapshot
                        .accounts
                        .as_slice()
                        .iter()
                        .find(|a| a.scope.account == target.account)
                        .and_then(|a| a.navigation_key.as_ref().map(|key| (a.scope, key))),
                    self.snapshot
                        .saved_accounts
                        .as_slice()
                        .iter()
                        .find(|a| a.id == target.account)
                        .map(|a| (a.id, &a.navigation_key)),
                )
            });
        let request = RequestId(next);
        self.account_removal.request = next;
        match self.port.try_submit(UiIntent::RemoveAccount {
            request,
            account: target.account,
            scope: target.scope,
        }) {
            Ok(()) => {
                self.account_removal.failure = None;
                self.account_removal.pending = Some((request, target));
                if clear_resume {
                    let mut values = self.preferences.active.clone();
                    values.onboarding_resume = None;
                    self.commit_preferences(values, cx);
                }
            }
            Err(error) => {
                let error = match error {
                    talk_core::ui::SubmitError::Busy => AccountRemovalFailure::Busy,
                    talk_core::ui::SubmitError::StaleScope => AccountRemovalFailure::Changed,
                    _ => AccountRemovalFailure::Unavailable,
                };
                self.account_removal.failure = Some((target, removal_message(error)));
            }
        }
    }
    pub(super) fn retry_account_removal(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if let Some((target, _)) = self.account_removal.failure.clone() {
            if let Some(scope) = retry_scope(&self.snapshot, &target) {
                self.request_account_removal(target.account, scope, window, cx);
            } else {
                self.account_removal.failure = Some((
                    target,
                    "Das Konto hat sich geändert. Bitte wähle es erneut aus.",
                ));
            }
        }
    }
    pub(super) fn refresh_account_removal(&mut self, snapshot: &PresentationSnapshot) {
        if let Some(result) = snapshot.account_removal {
            self.account_removal.complete(result);
            // A repeated snapshot is harmless; the runtime retains its one
            // outcome until this matching acknowledgement is admitted.
            let _ = self.port.try_submit(UiIntent::AcknowledgeAccountRemoval {
                request: result.request,
                account: result.account,
            });
        }
        if self
            .account_removal
            .confirmation
            .as_ref()
            .is_some_and(|target| !target_is_current(snapshot, target.account, target.scope))
        {
            self.account_removal.confirmation = None;
            self.account_removal.focus_cancel = false;
        }
    }
    pub(super) fn account_removal_feedback(&self, cx: &Context<Self>) -> Div {
        let mut area = div().flex().flex_col().gap_2();
        if let Some((target, message)) = &self.account_removal.failure {
            area = area.child(
                div()
                    .text_size(self.sp(12.))
                    .text_color(self.theme.danger)
                    .child(format!("{}: {}", target.label, self.l(message))),
            );
            if retry_scope(&self.snapshot, target).is_some() {
                area = area.child(self.button(
                    "retry-account-removal",
                    "Erneut versuchen",
                    ButtonStyle::Secondary,
                    Availability::when(
                        !self.account_removal.busy(),
                        "Bitte warte auf den laufenden Vorgang.",
                    ),
                    LocalAction::RetryAccountRemoval,
                    cx,
                ));
            }
        }
        area
    }
    pub(super) fn account_removal_dialog(
        &self,
        window: &Window,
        cx: &Context<Self>,
    ) -> Option<Div> {
        let target = self.account_removal.confirmation.as_ref()?;
        Some(div().absolute().inset_0().occlude().bg(self.theme.scrim)
            .flex().items_center().justify_center().p_6()
            .child(div().id("account-removal-dialog").track_focus(&self.account_removal.focus).tab_group()
                .w(px(440.)).max_w_full().rounded_xl().bg(self.theme.surface)
                .border_1().border_color(self.theme.border).shadow_xl()
                .child(self.scroll_area(super::navigation::Region::Overlay)
                .max_h(window.viewport_size().height - px(super::window_chrome::HEIGHT + 48.)).p_5()
                .child(div().flex().flex_col().gap_4()
                .child(div().text_size(self.sp(18.)).font_weight(FontWeight::MEDIUM)
                    .child(self.l("Konto aus dieser App entfernen?")))
                .child(div().font_weight(FontWeight::MEDIUM).child(target.label.clone()))
                .child(self.hint(self.l("Du wirst in dieser App abgemeldet und die gespeicherte Anmeldung wird entfernt. Dein NextCloud-Konto und gespeicherte Downloads bleiben erhalten.")))
                .when(self.snapshot.call.as_ref().is_some_and(|call| call.scope.account.account == target.account
                    && !matches!(call.stage, super::CallStage::Ended | super::CallStage::Failed)),
                    |d| d.child(self.hint(self.l("Der laufende Anruf wird beendet.")).text_color(self.theme.danger)))
                .when(self.drafts.has_unsent_for_account(target.account),
                    |d| d.child(self.hint(self.l("Lokale Entwürfe werden verworfen. Bereits gesendete Nachrichten können nicht zurückgerufen werden.")).text_color(self.theme.danger)))
                .child(div().flex().flex_wrap().justify_end().gap_2()
                    .child(self.button("cancel-account-removal", "Abbrechen", ButtonStyle::Secondary,
                        true, LocalAction::CancelAccountRemoval, cx))
                    .child(self.button("confirm-account-removal", "Aus dieser App entfernen", ButtonStyle::Danger,
                        true, LocalAction::ConfirmAccountRemoval, cx)))))))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use talk_core::ui::RuntimeGeneration;

    #[test]
    fn removal_preserves_another_accounts_onboarding_checkpoint() {
        let first = AccountScope {
            account: AccountId::from_local(1),
            runtime: RuntimeGeneration::from_local(1),
        };
        let second = AccountScope {
            account: AccountId::from_local(2),
            runtime: RuntimeGeneration::from_local(2),
        };
        let first_key = NavigationAccountKey::new(
            "a1:00000000000000000000000000000001:00000000000000000000000000000011",
        )
        .unwrap();
        let second_key = NavigationAccountKey::new(
            "a1:00000000000000000000000000000002:00000000000000000000000000000022",
        )
        .unwrap();
        // Removing B must retain A's checkpoint whether B is live or dormant.
        assert!(!removal_clears_resume(
            second.account,
            Some(second),
            &first_key,
            Some((second, &second_key)),
            None
        ));
        assert!(!removal_clears_resume(
            second.account,
            None,
            &first_key,
            None,
            Some((second.account, &second_key))
        ));
        // Both projections may clear only their own proven durable identity.
        assert!(removal_clears_resume(
            first.account,
            Some(first),
            &first_key,
            Some((first, &first_key)),
            None
        ));
        assert!(removal_clears_resume(
            first.account,
            None,
            &first_key,
            None,
            Some((first.account, &first_key))
        ));
        assert!(!removal_clears_resume(
            second.account,
            None,
            &first_key,
            None,
            Some((first.account, &first_key))
        ));
        assert!(!removal_clears_resume(
            first.account,
            None,
            &first_key,
            None,
            None
        ));
        let obsolete = AccountScope {
            runtime: RuntimeGeneration::from_local(99),
            ..first
        };
        assert!(!removal_clears_resume(
            first.account,
            Some(obsolete),
            &first_key,
            Some((first, &first_key)),
            None
        ));
    }

    #[test]
    fn removal_target_never_rebinds_a_dormant_record_or_old_runtime() {
        let account = AccountScope {
            account: AccountId::from_local(1),
            runtime: RuntimeGeneration::from_local(1),
        };
        let dormant = Some(SavedAccountPhase::Dormant);
        assert!(target_matches(
            account.account,
            Some(account),
            Some(account),
            dormant
        ));
        assert!(!target_matches(
            account.account,
            None,
            Some(account),
            dormant
        ));
        let obsolete = AccountScope {
            runtime: RuntimeGeneration::from_local(999),
            ..account
        };
        assert!(!target_matches(
            account.account,
            Some(obsolete),
            Some(account),
            dormant
        ));
        assert!(!target_matches(
            account.account,
            Some(account),
            None,
            dormant
        ));
        assert!(target_matches(account.account, None, None, dormant));
        assert!(!target_matches(
            account.account,
            None,
            None,
            Some(SavedAccountPhase::Restoring)
        ));
        assert!(!target_matches(account.account, None, None, None));
    }
    #[test]
    fn removal_feedback_distinguishes_keychain_denial_waiting_and_restart_risk() {
        let locked = removal_message(AccountRemovalFailure::NativeLocked);
        let denied = removal_message(AccountRemovalFailure::NativeDenied);
        let timeout = removal_message(AccountRemovalFailure::TimedOut);
        let pending = removal_message(AccountRemovalFailure::CleanupPending);
        let suppression = removal_message(AccountRemovalFailure::SuppressionUnconfirmed);
        assert!(locked.contains("entsperre") || locked.contains("Entsperre"));
        assert!(denied.contains("verweigert") && denied.contains("Erlaube"));
        assert!(timeout.contains("länger") && !timeout.contains("abgemeldet"));
        assert!(pending.contains("Speichervorgang") && pending.contains("abgemeldet"));
        assert!(suppression.contains("Neustart") && suppression.contains("wieder verbinden"));
        for key in [locked, denied, timeout, pending, suppression] {
            for locale in [
                talk_core::preferences::Locale::English,
                talk_core::preferences::Locale::Spanish,
            ] {
                assert_ne!(
                    crate::presentation::localization::text(locale, key),
                    key,
                    "all removal guidance needs en/de/es"
                );
            }
        }
    }
}
