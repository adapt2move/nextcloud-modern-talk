//! Settings pages consume the real computer/account/device projections.
mod notifications;

use super::controls::Availability;
use super::preferences::{PreferenceEdit, SettingsTab, preference_error_key};
use super::{ButtonStyle, Desktop, LocalAction};
use gpui::{Context, Div, FontWeight, div, prelude::*, px};
use talk_core::preferences::notifications::{
    DeliveryOutcome, NotificationCategory, NotificationError, NotificationState, SourceAvailability,
};
use talk_core::preferences::shortcuts::{ActionId, Modifiers, SHORTCUTS};
use talk_core::preferences::{EffectState, LoadState, LocalePreference, SaveState};

impl Desktop {
    pub(super) fn setting_toggle(
        &self,
        id: &'static str,
        label: &'static str,
        description: &'static str,
        on: bool,
        edit: PreferenceEdit,
        cx: &Context<Self>,
    ) -> Div {
        self.dialog_row(
            label,
            Some(description),
            self.switch(
                id,
                label,
                Some(on),
                self.settings_availability(),
                LocalAction::Preference(edit),
                cx,
            ),
        )
    }
    fn settings_heading(&self, key: &str) -> Div {
        div()
            .text_size(self.sp(13.))
            .font_weight(FontWeight::MEDIUM)
            .child(self.l(key).to_owned())
    }
    pub(super) fn settings_page(&self, tab: SettingsTab, cx: &Context<Self>) -> Div {
        let p = &self.preferences.active;
        let mut page = div()
            .flex()
            .flex_col()
            .gap_3()
            .w_full()
            .child(self.dialog_page_header(tab.key(), None));
        match tab {
            SettingsTab::General => {
                page=page.child(self.dialog_section(Some("Sprache"))
                        .child(self.hint(self.l("Sprache für diese App auf diesem Computer. Inhalte werden nicht übersetzt.")))
                        .child(div().flex().child(self.segmented().children([
                            (LocalePreference::System,"Systemsprache"),(LocalePreference::English,"English"),
                            (LocalePreference::German,"Deutsch"),(LocalePreference::Spanish,"Español"),
                        ].into_iter().enumerate().map(|(i,(value,label))| self.segment(("locale",i),label,
                            p.locale==value,
                            self.settings_availability(),
                            LocalAction::Preference(PreferenceEdit::Locale(value)),cx))))))
                    .child(self.setting_toggle("launch-login","Beim Anmelden starten","Talk zusammen mit deinem Betriebssystem starten.",p.launch_at_login,PreferenceEdit::LaunchAtLogin(!p.launch_at_login),cx))
                    .child(self.setting_toggle("start-background","Im Hintergrund starten","Gilt nur beim automatischen Start. Ein normaler Start öffnet das Fenster.",p.start_in_background,PreferenceEdit::StartInBackground(!p.start_in_background),cx));
                match self.preferences.startup.state {
                    EffectState::Failed(_) => {
                        page = page
                            .child(self.hint(
                                self.l("Der automatische Start konnte nicht eingerichtet werden."),
                            ))
                            .child(self.dialog_actions().child(self.button(
                                "retry-startup",
                                self.l("Erneut versuchen"),
                                ButtonStyle::Secondary,
                                true,
                                LocalAction::RetryPreferences(true),
                                cx,
                            )))
                    }
                    EffectState::Pending | EffectState::Applied | EffectState::Unknown => {}
                }
            }
            SettingsTab::Notifications => {
                page = page.child(self.notification_settings(cx));
            }
            SettingsTab::Shortcuts => {
                page=page.child(self.hint(self.l("Diese Tasten führen die angegebenen Aktionen aus. Eigene Belegungen sind derzeit nicht verfügbar.")));
                for shortcut in SHORTCUTS {
                    let label = match shortcut.action {
                        ActionId::OpenSettings => "Einstellungen öffnen",
                        ActionId::Search => "Suchen",
                        ActionId::ConversationSearch => "Im Gespräch suchen",
                        ActionId::Close => "Schließen",
                        ActionId::SendMessage => "Nachricht senden",
                        ActionId::NewLine => "Neue Zeile",
                        ActionId::NextControl => "Nächstes Element",
                        ActionId::PreviousControl => "Vorheriges Element",
                        ActionId::Undo => "Rückgängig",
                        ActionId::Redo => "Wiederholen",
                        ActionId::FormatBold => "Fett",
                        ActionId::FormatItalic => "Kursiv",
                        ActionId::FormatCode => "Inline-Code",
                    };
                    let primary = if cfg!(target_os = "macos") {
                        "⌘"
                    } else {
                        "Ctrl"
                    };
                    let prefix = match shortcut.modifiers {
                        Modifiers::None => String::new(),
                        Modifiers::Shift => "Shift + ".to_owned(),
                        Modifiers::Primary => format!("{primary} + "),
                        Modifiers::PrimaryShift => format!("{primary} + Shift + "),
                    };
                    page = page.child(
                        div()
                            .flex()
                            .justify_between()
                            .gap_3()
                            .py_2()
                            .border_b_1()
                            .border_color(self.theme.border)
                            .child(self.l(label))
                            .child(
                                div()
                                    .rounded(px(5.))
                                    .bg(self.theme.inset())
                                    .px_2()
                                    .child(format!("{prefix}{}", shortcut.key)),
                            ),
                    );
                }
            }
            SettingsTab::Updates => {
                use talk_core::preferences::{UpdateChannel, updates::UpdateState};
                let installing = matches!(
                    self.updates.state,
                    UpdateState::Reserved(_) | UpdateState::Applying(_) | UpdateState::Installed(_)
                );
                let checking = installing
                    || matches!(
                        self.updates.state,
                        UpdateState::Checking
                            | UpdateState::Downloading { .. }
                            | UpdateState::Verifying(_)
                    );
                page=page.child(self.hint(&self.lf("Installierte Version: {version}",&[("version",self.updates.current_version.as_str())])))
                    .child(self.hint(self.l("Updates werden automatisch für den gewählten Kanal geprüft und heruntergeladen. Du entscheidest, wann installiert und neu gestartet wird.")))
                    .child(self.dialog_actions().child(self.segmented().children([(UpdateChannel::Stable,"Stable"),(UpdateChannel::Prerelease,"Vorabversionen")].into_iter().enumerate().map(|(i,(channel,label))|
                        self.segment(("update-channel",i),label,p.update_channel==channel,
                            if installing { Availability::pending() } else { self.settings_availability() },
                            LocalAction::Preference(PreferenceEdit::UpdateChannel(channel)),cx))))
                        .child(self.button("check-updates","Updates suchen",ButtonStyle::Secondary,if checking { Availability::pending() } else { Availability::available() },LocalAction::CheckUpdates,cx)));
                // Progress and other data updates keep this variant key; only
                // the state hint reveals, never the download counter below.
                if let Some((status, state_key)) = update_status(&self.updates.state) {
                    page = page.child(
                        self.reveal(("update-status", state_key), self.hint(self.l(status))),
                    );
                }
                let release = match &self.updates.state {
                    UpdateState::Downloading { release, .. }
                    | UpdateState::Verifying(release)
                    | UpdateState::Ready(release)
                    | UpdateState::Reserved(release)
                    | UpdateState::Applying(release)
                    | UpdateState::Installed(release) => Some(release),
                    _ => None,
                };
                if let Some(release) = release {
                    page = page
                        .child(self.hint(&self.lf(
                            "Verfügbar: {version}",
                            &[("version", release.version.as_str())],
                        )))
                        .when(release.prerelease, |d| {
                            d.child(self.hint(self.l("Dies ist eine Vorabversion.")))
                        });
                }
                if let UpdateState::Downloading {
                    received, total, ..
                } = &self.updates.state
                {
                    page = page.child(self.hint(&self.lf(
                        "{received} von {total} heruntergeladen.",
                        &[
                            (
                                "received",
                                &super::localization::file_size(self.locale, Some(*received)),
                            ),
                            (
                                "total",
                                &super::localization::file_size(self.locale, Some(*total)),
                            ),
                        ],
                    )));
                }
                if matches!(self.updates.state, UpdateState::Ready(_)) {
                    page = page.child(self.dialog_actions().child(self.button(
                        "settings-install-update",
                        "Update installieren und neu starten",
                        ButtonStyle::Primary,
                        Availability::when(
                            self.can_install_update(),
                            "Beende zuerst den Anruf, um das Update zu installieren.",
                        ),
                        LocalAction::InstallUpdate(self.updates.generation),
                        cx,
                    )));
                }
            }
            // Replaced by the typed provider pages as their scoped handles arrive.
            SettingsTab::Privacy | SettingsTab::Files => {
                return self.account_settings_page(tab, cx);
            }
            SettingsTab::Devices => return self.device_page(cx),
            SettingsTab::Appearance | SettingsTab::Accounts => {}
        }
        page
    }
    pub(super) fn preference_status(&self, cx: &Context<Self>) -> Div {
        let Some(message) = preference_status_message(
            self.preference_error,
            self.preferences.save,
            self.preferences.load,
        ) else {
            return div();
        };
        let mut content = div()
            .flex_none()
            .flex()
            .flex_col()
            .gap_2()
            .pt_3()
            .border_t_1()
            .border_color(self.theme.border);
        content = content.child(self.hint(self.l(message)));
        if matches!(
            self.preferences.save,
            SaveState::Failed {
                retry_available: true,
                ..
            }
        ) {
            content = content.child(self.dialog_actions().child(self.button(
                "retry-preferences",
                self.l("Erneut versuchen"),
                ButtonStyle::Secondary,
                true,
                LocalAction::RetryPreferences(false),
                cx,
            )));
        }
        content
    }
}

