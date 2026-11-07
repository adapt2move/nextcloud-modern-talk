//! Account-scoped, presentation-only status picker. Network work stays with the account owner.
use super::{ButtonStyle, Desktop, LocalAction, controls::Availability, probes::Probe};
use gpui::{Context, Div, Window, div, prelude::*, px};
use talk_core::preferences::{
    RequestId,
    account::{
        AccountPreferenceChange, AccountPreferenceCommand, AccountPreferenceError,
        AccountPreferenceSnapshot, AccountPreferenceState, AccountPreferences, StatusChoice,
    },
};
use talk_core::ui::presentation::{AccountConnectionState, Presence};

type OptimisticStatus = (talk_core::ui::AccountScope, RequestId, StatusChoice);

fn visible_status(
    scope: talk_core::ui::AccountScope,
    snapshot: &AccountPreferenceSnapshot,
    optimistic: Option<OptimisticStatus>,
) -> Option<StatusChoice> {
    if snapshot.scope != scope
        || snapshot
            .values
            .as_ref()
            .is_none_or(|v| v.scope != scope || v.user_status_enabled != Some(true))
    {
        return None;
    }
    if let Some((pending_scope, request, choice)) = optimistic
        && pending_scope == scope
        && snapshot.state == (AccountPreferenceState::Saving { request })
    {
        return Some(choice);
    }
    snapshot.values.as_ref()?.user_presence
}

impl Desktop {
    pub(super) fn fixture_own_status(&self) -> Option<AccountPreferenceSnapshot> {
        let scope = self.snapshot.active_account?;
        Some(AccountPreferenceSnapshot {
            scope,
            values: Some(AccountPreferences {
                scope,
                send_read_receipts: Some(true),
                share_typing_status: Some(true),
                attachment_folder: None,
                attachments_allowed: None,
                conversation_subfolders: None,
                user_status_enabled: Some(true),
                user_presence: Some(StatusChoice::Away),
                user_status_observed: true,
                user_status_emoji: Some(true),
                status_message: None,
                predefined_statuses: Some({
                    let mut page = talk_core::ui::Page::empty();
                    let _ = page.try_push(talk_core::preferences::account::PredefinedStatus {
                        id: talk_core::ui::Text::new("meeting").expect("fixture"),
                        icon: talk_core::ui::Text::new("📅").expect("fixture"),
                        message: talk_core::ui::Text::new("Im Termin").expect("fixture"),
                    });
                    page
                }),
            }),
            state: AccountPreferenceState::Ready,
        })
    }

    pub(super) fn own_status_value(&self) -> Option<StatusChoice> {
        let scope = self.snapshot.active_account?;
        visible_status(
            scope,
            self.account_preferences.as_ref()?,
            self.own_status_optimistic,
        )
    }

    pub(super) fn own_presence(&self) -> Presence {
        match self.own_status_value() {
            Some(StatusChoice::Online) => Presence::Available,
            Some(StatusChoice::Away) => Presence::Away,
            Some(StatusChoice::DoNotDisturb) => Presence::DoNotDisturb,
            Some(StatusChoice::Invisible | StatusChoice::Offline) => Presence::Offline,
            None => Presence::Unknown,
        }
    }

    pub(super) fn toggle_own_status(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.snapshot.active_account.is_none() {
            return;
        }
        self.own_status_open = !self.own_status_open;
        if self.own_status_open {
            self.own_status_focus = true;
            if !(self.development && matches!(self.probe, Probe::Status | Probe::StatusMessage))
                && !self.account_preferences.as_ref().is_some_and(|p| {
                    matches!(
                        p.state,
                        AccountPreferenceState::Loading { .. }
                            | AccountPreferenceState::Saving { .. }
                    )
                })
            {
                // Keep the last status interactive while a bounded refresh is in flight.
                self.load_account_preferences(cx);
            }
        } else {
            self.own_status_editing = false;
            if let Some(focus) = self.control_focus.current(&"own-status-menu".into()) {
                focus.focus(window);
            }
        }
    }

