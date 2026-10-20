//! Memory-only computer settings boundary. Composition owns workers and disposal.
use super::{Desktop, LocalAction};
use gpui::Context;
use talk_core::preferences::devices::{DeviceError, DeviceIntent, DeviceSnapshot};
use talk_core::preferences::notifications::{
    NotificationCategory, NotificationError, NotificationState, SourceAvailability,
};
use talk_core::preferences::{
    DisplayScale, LoadState, LocalPreferences, Locale, LocalePreference, PreferenceCommand,
    PreferenceError, PreferenceSnapshot, RequestId, SaveState, SidebarDensity,
};

pub trait ComputerSettingsPort {
    fn update_snapshot(&self) -> talk_core::preferences::updates::UpdateSnapshot;
    fn take_update_snapshot(&mut self) -> Option<talk_core::preferences::updates::UpdateSnapshot>;
    fn check_updates(&self) -> Result<(), talk_core::preferences::updates::UpdateError>;
    fn open_release(&self) -> Result<(), talk_core::preferences::updates::UpdateError>;
    fn snapshot(&self) -> PreferenceSnapshot;
    fn take_snapshot(&mut self) -> Option<PreferenceSnapshot>;
    fn submit(&self, command: PreferenceCommand) -> Result<(), PreferenceError>;
    fn system_locale(&self) -> Locale;
    /// The admitted application bridge must discard responses whose account/runtime
    /// or latest request no longer matches. Presentation never admits an account.
    fn account_snapshot(
        &self,
        scope: talk_core::ui::AccountScope,
    ) -> talk_core::preferences::account::AccountPreferenceSnapshot;
    fn submit_account(
        &self,
        command: talk_core::preferences::account::AccountPreferenceCommand,
    ) -> Result<(), talk_core::preferences::account::AccountPreferenceError>;