fn update_status(
    state: &talk_core::preferences::updates::UpdateState,
) -> Option<(&'static str, usize)> {
    use talk_core::preferences::updates::UpdateState;
    Some(match state {
        UpdateState::Idle => ("Noch nicht geprüft.", 0),
        UpdateState::Current => ("Keine neuere Version in diesem Kanal verfügbar.", 2),
        UpdateState::Ready(_) => ("Das Update ist zur Installation bereit.", 5),
        UpdateState::Installed(_) => ("Das Update ist installiert.", 8),
        UpdateState::Stopped => ("Die Updateprüfung wurde beendet.", 9),
        UpdateState::Failed(error) => (super::preferences::update_error_label(*error)?, 10),
        UpdateState::Checking
        | UpdateState::Downloading { .. }
        | UpdateState::Verifying(_)
        | UpdateState::Reserved(_)
        | UpdateState::Applying(_) => return None,
    })
}

fn preference_status_message(
    error: Option<talk_core::preferences::PreferenceError>,
    save: SaveState,
    load: LoadState,
) -> Option<&'static str> {
    error
        .and_then(preference_error_key)
        .or_else(|| match save {
            SaveState::Failed { error, .. } => preference_error_key(error),
            SaveState::Idle | SaveState::Pending { .. } => None,
        })
        .or_else(|| {
            matches!(load, LoadState::Failed(_)).then_some(
                "Die gespeicherten Einstellungen konnten nicht gelesen werden. Die Standardwerte sind aktiv.",
            )
        })
}

