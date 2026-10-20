//! Small versioned preference file. No credentials, account records or arbitrary JSON.
//! All reads are bounded before allocation. Atomicwrites syncs the temporary file
//! and replaces the old file in its directory, including Windows replacement.
use atomicwrites::{AllowOverwrite, AtomicFile};
use std::{
    fs,
    io::{self, Read, Write},
    path::{Path, PathBuf},
};
use talk_core::preferences::{
    Appearance, DisplayScale, LoadState, LocalPreferences, LocalePreference, NotificationDelivery,
    OnboardingResume, Palette, PreferenceError, QuickSwitcherPreference, SidebarDensity,
    UpdateChannel,
    devices::{DeviceId, DeviceSelection},
};

mod navigation;

pub(super) const MAX_FILE_BYTES: usize = 512 * 1024;
pub(super) trait PreferenceStore: Send {
    fn load(&mut self) -> Result<(LocalPreferences, LoadState), PreferenceError>;
    fn save(&mut self, values: &LocalPreferences) -> Result<(), PreferenceError>;
}
pub(super) struct FileStore(pub PathBuf);
impl PreferenceStore for FileStore {
    fn load(&mut self) -> Result<(LocalPreferences, LoadState), PreferenceError> {
        let data = match read_bounded(&self.0, MAX_FILE_BYTES) {
            Ok(data) => data,
            Err(error) if error.kind() == io::ErrorKind::NotFound => {
                return Ok((
                    LocalPreferences::default(),
                    LoadState::DefaultsBecauseAbsent,
                ));
            }
            Err(error) => return Err(io_error(error)),
        };
        decode(&data)
    }
    fn save(&mut self, values: &LocalPreferences) -> Result<(), PreferenceError> {
        let encoded = encode(values);
        if encoded.len() > MAX_FILE_BYTES {
            return Err(PreferenceError::TooLarge);
        }
        atomic_write(&self.0, encoded.as_bytes()).map_err(io_error)
    }
}
pub(super) fn io_error(error: io::Error) -> PreferenceError {
    match error.kind() {
        io::ErrorKind::PermissionDenied => PreferenceError::PermissionDenied,
        io::ErrorKind::InvalidData => PreferenceError::InvalidData,
        _ => PreferenceError::StorageUnavailable,
    }
}
pub(super) fn read_bounded(path: &Path, limit: usize) -> io::Result<Vec<u8>> {
    let metadata = fs::symlink_metadata(path)?;
    if !metadata.is_file() || metadata.len() > limit as u64 {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "not a bounded regular file",
        ));
    }
    let file = fs::File::open(path)?;
    let mut bytes = Vec::with_capacity(limit.min(1024));
    file.take(limit as u64 + 1).read_to_end(&mut bytes)?;
    if bytes.len() > limit {
        return Err(io::ErrorKind::InvalidData.into());
    }
    Ok(bytes)
}
pub(super) fn atomic_write(path: &Path, bytes: &[u8]) -> io::Result<()> {
    let parent = path.parent().ok_or(io::ErrorKind::InvalidInput)?;
    fs::create_dir_all(parent)?;
    // Never replace a directory or follow an existing preference/startup symlink.
    match fs::symlink_metadata(path) {
        Ok(metadata) if !metadata.is_file() => return Err(io::ErrorKind::InvalidData.into()),
        Ok(_) => {}
        Err(error) if error.kind() == io::ErrorKind::NotFound => {}
        Err(error) => return Err(error),
    }
    AtomicFile::new(path, AllowOverwrite)
        .write(|file| {
            #[cfg(unix)]
            {
                use std::os::unix::fs::PermissionsExt;
                file.set_permissions(fs::Permissions::from_mode(0o600))?;
            }
            file.write_all(bytes)
        })
        .map_err(|error| match error {
            atomicwrites::Error::Internal(error) | atomicwrites::Error::User(error) => error,
        })
}