    pub(super) fn set_own_status(&mut self, choice: StatusChoice, cx: &mut Context<Self>) {
        let Some(snapshot) = self.account_preferences.as_ref().filter(|p| {
            Some(p.scope) == self.snapshot.active_account
                && p.values
                    .as_ref()
                    .is_some_and(|v| v.user_status_enabled == Some(true))
                && matches!(
                    p.state,
                    AccountPreferenceState::Ready | AccountPreferenceState::Loading { .. }
                )
        }) else {
            return;
        };
        let scope = snapshot.scope;
        if self.own_status_value() == Some(choice) {
            return;
        }
        if self.development && matches!(self.probe, Probe::Status | Probe::StatusMessage) {
            if let Some(values) = self
                .account_preferences
                .as_mut()
                .and_then(|p| p.values.as_mut())
            {
                values.user_presence = Some(choice);
            }
            cx.notify();
            return;
        }
        let Some(number) = self.account_preference_request.checked_add(1) else {
            return;
        };
        self.account_preference_request = number;
        let request = RequestId(number);
        match self
            .settings_port
            .submit_account(AccountPreferenceCommand::Set {
                scope,
                request,
                change: AccountPreferenceChange::UserPresence(choice),
            }) {
            Ok(()) => {
                self.account_preference_error = None;
                self.own_status_optimistic = Some((scope, request, choice));
            }
            Err(AccountPreferenceError::Busy) => {}
            Err(error) => self.account_preference_error = Some(error),
        }
        self.refresh_account_preferences(cx);
    }

    pub(super) fn own_status_menu(&self, cx: &Context<Self>) -> Div {
        if self.own_status_editing {
            return self.status_message_editor(cx);
        }
        let t = self.theme;
        let snapshot = self
            .account_preferences
            .as_ref()
            .filter(|p| Some(p.scope) == self.snapshot.active_account);
        let values = snapshot.and_then(|p| p.values.as_ref());
        let loading =
            snapshot.is_some_and(|p| matches!(p.state, AccountPreferenceState::Loading { .. }));
        let ready = snapshot.is_some_and(|p| {
            matches!(
                p.state,
                AccountPreferenceState::Ready | AccountPreferenceState::Loading { .. }
            ) && p.values.is_some()
        });
        let failed = self.account_preference_error.or_else(|| {
            snapshot.and_then(|p| match p.state {
                AccountPreferenceState::Failed { error, .. } => Some(error),
                _ => None,
            })
        });
        let mut menu = div()
            .absolute()
            .bottom(px(58.))
            .left_0()
            .right_0()
            .occlude()
            .rounded(px(12.))
            .border_1()
            .border_color(t.border)
            .bg(t.surface)
            .shadow_lg()
            .p_2()
            .flex()
            .flex_col()
            .gap_1()
            .child(
                div()
                    .px_2()
                    .py_1()
                    .text_size(self.sp(12.))
                    .text_color(t.muted)
                    .child(self.l("Status festlegen")),
            );
        if values.is_some_and(|v| v.user_status_enabled == Some(true)) {
            for (index, choice, label, color) in [
                (
                    0usize,
                    StatusChoice::Online,
                    "Verfügbar",
                    t.presence_dot(Presence::Available),
                ),
                (
                    1,
                    StatusChoice::Away,
                    "Abwesend",
                    t.presence_dot(Presence::Away),
                ),
                (
                    2,
                    StatusChoice::DoNotDisturb,
                    "Nicht stören",
                    t.presence_dot(Presence::DoNotDisturb),
                ),
                (3, StatusChoice::Invisible, "Unsichtbar", t.muted),
            ] {
                let selected = self.own_status_value() == Some(choice);
                menu = menu.child(
                    self.button(
                        ("own-status-choice", index),
                        "",
                        ButtonStyle::Quiet,
                        Availability::when(ready, "Status wird aktualisiert."),
                        LocalAction::SetOwnStatus(choice),
                        cx,
                    )
                    .w_full()
                    .justify_start()
                    .gap_2()
                    .px_2()
                    .child(div().size(px(9.)).rounded_full().bg(color))
                    .child(div().flex_1().child(self.l(label)))
                    .when(selected, |d| {
                        d.child(self.scaled_icon("check", 14., t.accent))
                    }),
                );
            }
        } else if loading {
            menu = menu.child(self.hint(self.l("Status wird geladen …")).px_2().py_2());
        } else if failed.is_none() && values.is_some() {
            menu = menu.child(
                self.hint(self.l("Diese Einstellung wird vom Server nicht angeboten."))
                    .px_2()
                    .py_2(),
            );
        } else if failed.is_none() {
            menu = menu.child(
                self.hint(self.l("Für dieses Konto besteht keine aktive Serververbindung."))
                    .px_2()
                    .py_2(),
            );
            menu = menu.child(
                self.button(
                    "own-status-retry",
                    "Erneut laden",
                    ButtonStyle::Quiet,
                    true,
                    LocalAction::ReloadAccountPreferences,
                    cx,
                )
                .w_full()
                .justify_start(),
            );
        }
        if values.is_some_and(|v| v.user_status_enabled == Some(true)) {
            let label = self
                .visible_status_message()
                .map(|m| {
                    [
                        m.icon.as_ref().map(|s| s.as_str()),
                        m.message.as_ref().map(|s| s.as_str()),
                    ]
                    .into_iter()
                    .flatten()
                    .filter(|s| !s.trim().is_empty())
                    .collect::<Vec<_>>()
                    .join(" ")
                })
                .filter(|s| !s.is_empty())
                .unwrap_or_else(|| self.l("Statusnachricht festlegen").to_owned());
            menu = menu.child(div().h(px(1.)).bg(t.border)).child(
                self.button(
                    "edit-status-message",
                    "",
                    ButtonStyle::Quiet,
                    ready,
                    LocalAction::EditStatusMessage,
                    cx,
                )
                .w_full()
                .justify_start()
                .px_2()
                .child(self.scaled_icon("chat", 16., t.muted))
                .child(div().min_w_0().text_ellipsis().child(label)),
            );
        }
        if let Some(error) = failed {
            let reason = match error {
                AccountPreferenceError::Network => {
                    "Der Server ist nicht erreichbar. Bitte erneut laden."
                }
                AccountPreferenceError::Uncertain => {
                    "Die Änderung konnte nicht bestätigt werden. Bitte neu laden."
                }
                AccountPreferenceError::PermissionDenied => {
                    "Der Server erlaubt diese Änderung nicht."
                }
                AccountPreferenceError::AuthenticationRequired => {
                    "Bitte melde dich bei diesem Konto erneut an."
                }
                _ => "Für dieses Konto besteht keine aktive Serververbindung.",
            };
            menu = menu.child(self.hint(self.l(reason)).px_2().py_1());
            menu = menu.child(
                self.button(
                    "own-status-retry",
                    "Erneut laden",
                    ButtonStyle::Quiet,
                    true,
                    LocalAction::ReloadAccountPreferences,
                    cx,
                )
                .w_full()
                .justify_start(),
            );
        }
        menu
    }