// Synchronous failures have a category even when no request was admitted.
// An accepted request identity remains reserved for actual worker progress.
pub(super) fn record_notification_test(
    accepted: &mut Option<(talk_core::preferences::RequestId, NotificationCategory)>,
    failure: &mut Option<(NotificationCategory, NotificationError)>,
    attempt: (talk_core::preferences::RequestId, NotificationCategory),
    result: Result<(), NotificationError>,
) {
    match result {
        Ok(()) => {
            *accepted = Some(attempt);
            *failure = None;
        }
        Err(NotificationError::Busy) => {}
        Err(error) => *failure = Some((attempt.1, error)),
    }
}
fn notification_feedback(
    category: NotificationCategory,
    failure: Option<(NotificationCategory, NotificationError)>,
    accepted: Option<(talk_core::preferences::RequestId, NotificationCategory)>,
    state: NotificationState,
) -> Option<&'static str> {
    if failure.is_some_and(|(scope, error)| scope == category && error != NotificationError::Busy) {
        return Some("Benachrichtigung konnte nicht ausgegeben werden.");
    }
    let (request, _) = accepted.filter(|(_, scope)| *scope == category)?;
    match state {
        NotificationState::Finished {
            request: current,
            banner,
            sound,
        } if current == request => Some(match (banner, sound) {
            (DeliveryOutcome::Failed(NotificationError::PermissionDenied), _)
            | (_, DeliveryOutcome::Failed(NotificationError::PermissionDenied)) => {
                "Das Betriebssystem hat die Benachrichtigung abgelehnt."
            }
            (DeliveryOutcome::Failed(NotificationError::Unavailable), _)
            | (_, DeliveryOutcome::Failed(NotificationError::Unavailable)) => {
                "Benachrichtigungen sind auf diesem System nicht verfügbar."
            }
            (DeliveryOutcome::Failed(NotificationError::SourceUnavailable), _)
            | (_, DeliveryOutcome::Failed(NotificationError::SourceUnavailable)) => {
                "Keine bestätigte Kalenderquelle verfügbar."
            }
            (DeliveryOutcome::Cancelled, _) | (_, DeliveryOutcome::Cancelled) => {
                "Testbenachrichtigung abgebrochen."
            }
            (DeliveryOutcome::Failed(_), _) | (_, DeliveryOutcome::Failed(_)) => {
                "Benachrichtigung konnte nicht ausgegeben werden."
            }
            (DeliveryOutcome::Suppressed, DeliveryOutcome::Suppressed) => {
                "Durch die Benachrichtigungseinstellungen unterdrückt."
            }
            _ => {
                "An das Betriebssystem übergeben. Anzeige und Ton hängen von den Systemeinstellungen ab."
            }
        }),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::presentation::preferences::update_error_label;
    use talk_core::preferences::{
        PreferenceError, RequestId, Revision,
        updates::{UpdateError, UpdateState},
    };

    #[test]
    fn notification_rejections_keep_their_category_without_faking_an_accepted_request() {
        let mut accepted = None;
        let mut failure = None;
        let a = NotificationCategory::Messages;
        let b = NotificationCategory::IncomingCalls;
        record_notification_test(
            &mut accepted,
            &mut failure,
            (RequestId(1), b),
            Err(NotificationError::Unavailable),
        );
        assert!(accepted.is_none());
        assert!(notification_feedback(b, failure, accepted, NotificationState::Idle).is_some());
        assert!(notification_feedback(a, failure, accepted, NotificationState::Idle).is_none());
        record_notification_test(&mut accepted, &mut failure, (RequestId(2), a), Ok(()));
        assert_eq!(accepted, Some((RequestId(2), a)));
        record_notification_test(
            &mut accepted,
            &mut failure,
            (RequestId(3), b),
            Err(NotificationError::Unavailable),
        );
        assert_eq!(accepted, Some((RequestId(2), a)));
        assert!(
            notification_feedback(
                b,
                failure,
                accepted,
                NotificationState::Pending(RequestId(2))
            )
            .is_some()
        );
        assert!(
            notification_feedback(
                a,
                failure,
                accepted,
                NotificationState::Pending(RequestId(2))
            )
            .is_none()
        );
        let retained = (accepted, failure);
        record_notification_test(
            &mut accepted,
            &mut failure,
            (RequestId(4), b),
            Err(NotificationError::Busy),
        );
        assert_eq!((accepted, failure), retained);
        assert!(notification_feedback(b, failure, accepted, NotificationState::Idle).is_some());
    }

    #[test]
    fn pending_controls_and_rejected_saves_do_not_hide_terminal_failures() {
        assert_eq!(update_status(&UpdateState::Checking), None);
        assert_eq!(
            update_status(&UpdateState::Failed(UpdateError::Busy)),
            update_error_label(UpdateError::Busy).map(|label| (label, 10)),
        );
        assert!(update_status(&UpdateState::Failed(UpdateError::Busy)).is_some());
        assert!(update_status(&UpdateState::Failed(UpdateError::InstallUncertain)).is_some());
        let pending = SaveState::Pending {
            request: RequestId(4),
            revision: Revision(2),
        };
        assert_eq!(
            preference_status_message(None, pending, LoadState::Loaded),
            None
        );
        assert_eq!(
            preference_status_message(
                Some(PreferenceError::Busy),
                SaveState::Idle,
                LoadState::Loading
            ),
            None,
        );
        let failed = SaveState::Failed {
            request: RequestId(4),
            revision: Revision(2),
            error: PreferenceError::PermissionDenied,
            retry_available: true,
        };
        assert_eq!(
            preference_status_message(Some(PreferenceError::Busy), failed, LoadState::Loaded),
            preference_error_key(PreferenceError::PermissionDenied),
        );
        assert_eq!(
            preference_status_message(
                Some(PreferenceError::PermissionDenied),
                pending,
                LoadState::Loaded
            ),
            preference_error_key(PreferenceError::PermissionDenied),
        );
        assert!(
            preference_status_message(
                Some(PreferenceError::Busy),
                SaveState::Failed {
                    request: RequestId(4),
                    revision: Revision(2),
                    error: PreferenceError::Busy,
                    retry_available: true,
                },
                LoadState::Failed(PreferenceError::StorageUnavailable),
            )
            .is_some()
        );
    }
}