    fn permission_snapshot(
        &self,
    ) -> Result<
        talk_core::preferences::permissions::PermissionSnapshot,
        talk_core::preferences::permissions::PermissionError,
    >;
    fn take_permission_snapshot(
        &mut self,
    ) -> Option<
        Result<
            talk_core::preferences::permissions::PermissionSnapshot,
            talk_core::preferences::permissions::PermissionError,
        >,
    >;
    fn submit_permission(
        &self,
        intent: talk_core::preferences::permissions::PermissionIntent,
    ) -> Result<(), talk_core::preferences::permissions::PermissionError>;
    fn device_preferences_applied(&self) -> bool;
    fn device_snapshot(&self) -> Result<DeviceSnapshot, DeviceError>;
    fn take_device_snapshot(&mut self) -> Option<Result<DeviceSnapshot, DeviceError>>;
    fn submit_device(&self, intent: DeviceIntent) -> Result<(), DeviceError>;
    fn notification_state(&mut self) -> NotificationState;
    fn notification_capabilities(
        &self,
    ) -> talk_core::preferences::notifications::NativeNotificationCapabilities;
    fn notification_source(
        &self,
        scope: Option<talk_core::ui::AccountScope>,
        category: NotificationCategory,
    ) -> SourceAvailability;
    fn test_notification(
        &mut self,
        request: RequestId,
        category: NotificationCategory,
        title: &str,
        body: &str,
    ) -> Result<(), NotificationError>;
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SettingsTab {
    General,
    Appearance,
    Devices,
    Notifications,
    Privacy,
    Files,
    Shortcuts,
    Updates,
    Accounts,
}
impl SettingsTab {
    pub const ALL: [Self; 9] = [
        Self::General,
        Self::Appearance,
        Self::Devices,
        Self::Notifications,
        Self::Privacy,
        Self::Files,
        Self::Shortcuts,
        Self::Updates,
        Self::Accounts,
    ];
    pub fn key(self) -> &'static str {
        match self {
            Self::General => "Allgemein",
            Self::Appearance => "Darstellung",
            Self::Devices => "Geräte",
            Self::Notifications => "Benachrichtigungen",
            Self::Privacy => "Datenschutz",
            Self::Files => "Dateien",
            Self::Shortcuts => "Tastenkürzel",
            Self::Updates => "Updates",
            Self::Accounts => "Konten",
        }
    }
    pub fn icon(self) -> &'static str {
        match self {
            Self::General => "settings",
            Self::Appearance => "palette",
            Self::Devices => "mic",
            Self::Notifications => "bell",
            Self::Privacy => "shield",
            Self::Files => "file",
            Self::Shortcuts => "keyboard",
            Self::Updates => "download",
            Self::Accounts => "user",
        }
    }
}
#[derive(Clone, Debug)]
pub enum PreferenceEdit {
    UpdateChannel(talk_core::preferences::UpdateChannel),
    Locale(LocalePreference),
    QuickSwitcher(talk_core::preferences::QuickSwitcherPreference),
    LaunchAtLogin(bool),
    StartInBackground(bool),
    NotificationSound(NotificationCategory, bool),
    NotificationBanner(NotificationCategory, bool),
}
impl Desktop {
    pub(super) fn refresh_preferences(&mut self, cx: &mut Context<Self>) {
        if let Some(snapshot) = self.settings_port.take_update_snapshot() {
            self.updates = snapshot;
            cx.notify();
        }
        if let Some(snapshot) = self.settings_port.take_snapshot() {
            self.apply_preferences(snapshot, cx);
        }
        self.refresh_permissions(cx);
        self.refresh_devices(cx);
        self.refresh_account_preferences(cx);
        let state = self.settings_port.notification_state();
        if self.notification_state != state {
            self.notification_state = state;
            cx.notify();
        }
    }
    fn apply_preferences(&mut self, snapshot: PreferenceSnapshot, cx: &mut Context<Self>) {
        self.palette = snapshot.active.palette;
        self.appearance = snapshot.active.appearance;
        self.scale = snapshot.active.scale.percent();
        self.compact = snapshot.active.sidebar_density == SidebarDensity::Compact;
        self.locale = match snapshot.active.locale {
            LocalePreference::System => self.settings_port.system_locale(),
            preference => preference.resolve(&[]),
        };
        self.preferences = snapshot;
        self.apply_device_preferences();
        // Labels change without replacing editor entities, drafts or scoped handles.
        cx.notify();
    }
    pub(super) fn commit_preferences(&mut self, values: LocalPreferences, cx: &mut Context<Self>) {
        if let Some(request) = self.settings_request.checked_add(1) {
            self.settings_request = request;
            self.preference_error = self
                .settings_port
                .submit(PreferenceCommand::Set {
                    generation: self.preferences.generation,
                    request: RequestId(request),
                    expected_revision: self.preferences.revision,
                    values,
                })
                .err();
            self.apply_preferences(self.settings_port.snapshot(), cx);
        } else {
            self.preference_error = Some(PreferenceError::RevisionExhausted);
        }
    }
    pub(super) fn edit_preference(&mut self, edit: PreferenceEdit, cx: &mut Context<Self>) {
        let mut values = self.preferences.active.clone();
        match edit {
            PreferenceEdit::UpdateChannel(channel) => values.update_channel = channel,
            PreferenceEdit::Locale(v) => values.locale = v,
            PreferenceEdit::QuickSwitcher(v) => values.quick_switcher = v,
            PreferenceEdit::LaunchAtLogin(v) => values.launch_at_login = v,
            PreferenceEdit::StartInBackground(v) => values.start_in_background = v,
            PreferenceEdit::NotificationSound(category, on)
            | PreferenceEdit::NotificationBanner(category, on) => {
                let delivery = match category {
                    NotificationCategory::Messages => &mut values.notifications.messages,
                    NotificationCategory::Important => &mut values.notifications.important,
                    NotificationCategory::IncomingCalls => &mut values.notifications.incoming_calls,
                    NotificationCategory::Calendar => &mut values.notifications.calendar,
                };
                if matches!(edit, PreferenceEdit::NotificationSound(..)) {
                    delivery.sounds = on;
                } else {
                    delivery.banners = on;
                }
            }
        }
        self.commit_preferences(values, cx);
    }
    pub(super) fn retry_preferences(&mut self, effects: bool, cx: &mut Context<Self>) {
        let Some(request) = self.settings_request.checked_add(1) else {
            return;
        };
        self.settings_request = request;
        let command = if effects {
            PreferenceCommand::RetryEffects {
                generation: self.preferences.generation,
                request: RequestId(request),
                expected_revision: self.preferences.revision,
            }
        } else if let SaveState::Failed {
            revision,
            retry_available: true,
            ..
        } = self.preferences.save
        {
            PreferenceCommand::Retry {
                generation: self.preferences.generation,
                request: RequestId(request),
                failed_revision: revision,
            }
        } else {
            return;
        };
        self.preference_error = self.settings_port.submit(command).err();
        self.apply_preferences(self.settings_port.snapshot(), cx);
    }
    pub(super) fn settings_availability(&self) -> super::controls::Availability {
        let running = self.preferences.runtime == talk_core::preferences::RuntimeState::Running;
        super::controls::Availability::when(
            running && self.preferences.load != LoadState::Loading,
            if running {
                "Einstellungen werden geladen …"
            } else {
                "Der Einstellungsdienst ist nicht verfügbar."
            },
        )
    }
    pub(super) fn local_preference_action(
        &mut self,
        action: &LocalAction,
        cx: &mut Context<Self>,
    ) -> bool {
        let mut values = self.preferences.active.clone();
        match action {
            LocalAction::Theme(p) => values.palette = *p,
            LocalAction::Appearance(a) => values.appearance = *a,
            LocalAction::Density(compact) => {
                values.sidebar_density = if *compact {
                    SidebarDensity::Compact
                } else {
                    SidebarDensity::Relaxed
                }
            }
            LocalAction::Scale(scale) => {
                values.scale = match scale {
                    125 => DisplayScale::Percent125,
                    150 => DisplayScale::Percent150,
                    200 => DisplayScale::Percent200,
                    _ => DisplayScale::Percent100,
                }
            }
            _ => return false,
        }
        self.commit_preferences(values, cx);
        true
    }
}
pub fn preference_error_key(error: PreferenceError) -> &'static str {
    match error {
        PreferenceError::PermissionDenied => {
            "Die Einstellungen konnten nicht gespeichert werden. Bitte prüfe die Zugriffsrechte."
        }
        PreferenceError::InvalidData
        | PreferenceError::UnsupportedVersion
        | PreferenceError::TooLarge => {
            "Die Einstellungen konnten nicht gespeichert werden. Die bestätigten Werte sind wieder aktiv."
        }
        PreferenceError::StartupUnavailable | PreferenceError::ApplyFailed => {
            "Der automatische Start konnte nicht eingerichtet werden."
        }
        PreferenceError::Busy => "Bitte warte auf den laufenden Vorgang.",
        PreferenceError::Stale => "Die Einstellungen haben sich geändert. Bitte erneut versuchen.",
        PreferenceError::Closed | PreferenceError::WorkerUnavailable => {
            "Der Einstellungsdienst ist nicht verfügbar."
        }
        PreferenceError::RevisionExhausted => {
            "Bitte starte die App neu, bevor du weitere Einstellungen änderst."
        }
        PreferenceError::StorageUnavailable => {
            "Die Einstellungen konnten nicht gespeichert werden. Die bestätigten Werte sind wieder aktiv."
        }
    }
}
