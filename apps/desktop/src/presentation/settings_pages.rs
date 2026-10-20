//! Settings pages consume the real computer/account/device projections.
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
        div()
            .flex_none()
            .flex()
            .items_center()
            .justify_between()
            .gap_4()
            .py_3()
            .border_b_1()
            .border_color(self.theme.border)
            .child(
                div()
                    .min_w_0()
                    .flex_1()
                    .child(self.settings_heading(label))
                    .child(self.hint(self.l(description))),
            )
            .child(self.switch(
                id,
                label,
                Some(on),
                self.settings_availability(),
                LocalAction::Preference(edit),
                cx,
            ))
    }
    fn settings_heading(&self, key: &str) -> Div {
        div()
            .text_size(self.sp(13.))
            .font_weight(FontWeight::MEDIUM)
            .child(self.l(key).to_owned())
    }
    pub(super) fn settings_page(&self, tab: SettingsTab, cx: &Context<Self>) -> Div {
        let p = &self.preferences.active;
        let mut page = div().flex().flex_col().gap_3().w_full().child(
            div()
                .text_size(self.sp(20.))
                .font_weight(FontWeight::MEDIUM)
                .child(self.l(tab.key())),
        );
        match tab {
            SettingsTab::General => {
                page=page.child(self.settings_heading(self.l("Sprache")))
                    .child(self.hint(self.l("Sprache für diese App auf diesem Computer. Inhalte werden nicht übersetzt.")))
                    .child(div().flex().flex_wrap().gap_2().children([
                        (LocalePreference::System,self.l("Systemsprache")),(LocalePreference::English,"English"),
                        (LocalePreference::German,"Deutsch"),(LocalePreference::Spanish,"Español"),
                    ].into_iter().enumerate().map(|(i,(value,label))| self.button(("locale",i),label,
                        if p.locale==value {ButtonStyle::Primary} else {ButtonStyle::Secondary},
                        self.settings_availability(),
                        LocalAction::Preference(PreferenceEdit::Locale(value)),cx))))
                    .child(self.setting_toggle("launch-login","Beim Anmelden starten","Talk zusammen mit deinem Betriebssystem starten.",p.launch_at_login,PreferenceEdit::LaunchAtLogin(!p.launch_at_login),cx))
                    .child(self.setting_toggle("start-background","Im Hintergrund starten","Gilt nur beim automatischen Start. Ein normaler Start öffnet das Fenster.",p.start_in_background,PreferenceEdit::StartInBackground(!p.start_in_background),cx));
                match self.preferences.startup.state {
                    EffectState::Failed(_) => {
                        page = page
                            .child(self.hint(
                                self.l("Der automatische Start konnte nicht eingerichtet werden."),
                            ))
                            .child(self.button(
                                "retry-startup",
                                self.l("Erneut versuchen"),
                                ButtonStyle::Secondary,
                                true,
                                LocalAction::RetryPreferences(true),
                                cx,
                            ))
                    }
                    EffectState::Pending => {
                        page =
                            page.child(self.hint(self.l("Automatischer Start wird eingerichtet …")))
                    }
                    EffectState::Applied | EffectState::Unknown => {}
                }
            }
            SettingsTab::Notifications => {
                page = page.child(self.hint(self.l("Banner erscheinen bei minimiertem Fenster. Fokus- und Ruhemodi des Betriebssystems gelten weiter.")));
                let column = self.sp(48.).min(px(88.));
                page = page.child(
                    div()
                        .flex_none()
                        .flex()
                        .items_center()
                        .gap_3()
                        .pt_2()
                        .child(div().flex_1())
                        .children(["Banner", "Ton"].into_iter().map(|key| {
                            div()
                                .flex_none()
                                .w(column)
                                .text_center()
                                .text_size(self.sp(12.))
                                .text_color(self.theme.muted)
                                .child(self.l(key))
                        }))
                        .child(div().flex_none().w(px(36.))),
                );
                for (index, (category, title, description, test, delivery)) in [
                    (NotificationCategory::Messages, "Nachrichten", "Hinweise auf neue Nachrichten.", "Nachrichten testen", p.notifications.messages),
                    (NotificationCategory::Important, "Wichtig", "Konferenzbeitritte und persönliche Erinnerungen.", "Wichtige Hinweise testen", p.notifications.important),
                    (NotificationCategory::IncomingCalls, "Eingehende Anrufe", "Eigene Hinweise für eingehende Anrufe.", "Anrufhinweis testen", p.notifications.incoming_calls),
                    (NotificationCategory::Calendar, "Kalender", "Optional für eine bestätigte NextCloud-Kalenderquelle dieses Kontos. Standardmäßig aus.", "Kalenderhinweis testen", p.notifications.calendar),
                ].into_iter().enumerate() {
                    let source_available = category != NotificationCategory::Calendar
                        || self.settings_port.notification_source(self.snapshot.active_account, category) == SourceAvailability::Available;
                    let available = if source_available { self.settings_availability() }
                        else { Availability::when(false, "Keine bestätigte Kalenderquelle verfügbar.") };
                    let mut section = div().flex_none().flex().flex_col().gap_1().py_2()
                        .border_b_1().border_color(self.theme.border)
                        .child(div().flex().items_center().gap_3()
                            .child(div().min_w_0().flex_1()
                                .child(self.settings_heading(title)).child(self.hint(self.l(description))))
                            .child(div().flex_none().w(column).flex().justify_center()
                                .child(self.switch(("notification-banner", index), "Banner", Some(delivery.banners), available,
                                    LocalAction::Preference(PreferenceEdit::NotificationBanner(category, !delivery.banners)), cx)))
                            .child(div().flex_none().w(column).flex().justify_center()
                                .child(self.switch(("notification-sound", index), "Ton", Some(delivery.sounds), available,
                                    LocalAction::Preference(PreferenceEdit::NotificationSound(category, !delivery.sounds)), cx)))
                            .child(self.icon_button(("test-notification", index), ("bell", test), ButtonStyle::Quiet, available,
                                LocalAction::TestNotification(category), cx).w(px(36.)).flex_none()));
                    if category == NotificationCategory::IncomingCalls
                        && (!self.settings_port.notification_capabilities().native_call_ui
                            || !self.settings_port.notification_capabilities().native_call_actions) {
                        section = section.child(self.hint(self.l("Derzeit werden normale Betriebssystem-Banner und Töne verwendet. Native Anrufaktionen sind noch nicht verfügbar.")));
                    }
                    if !source_available { section = section.child(self.hint(self.l("Keine bestätigte Kalenderquelle verfügbar."))); }
                    if let Some((request, _)) = self.notification_test.filter(|(_, c)| *c == category) {
                        let label = if self.notification_error.is_some() { Some("Benachrichtigung konnte nicht ausgegeben werden.") }
                        else { match self.notification_state {
                            NotificationState::Pending(current) if current == request => Some("Benachrichtigung wird angefragt …"),
                            NotificationState::Finished { request: current, banner, sound } if current == request => Some(match (banner, sound) {
                                (DeliveryOutcome::Failed(NotificationError::PermissionDenied), _) | (_, DeliveryOutcome::Failed(NotificationError::PermissionDenied)) => "Das Betriebssystem hat die Benachrichtigung abgelehnt.",
                                (DeliveryOutcome::Failed(NotificationError::Unavailable), _) | (_, DeliveryOutcome::Failed(NotificationError::Unavailable)) => "Benachrichtigungen sind auf diesem System nicht verfügbar.",
                                (DeliveryOutcome::Failed(NotificationError::SourceUnavailable), _) | (_, DeliveryOutcome::Failed(NotificationError::SourceUnavailable)) => "Keine bestätigte Kalenderquelle verfügbar.",
                                (DeliveryOutcome::Cancelled, _) | (_, DeliveryOutcome::Cancelled) => "Testbenachrichtigung abgebrochen.",
                                (DeliveryOutcome::Failed(_), _) | (_, DeliveryOutcome::Failed(_)) => "Benachrichtigung konnte nicht ausgegeben werden.",
                                (DeliveryOutcome::Suppressed, DeliveryOutcome::Suppressed) => "Durch die Benachrichtigungseinstellungen unterdrückt.",
                                _ => "An das Betriebssystem übergeben. Anzeige und Ton hängen von den Systemeinstellungen ab.",
                            }), _ => None,
                        }};
                        if let Some(label) = label { section = section.child(self.hint(self.l(label))); }
                    }
                    page = page.child(section);
                }
                page = page.child(self.hint(self.l(
                    "Tests prüfen nur die lokale Ausgabe, nicht den Empfang von Kontoereignissen.",
                )));
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
                                    .bg(self.theme.subtle)
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
                    .child(div().flex().flex_wrap().gap_2().children([(UpdateChannel::Stable,"Stable"),(UpdateChannel::Prerelease,"Vorabversionen")].into_iter().enumerate().map(|(i,(channel,label))|
                        self.button(("update-channel",i),label,if p.update_channel==channel {ButtonStyle::Primary}else{ButtonStyle::Secondary},
                            if installing { Availability::when(false,"Das Update wird installiert …") } else { self.settings_availability() },
                            LocalAction::Preference(PreferenceEdit::UpdateChannel(channel)),cx))))
                    .child(self.button("check-updates","Updates suchen",ButtonStyle::Secondary,Availability::when(!checking,"Eine Updateaktion läuft bereits."),LocalAction::CheckUpdates,cx));
                let (status, state_key) = match &self.updates.state {
                    UpdateState::Idle => ("Noch nicht geprüft.", 0usize),
                    UpdateState::Checking => ("Updates werden geprüft …", 1),
                    UpdateState::Current => ("Keine neuere Version in diesem Kanal verfügbar.", 2),
                    UpdateState::Downloading { .. } => ("Das Update wird heruntergeladen …", 3),
                    UpdateState::Verifying(_) => ("Das Update wird geprüft …", 4),
                    UpdateState::Ready(_) => ("Das Update ist zur Installation bereit.", 5),
                    UpdateState::Reserved(_) => ("Die App bereitet die Installation vor …", 6),
                    UpdateState::Applying(_) => ("Das Update wird installiert …", 7),
                    UpdateState::Installed(_) => (
                        "Das Update ist installiert. Der Neustart wird vorbereitet …",
                        8,
                    ),
                    UpdateState::Stopped => ("Die Updateprüfung wurde beendet.", 9),
                    UpdateState::Failed(error) => {
                        (super::preferences::update_error_label(*error), 10)
                    }
                };
                // Progress and other data updates keep this variant key; only
                // the state hint reveals, never the download counter below.
                page = page
                    .child(self.reveal(("update-status", state_key), self.hint(self.l(status))));
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
                        "{received} von {total} MiB heruntergeladen.",
                        &[
                            ("received", &format!("{:.1}", *received as f64 / 1_048_576.)),
                            ("total", &format!("{:.1}", *total as f64 / 1_048_576.)),
                        ],
                    )));
                }
                if matches!(self.updates.state, UpdateState::Ready(_)) {
                    page = page.child(self.button(
                        "settings-install-update",
                        "Update installieren und neu starten",
                        ButtonStyle::Primary,
                        Availability::when(
                            self.can_install_update(),
                            "Beende zuerst den Anruf, um das Update zu installieren.",
                        ),
                        LocalAction::InstallUpdate(self.updates.generation),
                        cx,
                    ));
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
        let mut content = div()
            .flex_none()
            .flex()
            .flex_col()
            .gap_2()
            .pt_3()
            .border_t_1()
            .border_color(self.theme.border);
        let message = if let Some(error) = self.preference_error {
            preference_error_key(error)
        } else {
            match self.preferences.save {
                SaveState::Pending { .. } => "Wird gespeichert …",
                SaveState::Failed { error, .. } => preference_error_key(error),
                SaveState::Idle => match self.preferences.load {
                    LoadState::Loading => "Einstellungen werden geladen …",
                    LoadState::Failed(_) => {
                        "Die gespeicherten Einstellungen konnten nicht gelesen werden. Die Standardwerte sind aktiv."
                    }
                    LoadState::DefaultsBecauseAbsent | LoadState::Loaded | LoadState::Migrated => {
                        return div();
                    }
                },
            }
        };
        content = content.child(self.hint(self.l(message)));
        if matches!(
            self.preferences.save,
            SaveState::Failed {
                retry_available: true,
                ..
            }
        ) {
            content = content.child(self.button(
                "retry-preferences",
                self.l("Erneut versuchen"),
                ButtonStyle::Secondary,
                true,
                LocalAction::RetryPreferences(false),
                cx,
            ));
        }
        content
    }
}
