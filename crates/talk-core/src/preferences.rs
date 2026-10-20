//! Harmless computer preferences and their bounded application boundary.
//! Account privacy and server file destinations have separate scoped DTOs in `account`.
//! Saved System choices never contain an observation of the current OS theme/locale.

pub mod account;
pub mod devices;
pub mod navigation;
pub mod notifications;
pub mod permissions;
pub mod shortcuts;

use devices::DevicePreferences;
use navigation::NavigationPreferences;

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum QuickSwitcherPreference {
    #[default]
    Auto,
    On,
    Off,
}
impl QuickSwitcherPreference {
    /// Count admitted signed-in accounts, not dormant profiles or guessed scopes.
    pub fn effective(self, admitted_accounts: usize) -> bool {
        match self {
            Self::Auto => admitted_accounts > 1,
            Self::On => true,
            Self::Off => false,
        }
    }
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum Palette {
    #[default]
    Blue,
    Sage,
}
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum Appearance {
    #[default]
    System,
    Light,
    Dark,
}
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum DisplayScale {
    #[default]
    Percent100,
    Percent125,
    Percent150,
    Percent200,
}
impl DisplayScale {
    pub const fn percent(self) -> u16 {
        match self {
            Self::Percent100 => 100,
            Self::Percent125 => 125,
            Self::Percent150 => 150,
            Self::Percent200 => 200,
        }
    }
}
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum SidebarDensity {
    Compact,
    #[default]
    Relaxed,
}
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum LocalePreference {
    #[default]
    System,
    English,
    German,
    Spanish,
}
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum Locale {
    #[default]
    English,
    German,
    Spanish,
}
impl LocalePreference {
    /// OS observations are transient. Match preferred language order, never user content.
    /// At most 32 language tags of at most 128 bytes are examined.
    pub fn resolve(self, preferred: &[&str]) -> Locale {
        match self {
            Self::English => Locale::English,
            Self::German => Locale::German,
            Self::Spanish => Locale::Spanish,
            Self::System => preferred
                .iter()
                .take(32)
                .filter(|tag| tag.len() <= 128)
                .find_map(|tag| {
                    match tag
                        .split(['-', '_', '.', '@'])
                        .next()?
                        .to_ascii_lowercase()
                        .as_str()
                    {
                        "en" => Some(Locale::English),
                        "de" => Some(Locale::German),
                        "es" => Some(Locale::Spanish),
                        _ => None,
                    }
                })
                .unwrap_or_default(),
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct NotificationDelivery {
    pub banners: bool,
    pub sounds: bool,
}
impl NotificationDelivery {
    pub const OFF: Self = Self {
        banners: false,
        sounds: false,
    };
}
impl Default for NotificationDelivery {
    fn default() -> Self {
        Self {
            banners: true,
            sounds: true,
        }
    }
}
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct NotificationPreferences {
    pub messages: NotificationDelivery,
    pub important: NotificationDelivery,
    pub incoming_calls: NotificationDelivery,
    pub calendar: NotificationDelivery,
}
impl Default for NotificationPreferences {
    fn default() -> Self {
        Self {
            messages: NotificationDelivery::default(),
            important: NotificationDelivery::default(),
            incoming_calls: NotificationDelivery::default(),
            calendar: NotificationDelivery::OFF,
        }
    }
}
impl NotificationPreferences {
    pub fn delivery(self, category: notifications::NotificationCategory) -> NotificationDelivery {
        use notifications::NotificationCategory;
        match category {
            NotificationCategory::Messages => self.messages,
            NotificationCategory::Important => self.important,
            NotificationCategory::IncomingCalls => self.incoming_calls,
            NotificationCategory::Calendar => self.calendar,
        }
    }
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct LocalPreferences {
    pub update_channel: UpdateChannel,
    pub quick_switcher: QuickSwitcherPreference,
    pub navigation: NavigationPreferences,
    pub palette: Palette,
    pub appearance: Appearance,
    pub scale: DisplayScale,
    pub sidebar_density: SidebarDensity,
    pub locale: LocalePreference,
    pub launch_at_login: bool,
    pub start_in_background: bool,
    pub notifications: NotificationPreferences,
    pub devices: DevicePreferences,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Ord, PartialOrd)]
pub struct RequestId(pub u64);
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, Ord, PartialOrd)]
pub struct Revision(pub u64);

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum PreferenceError {
    Stale,
    Closed,
    Busy,
    RevisionExhausted,
    InvalidData,
    UnsupportedVersion,
    TooLarge,
    PermissionDenied,
    StorageUnavailable,
    StartupUnavailable,
    ApplyFailed,
    WorkerUnavailable,
}
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum LoadState {
    Loading,
    Loaded,
    DefaultsBecauseAbsent,
    Migrated,
    Failed(PreferenceError),
}
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SaveState {
    Idle,
    Pending {
        request: RequestId,
        revision: Revision,
    },
    Failed {
        request: RequestId,
        revision: Revision,
        error: PreferenceError,
        retry_available: bool,
    },
}
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum EffectState {
    Unknown,
    Pending,
    Applied,
    Failed(PreferenceError),
}
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct StartupSnapshot {
    /// Observed app-owned OS registration, not an assertion based on the preference.
    pub registered: Option<bool>,
    pub state: EffectState,
}
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum RuntimeState {
    Running,
    Draining,
    Stopped,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PreferenceSnapshot {
    pub generation: u64,
    pub revision: Revision,
    pub durable_revision: Revision,
    /// Render and apply these. On a current disk failure they revert to confirmed.
    pub active: LocalPreferences,
    pub confirmed: LocalPreferences,
    /// Transient observation, never encoded as the saved System choice.
    pub observed_system_locale: Locale,
    pub load: LoadState,
    pub save: SaveState,
    pub startup: StartupSnapshot,
    pub runtime: RuntimeState,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum PreferenceCommand {
    Set {
        generation: u64,
        request: RequestId,
        expected_revision: Revision,
        values: LocalPreferences,
    },
    Retry {
        generation: u64,
        request: RequestId,
        failed_revision: Revision,
    },
    /// Retry applying the already durable preferences; this does not claim a new save.
    RetryEffects {
        generation: u64,
        request: RequestId,
        expected_revision: Revision,
    },
}

/// No hidden window is requested by an ordinary explicit launch. The OS entry uses
/// `--autostart`; composition must honor this after load, and offer a way to reopen.
pub fn starts_hidden(values: &LocalPreferences, autostart: bool) -> bool {
    autostart && values.start_in_background
}

pub mod updates;

/// Selects published GitHub releases. Checking and opening downloads are explicit.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum UpdateChannel {
    #[default]
    Stable,
    Prerelease,
}
