//! Server-owned preferences; a projected fixture account never grants authority.
use super::controls::Availability;
use super::preferences::SettingsTab;
use super::{ButtonStyle, Desktop, LocalAction};
use gpui::{Context, Div, FontWeight, div, prelude::*};
use talk_core::preferences::{
    RequestId,
    account::{
        AccountPreferenceChange, AccountPreferenceCommand, AccountPreferenceError,
        AccountPreferenceState,
    },
};

impl Desktop {
    pub(super) fn refresh_account_preferences(&mut self, cx: &mut Context<Self>) {
        let next = self
            .snapshot
            .active_account
            .map(|scope| self.settings_port.account_snapshot(scope))
            .filter(|p| {
                Some(p.scope) == self.snapshot.active_account
                    && p.values.as_ref().is_none_or(|v| v.scope == p.scope)
            });
        if next == self.account_preferences {
            return;
        }
        let scope_changed =
            next.as_ref().map(|p| p.scope) != self.account_preferences.as_ref().map(|p| p.scope);
        let old_folder = self
            .account_preferences
            .as_ref()
            .and_then(|p| p.values.as_ref())
            .and_then(|v| v.attachment_folder.as_ref());
        let new_folder = next
            .as_ref()
            .and_then(|p| p.values.as_ref())
            .and_then(|v| v.attachment_folder.as_ref());
        if scope_changed || old_folder != new_folder {
            let value = new_folder.map_or("", |v| v.as_str()).to_owned();
            self.upload_folder.update(cx, |input, cx| {
                input.set_context_value(&value);
                cx.notify();
            });
        }
        if scope_changed {
            self.account_preference_error = None;
        }
        self.account_preferences = next;
        cx.notify();
    }
    pub(super) fn load_account_preferences(&mut self, cx: &mut Context<Self>) {
        let Some(scope) = self.snapshot.active_account else {
            return;
        };
        let Some(request) = self.account_preference_request.checked_add(1) else {
            return;
        };
        self.account_preference_request = request;
        self.account_preference_error = self
            .settings_port
            .submit_account(AccountPreferenceCommand::Load {
                scope,
                request: RequestId(request),
            })
            .err();
        self.refresh_account_preferences(cx);
    }
    pub(super) fn change_account_preference(
        &mut self,
        change: AccountPreferenceChange,
        cx: &mut Context<Self>,
    ) {
        let Some(p) = self.account_preferences.as_ref().filter(|p| {
            Some(p.scope) == self.snapshot.active_account
                && p.state == AccountPreferenceState::Ready
        }) else {
            return;
        };
        let Some(request) = self.account_preference_request.checked_add(1) else {
            return;
        };
        self.account_preference_request = request;
        self.account_preference_error = self
            .settings_port
            .submit_account(AccountPreferenceCommand::Set {
                scope: p.scope,
                request: RequestId(request),
                change,
            })
            .err();
        self.refresh_account_preferences(cx);
    }
    pub(super) fn account_settings_page(&self, tab: SettingsTab, cx: &Context<Self>) -> Div {
        let mut page = self
            .dialog_page()
            .child(self.dialog_page_header(tab.key(), None));
        if tab == SettingsTab::Privacy {
            page = page.child(self.dialog_section(Some("Bildschirmfreigabe")).child(
                self.permission_panel(
                    talk_core::preferences::permissions::PermissionKind::ScreenSharing,
                    cx,
                ),
            ));
        }
        // Account-owned values are grouped under the account they belong to.
        let mut section = self.dialog_section(Some("Kontoeinstellungen")).child(
            self.hint(self.l("Diese Einstellungen gehören zum ausgewählten NextCloud-Konto.")),
        );
        let Some(scope) = self.snapshot.active_account else {
            return page.child(section.child(self.hint(self.l("Noch kein Konto verbunden."))));
        };
        if let Some(account) = self
            .snapshot
            .accounts
            .as_slice()
            .iter()
            .find(|a| a.scope == scope)
        {
            section = section.child(
                div()
                    .flex()
                    .flex_wrap()
                    .gap_x_2()
                    .items_baseline()
                    .text_size(self.sp(13.))
                    .child(
                        div()
                            .font_weight(FontWeight::MEDIUM)
                            .child(self.account_name(account)),
                    )
                    .child(self.hint(account.server_label.as_str())),
            );
        }
        let snapshot = self
            .account_preferences
            .as_ref()
            .filter(|p| p.scope == scope);
        let state = snapshot.map_or(AccountPreferenceState::Unavailable, |p| p.state);
        let values = snapshot.and_then(|p| p.values.as_ref());
        let ready = state == AccountPreferenceState::Ready;
        let reason = match state {
            AccountPreferenceState::Unavailable => {
                "Für dieses Konto besteht keine aktive Serververbindung."
            }
            AccountPreferenceState::Loading { .. } => "Kontoeinstellungen werden geladen …",
            AccountPreferenceState::Saving { .. } => "Bitte warte auf den laufenden Vorgang.",
            AccountPreferenceState::Failed { error, .. } => account_error_key(error),
            AccountPreferenceState::Ready => "Diese Einstellung wird vom Server nicht angeboten.",
        };
        if tab == SettingsTab::Privacy {
            for (id, label, description, value, read) in [
                (
                    "privacy-read",
                    "Lesebestätigungen senden",
                    "Aus blendet alle Lesestatus aus.",
                    values.and_then(|v| v.send_read_receipts),
                    true,
                ),
                (
                    "privacy-typing",
                    "Schreibstatus teilen",
                    "Aus blendet alle Tippanzeigen aus. Nachrichten können weiterhin gesendet werden.",
                    values.and_then(|v| v.share_typing_status),
                    false,
                ),
            ] {
                section = section.child(self.dialog_row(
                    label,
                    Some(description),
                    self.switch(
                        id,
                        label,
                        value,
                        Availability::when(ready && value.is_some(), reason),
                        LocalAction::AccountPreference(if read {
                            AccountPreferenceChange::SendReadReceipts(value != Some(true))
                        } else {
                            AccountPreferenceChange::ShareTypingStatus(value != Some(true))
                        }),
                        cx,
                    ),
                ));
            }
        } else {
            section=section.child(self.hint(self.l("Ordner in NextCloud für geteilte Dokumente. Kein Downloadordner auf diesem Computer.")))
                .child(self.input_surface(&self.upload_folder, cx).child(self.upload_folder.clone()))
                .child(self.dialog_actions().child(self.button("save-upload-folder",self.l("Ordner speichern"),ButtonStyle::Primary,Availability::when(ready&&values.and_then(|v|v.attachment_folder.as_ref()).is_some(),reason),LocalAction::SaveUploadFolder,cx)));
            if values.is_some_and(|v| v.conversation_subfolders == Some(true)) {
                section = section.child(
                    self.hint(self.l("Der Server legt darunter Unterordner für Gespräche an.")),
                );
            }
        }
        let feedback = self
            .account_preference_error
            .map(account_error_key)
            .or_else(|| {
                (!ready && !matches!(state, AccountPreferenceState::Saving { .. }))
                    .then_some(reason)
            });
        if let Some(feedback) = feedback {
            section = section.child(self.hint(self.l(feedback)));
        }
        page.child(section.child(self.dialog_actions().child(self.button(
            "reload-account-settings",
            self.l("Kontoeinstellungen neu laden"),
            ButtonStyle::Secondary,
            Availability::when(
                !matches!(
                    state,
                    AccountPreferenceState::Unavailable
                        | AccountPreferenceState::Loading { .. }
                        | AccountPreferenceState::Saving { .. }
                ),
                reason,
            ),
            LocalAction::ReloadAccountPreferences,
            cx,
        ))))
    }
}
fn account_error_key(error: AccountPreferenceError) -> &'static str {
    match error {
        AccountPreferenceError::Unavailable => {
            "Für dieses Konto besteht keine aktive Serververbindung."
        }
        AccountPreferenceError::Stale => {
            "Das ausgewählte Konto hat sich geändert. Bitte erneut laden."
        }
        AccountPreferenceError::Busy => "Bitte warte auf den laufenden Vorgang.",
        AccountPreferenceError::PermissionDenied => "Der Server erlaubt diese Änderung nicht.",
        AccountPreferenceError::AuthenticationRequired => {
            "Bitte melde dich bei diesem Konto erneut an."
        }
        AccountPreferenceError::Network => "Der Server ist nicht erreichbar. Bitte erneut laden.",
        AccountPreferenceError::Uncertain => {
            "Das Ergebnis ist unklar. Bitte lade die Kontoeinstellungen neu, bevor du erneut änderst."
        }
        AccountPreferenceError::InvalidFolder => {
            "Bitte gib einen gültigen Ordner innerhalb eurer NextCloud an."
        }
    }
}