    pub(super) fn own_status_connected(&self) -> bool {
        self.snapshot.accounts.as_slice().iter().any(|account| {
            Some(account.scope) == self.snapshot.active_account
                && account.connection == AccountConnectionState::Connected
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use talk_core::ui::{AccountId, RuntimeGeneration};
    #[test]
    fn optimism_requires_exact_scope_and_pending_request_and_never_grants_capability() {
        let scope = talk_core::ui::AccountScope {
            account: AccountId::from_local(1),
            runtime: RuntimeGeneration::from_local(2),
        };
        let mut snapshot = AccountPreferenceSnapshot {
            scope,
            values: Some(AccountPreferences {
                scope,
                send_read_receipts: None,
                share_typing_status: None,
                attachment_folder: None,
                attachments_allowed: None,
                conversation_subfolders: None,
                user_status_enabled: Some(true),
                user_presence: Some(StatusChoice::Away),
                user_status_observed: true,
                user_status_emoji: Some(true),
                status_message: None,
                predefined_statuses: None,
            }),
            state: AccountPreferenceState::Saving {
                request: RequestId(3),
            },
        };
        let optimistic = Some((scope, RequestId(3), StatusChoice::DoNotDisturb));
        assert_eq!(
            visible_status(scope, &snapshot, optimistic),
            Some(StatusChoice::DoNotDisturb)
        );
        snapshot.state = AccountPreferenceState::Ready;
        assert_eq!(
            visible_status(scope, &snapshot, optimistic),
            Some(StatusChoice::Away)
        );
        snapshot.state = AccountPreferenceState::Saving {
            request: RequestId(4),
        };
        assert_eq!(
            visible_status(scope, &snapshot, optimistic),
            Some(StatusChoice::Away)
        );
        let mut other = scope;
        other.runtime = RuntimeGeneration::from_local(3);
        assert_eq!(visible_status(other, &snapshot, optimistic), None);
        snapshot.values.as_mut().unwrap().user_status_enabled = Some(false);
        assert_eq!(visible_status(scope, &snapshot, optimistic), None);
    }
}