pub(super) fn encode(values: &LocalPreferences) -> String {
    let mut encoded = format!(
        "nextcloud-modern-talk-preferences 6\npalette={}\nappearance={}\nscale={}\nsidebar={}\nlocale={}\nlogin={}\nbackground={}\nmessages_banners={}\nmessages_sounds={}\nimportant_banners={}\nimportant_sounds={}\nincoming_calls_banners={}\nincoming_calls_sounds={}\ncalendar_banners={}\ncalendar_sounds={}\nmicrophone={}\nspeaker={}\ncamera={}\n",
        match values.palette {
            Palette::Blue => "blue",
            Palette::Sage => "sage",
        },
        match values.appearance {
            Appearance::System => "system",
            Appearance::Light => "light",
            Appearance::Dark => "dark",
        },
        values.scale.percent(),
        match values.sidebar_density {
            SidebarDensity::Compact => "compact",
            SidebarDensity::Relaxed => "relaxed",
        },
        match values.locale {
            LocalePreference::System => "system",
            LocalePreference::English => "en",
            LocalePreference::German => "de",
            LocalePreference::Spanish => "es",
        },
        u8::from(values.launch_at_login),
        u8::from(values.start_in_background),
        u8::from(values.notifications.messages.banners),
        u8::from(values.notifications.messages.sounds),
        u8::from(values.notifications.important.banners),
        u8::from(values.notifications.important.sounds),
        u8::from(values.notifications.incoming_calls.banners),
        u8::from(values.notifications.incoming_calls.sounds),
        u8::from(values.notifications.calendar.banners),
        u8::from(values.notifications.calendar.sounds),
        encode_device(&values.devices.microphone),
        encode_device(&values.devices.speaker),
        encode_device(&values.devices.camera)
    );
    encoded.push_str(match values.quick_switcher {
        QuickSwitcherPreference::Auto => "quick_switcher=auto\n",
        QuickSwitcherPreference::On => "quick_switcher=on\n",
        QuickSwitcherPreference::Off => "quick_switcher=off\n",
    });
    encoded.push_str(match values.update_channel {
        UpdateChannel::Stable => "update_channel=stable\n",
        UpdateChannel::Prerelease => "update_channel=prerelease\n",
    });
    if let Some(resume) = &values.onboarding_resume {
        use std::fmt::Write;
        let _ = writeln!(
            encoded,
            "onboarding_resume={}:{}",
            resume.expires_at_unix_seconds,
            resume.account.as_str()
        );
    } else {
        encoded.push_str("onboarding_resume=none\n");
    }
    navigation::encode(&values.navigation, &mut encoded);
    encoded
}
fn encode_device(selection: &DeviceSelection) -> String {
    match selection {
        DeviceSelection::SystemDefault => "default".into(),
        DeviceSelection::Specific(id) => {
            let mut result = String::from("id:");
            for byte in id.as_str().bytes() {
                use std::fmt::Write;
                let _ = write!(result, "{byte:02x}");
            }
            result
        }
    }
}
fn decode_device(value: &str) -> Result<DeviceSelection, PreferenceError> {
    if value == "default" {
        return Ok(DeviceSelection::SystemDefault);
    }
    let hex = value
        .strip_prefix("id:")
        .ok_or(PreferenceError::InvalidData)?;
    if hex.is_empty()
        || hex.len() > 512
        || hex.len() % 2 != 0
        || !hex.bytes().all(|b| b.is_ascii_hexdigit())
    {
        return Err(PreferenceError::InvalidData);
    }
    let bytes = hex
        .as_bytes()
        .as_chunks::<2>()
        .0
        .iter()
        .map(|part| {
            let high = (part[0] as char)
                .to_digit(16)
                .ok_or(PreferenceError::InvalidData)?;
            let low = (part[1] as char)
                .to_digit(16)
                .ok_or(PreferenceError::InvalidData)?;
            Ok((high * 16 + low) as u8)
        })
        .collect::<Result<Vec<_>, PreferenceError>>()?;
    let value = std::str::from_utf8(&bytes).map_err(|_| PreferenceError::InvalidData)?;
    DeviceId::new(value)
        .map(DeviceSelection::Specific)
        .map_err(|_| PreferenceError::InvalidData)
}
fn boolean(value: &str) -> Result<bool, PreferenceError> {
    match value {
        "0" => Ok(false),
        "1" => Ok(true),
        _ => Err(PreferenceError::InvalidData),
    }
}
pub(super) fn decode(bytes: &[u8]) -> Result<(LocalPreferences, LoadState), PreferenceError> {
    if bytes.len() > MAX_FILE_BYTES {
        return Err(PreferenceError::TooLarge);
    }
    let data = std::str::from_utf8(bytes).map_err(|_| PreferenceError::InvalidData)?;
    let mut lines = data.lines();
    let version = match lines.next() {
        Some("nextcloud-modern-talk-preferences 1") => 1,
        Some("nextcloud-modern-talk-preferences 2") => 2,
        Some("nextcloud-modern-talk-preferences 3") => 3,
        Some("nextcloud-modern-talk-preferences 4") => 4,
        Some("nextcloud-modern-talk-preferences 5") => 5,
        Some("nextcloud-modern-talk-preferences 6") => 6,
        Some(header) if header.starts_with("nextcloud-modern-talk-preferences ") => {
            return Err(PreferenceError::UnsupportedVersion);
        }
        _ => return Err(PreferenceError::InvalidData),
    };
    let mut result = LocalPreferences::default();
    let mut seen = 0u32;
    let mut legacy = NotificationDelivery::default();
    for line in lines {
        let (key, value) = line.split_once('=').ok_or(PreferenceError::InvalidData)?;
        if version >= 4 && matches!(key, "layout_sections" | "layout_group" | "layout_room") {
            navigation::decode_row(&mut result.navigation, key, value)?;
            continue;
        }
        let index = match key {
            "palette" => {
                result.palette = match value {
                    "blue" => Palette::Blue,
                    "sage" => Palette::Sage,
                    _ => return Err(PreferenceError::InvalidData),
                };
                0
            }
            "appearance" => {
                result.appearance = match value {
                    "system" => Appearance::System,
                    "light" => Appearance::Light,
                    "dark" => Appearance::Dark,
                    _ => return Err(PreferenceError::InvalidData),
                };
                1
            }
            "scale" => {
                result.scale = match value {
                    "100" => DisplayScale::Percent100,
                    "125" => DisplayScale::Percent125,
                    "150" => DisplayScale::Percent150,
                    "200" => DisplayScale::Percent200,
                    _ => return Err(PreferenceError::InvalidData),
                };
                2
            }
            "sidebar" => {
                result.sidebar_density = match value {
                    "compact" => SidebarDensity::Compact,
                    "relaxed" => SidebarDensity::Relaxed,
                    "comfortable" if version == 1 => SidebarDensity::Relaxed,
                    _ => return Err(PreferenceError::InvalidData),
                };
                3
            }
            "locale" => {
                result.locale = match value {
                    "system" => LocalePreference::System,
                    "en" => LocalePreference::English,
                    "de" => LocalePreference::German,
                    "es" => LocalePreference::Spanish,
                    _ => return Err(PreferenceError::InvalidData),
                };
                4
            }
            "login" => {
                result.launch_at_login = boolean(value)?;
                5
            }
            "background" => {
                result.start_in_background = boolean(value)?;
                6
            }
            "sounds" if version <= 2 => {
                legacy.sounds = boolean(value)?;
                7
            }
            "banners" if version <= 2 => {
                legacy.banners = boolean(value)?;
                8
            }
            "microphone" if version >= 2 => {
                result.devices.microphone = decode_device(value)?;
                9
            }
            "speaker" if version >= 2 => {
                result.devices.speaker = decode_device(value)?;
                10
            }
            "camera" if version >= 2 => {
                result.devices.camera = decode_device(value)?;
                11
            }
            "messages_banners" if version >= 3 => {
                result.notifications.messages.banners = boolean(value)?;
                12
            }
            "messages_sounds" if version >= 3 => {
                result.notifications.messages.sounds = boolean(value)?;
                13
            }
            "important_banners" if version >= 3 => {
                result.notifications.important.banners = boolean(value)?;
                14
            }
            "important_sounds" if version >= 3 => {
                result.notifications.important.sounds = boolean(value)?;
                15
            }
            "incoming_calls_banners" if version >= 3 => {
                result.notifications.incoming_calls.banners = boolean(value)?;
                16
            }
            "incoming_calls_sounds" if version >= 3 => {
                result.notifications.incoming_calls.sounds = boolean(value)?;
                17
            }
            "calendar_banners" if version >= 3 => {
                result.notifications.calendar.banners = boolean(value)?;
                18
            }
            "calendar_sounds" if version >= 3 => {
                result.notifications.calendar.sounds = boolean(value)?;
                19
            }
            "quick_switcher" if version >= 4 => {
                result.quick_switcher = match value {
                    "auto" => QuickSwitcherPreference::Auto,
                    "on" => QuickSwitcherPreference::On,
                    "off" => QuickSwitcherPreference::Off,
                    _ => return Err(PreferenceError::InvalidData),
                };
                20
            }
            "update_channel" if version >= 5 => {
                result.update_channel = match value {
                    "stable" => UpdateChannel::Stable,
                    "prerelease" => UpdateChannel::Prerelease,
                    _ => return Err(PreferenceError::InvalidData),
                };
                21
            }
            "onboarding_resume" if version >= 6 => {
                result.onboarding_resume = if value == "none" {
                    None
                } else {
                    let (expires, account) =
                        value.split_once(':').ok_or(PreferenceError::InvalidData)?;
                    if expires.is_empty()
                        || expires.len() > 20
                        || !expires.bytes().all(|b| b.is_ascii_digit())
                    {
                        return Err(PreferenceError::InvalidData);
                    }
                    Some(OnboardingResume {
                        expires_at_unix_seconds: expires
                            .parse()
                            .map_err(|_| PreferenceError::InvalidData)?,
                        account: talk_core::preferences::navigation::NavigationAccountKey::new(
                            account,
                        )
                        .map_err(|_| PreferenceError::InvalidData)?,
                    })
                };
                22
            }
            _ => return Err(PreferenceError::InvalidData),
        };
        if seen & (1 << index) != 0 {
            return Err(PreferenceError::InvalidData);
        }
        seen |= 1 << index;
    }
    let required = match version {
        1 => 0x1ff,
        2 => 0xfff,
        3 => 0xffe7f,
        4 => 0x1ffe7f,
        5 => 0x3ffe7f,
        _ => 0x7ffe7f,
    };
    if seen != required {
        return Err(PreferenceError::InvalidData);
    }
    // v1/v2 silence is preserved independently for all previously covered categories.
    // Calendar never becomes enabled by migration. v1 has system-default devices.
    if version < 3 {
        result.notifications.messages = legacy;
        result.notifications.important = legacy;
        result.notifications.incoming_calls = legacy;
        result.notifications.calendar = NotificationDelivery::OFF;
    }
    // Existing login=false remains explicit. Older formats have no setup handoff.
    // Migration is decoded in memory; the next explicit save atomically writes v6.
    Ok((
        result,
        if version < 6 {
            LoadState::Migrated
        } else {
            LoadState::Loaded
        },
    ))
}
