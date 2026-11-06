//! Local delivery preferences and a separately observed OS authorization state.
use super::*;
use talk_core::preferences::notifications::{
    NotificationAccessAction, NotificationAccessOperation, NotificationAuthorization,
};

impl Desktop {
    pub(super) fn notification_settings(&self, cx: &Context<Self>) -> Div {
        let p = &self.preferences.active;
        let mut page = div().flex().flex_col().gap_3().min_w_0().w_full();
        if cfg!(target_os = "macos") {
            page = page.child(self.notification_access_panel(cx));
        }

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
    let testing = self.notification_test.is_some_and(|(request, current)| {
        current == category
            && matches!(self.notification_state, NotificationState::Pending(pending) if pending == request)
    });
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
            .child(self.icon_button(("test-notification", index), ("bell", test), ButtonStyle::Quiet, if !self.notification_tests_authorized() { Availability::when(false, "Erlaube zuerst Benachrichtigungen in macOS.") } else if testing { Availability::pending() } else { available },
                LocalAction::TestNotification(category), cx).w(px(36.)).flex_none()));
    if category == NotificationCategory::IncomingCalls
        && !self.settings_port.notification_capabilities().native_call_actions {
        section = section.child(self.hint(self.l("Derzeit werden normale Betriebssystem-Banner und Töne verwendet. Native Anrufaktionen sind noch nicht verfügbar.")));
    }
    if !source_available { section = section.child(self.hint(self.l("Keine bestätigte Kalenderquelle verfügbar."))); }
    if let Some(label) = notification_feedback(category, self.notification_error, self.notification_test, self.notification_state) {
        section = section.child(self.hint(self.l(label)));
    }
page = page.child(section);
}
        page = page.child(self.hint(
            self.l("Tests prüfen nur die lokale Ausgabe, nicht den Empfang von Kontoereignissen."),
        ));

        page
    }
    fn notification_access_panel(&self, cx: &Context<Self>) -> Div {
        let access = self.notification_access;
        let pending = matches!(
            access.operation,
            NotificationAccessOperation::Pending { .. }
        );
        let status = match access.operation {
            NotificationAccessOperation::Pending {
                action: NotificationAccessAction::Request,
                ..
            } => "Bestätige die Anfrage von macOS, wenn sie erscheint.",
            NotificationAccessOperation::Pending { .. } => {
                "Benachrichtigungsfreigabe wird geprüft."
            }
            _ => match access.authorization {
                NotificationAuthorization::Unknown => "Zugriffsstatus unbekannt.",
                NotificationAuthorization::NotDetermined => "Zugriff noch nicht freigegeben.",
                NotificationAuthorization::Granted | NotificationAuthorization::NotRequired => {
                    "Zugriff erlaubt."
                }
                NotificationAuthorization::Denied => "Zugriff verweigert.",
                NotificationAuthorization::Unavailable => {
                    "Die native Zugriffsprüfung ist auf diesem System noch nicht eingerichtet."
                }
            },
        };
        let availability = if pending {
            Availability::pending()
        } else {
            Availability::available()
        };
        let mut panel = self
            .dialog_section(Some("macOS-Freigabe"))
            .child(self.hint(self.l(status)));
        let (label, action) = if access.authorization == NotificationAuthorization::NotDetermined {
            (
                "Benachrichtigungen erlauben",
                NotificationAccessAction::Request,
            )
        } else {
            (
                "Systemeinstellungen öffnen",
                NotificationAccessAction::OpenSystemSettings,
            )
        };
        let mut actions = self.dialog_actions();
        if access.authorization != NotificationAuthorization::Unavailable {
            // One stable control identity retains keyboard focus after a grant.
            actions = actions.child(self.button(
                "notification-access-action",
                label,
                if action == NotificationAccessAction::Request {
                    ButtonStyle::Primary
                } else {
                    ButtonStyle::Secondary
                },
                availability,
                LocalAction::NotificationAccess(action),
                cx,
            ));
        }
        panel = panel.child(actions.child(self.button(
            "notification-access-check",
            "Erneut prüfen",
            ButtonStyle::Quiet,
            availability,
            LocalAction::NotificationAccess(NotificationAccessAction::Check),
            cx,
        )));
        if access.authorization == NotificationAuthorization::Denied {
            panel = panel.child(self.hint(self.l("Öffne „Mitteilungen“ in den Systemeinstellungen und erlaube NextCloud Modern Talk. Wähle danach „Erneut prüfen“.")));
        }
        let error = self.notification_access_error.or(match access.operation {
            NotificationAccessOperation::Failed { error, .. } => Some(error),
            _ => None,
        });
        if let Some(error) = error {
            panel = panel.child(self.hint(self.l(match error {
                NotificationError::Busy => {
                    "Der Benachrichtigungsdienst ist gerade beschäftigt. Versuche es erneut."
                }
                NotificationError::TimedOut => {
                    "Die Zugriffsanfrage hat zu lange gedauert. Bitte erneut prüfen."
                }
                NotificationError::Cancelled | NotificationError::Closed => {
                    "Die Zugriffsanfrage wurde abgebrochen."
                }
                NotificationError::PermissionDenied | NotificationError::Stale => {
                    "Prüfe den Zugriff erneut, um den aktuellen Status zu erfahren."
                }
                _ => "Die Zugriffsaktion ist fehlgeschlagen. Bitte erneut prüfen.",
            })));
        }
        panel
    }
}
