//! Cached OS presentation preferences and one deliberate native emoji action.
//! The existing settings worker owns all OS work. The picker owns its UI after
//! activation; admission is not a claim that a character was selected or inserted.
#[cfg(target_os = "linux")]
use super::native_process;
use std::time::{Duration, Instant};
use talk_core::preferences::PreferenceError;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum Picker {
    Unavailable,
    #[cfg(target_os = "windows")]
    Windows,
    #[cfg(target_os = "linux")]
    IBus,
    #[cfg(target_os = "linux")]
    Plasma,
}

#[derive(Clone, Copy)]
pub(super) struct Profile {
    pub reduced_motion: bool,
    pub picker: Picker,
}
impl Default for Profile {
    fn default() -> Self {
        Self {
            reduced_motion: true,
            picker: Picker::Unavailable,
        }
    }
}

pub(super) struct State {
    pub profile: Profile,
    pub refresh: bool,
    pub pending: Option<(u64, Instant)>,
    pub in_flight: bool,
    pub generation: u64,
    pub error: Option<PreferenceError>,
}
impl Default for State {
    fn default() -> Self {
        Self {
            profile: Profile::default(),
            refresh: !cfg!(test),
            pending: None,
            in_flight: false,
            generation: 0,
            error: None,
        }
    }
}
impl State {
    pub fn admit(&mut self, now: Instant) -> Result<(), PreferenceError> {
        if self.profile.picker == Picker::Unavailable {
            return Err(PreferenceError::WorkerUnavailable);
        }
        if self.pending.is_some() || self.in_flight {
            return Err(PreferenceError::Busy);
        }
        let generation = self
            .generation
            .checked_add(1)
            .ok_or(PreferenceError::RevisionExhausted)?;
        self.generation = generation;
        self.error = None;
        // A delayed global OS shortcut must not suddenly appear after an
        // unrelated storage operation or context switch.
        self.pending = Some((generation, now + Duration::from_millis(500)));
        Ok(())
    }

    pub fn cancel(&mut self) {
        // At most one action is in flight; wrapping cannot match its previous
        // generation and prevents an exhausted counter from accepting that result.
        self.generation = self.generation.wrapping_add(1);
        self.pending = None;
        self.error = None;
    }

    pub fn finish(&mut self, generation: u64, result: Result<(), PreferenceError>) {
        self.in_flight = false;
        if generation == self.generation {
            self.error = result.err();
        }
    }
}

pub(super) fn observe(cancelled: impl Fn() -> bool) -> Profile {
    #[cfg(target_os = "macos")]
    {
        // This read needs neither a bundle identity nor permission. Using the
        // permission helper here made every cargo-run build appear to request
        // reduced motion, permanently disabling its caret and arrival animation.
        let reduced = cancelled()
            || objc2_app_kit::NSWorkspace::sharedWorkspace()
                .accessibilityDisplayShouldReduceMotion();
        Profile {
            reduced_motion: reduced,
            // GPUI opens the character palette in the actual app process.
            picker: Picker::Unavailable,
        }
    }
    #[cfg(target_os = "windows")]
    {
        let enabled = (!cancelled())
            .then(windows::UI::ViewManagement::UISettings::new)
            .and_then(Result::ok)
            .and_then(|settings| settings.AnimationsEnabled().ok());
        Profile {
            reduced_motion: enabled.map(|value| !value).unwrap_or(true),
            picker: Picker::Windows,
        }
    }
    #[cfg(target_os = "linux")]
    {
        use std::{path::Path, process::Command};
        let plasma = std::env::var("XDG_CURRENT_DESKTOP")
            .is_ok_and(|name| name.split(':').any(|part| part.eq_ignore_ascii_case("KDE")));
        let reduced_motion = if plasma {
            native_process::capture(
                Command::new("/usr/bin/kreadconfig6").args([
                    "--file",
                    "kdeglobals",
                    "--group",
                    "KDE",
                    "--key",
                    "AnimationDurationFactor",
                ]),
                Duration::from_secs(2),
                &cancelled,
            )
            .ok()
            .and_then(|value| value.trim().parse::<f64>().ok())
            .filter(|value| value.is_finite() && *value >= 0.0)
            .map(|factor| factor == 0.0)
            .unwrap_or(true)
        } else {
            native_process::capture(
                Command::new("/usr/bin/gsettings").args([
                    "get",
                    "org.gnome.desktop.interface",
                    "enable-animations",
                ]),
                Duration::from_secs(2),
                &cancelled,
            )
            .ok()
            .and_then(|value| parse_bool(&value))
            .map(|enabled| !enabled)
            .unwrap_or(true)
        };
        let picker = if plasma
            && Path::new("/usr/bin/gio").is_file()
            && Path::new("/usr/share/applications/org.kde.plasma.emojier.desktop").is_file()
        {
            Picker::Plasma
        } else if Path::new("/usr/bin/ibus").is_file()
            && [
                "/usr/libexec/ibus-ui-emojier",
                "/usr/lib/ibus/ibus-ui-emojier",
            ]
            .iter()
            .any(|path| Path::new(path).is_file())
        {
            Picker::IBus
        } else {
            Picker::Unavailable
        };
        Profile {
            reduced_motion,
            picker,
        }
    }
    #[cfg(not(any(target_os = "macos", target_os = "windows", target_os = "linux")))]
    {
        let _ = cancelled;
        Profile::default()
    }
}

pub(super) fn open(picker: Picker, cancelled: impl Fn() -> bool) -> Result<(), PreferenceError> {
    if cancelled() {
        return Err(PreferenceError::Closed);
    }
    match picker {
        Picker::Unavailable => Err(PreferenceError::WorkerUnavailable),
        #[cfg(target_os = "linux")]
        Picker::IBus | Picker::Plasma => {
            // Both installed system pickers copy the selected emoji. The view
            // says to paste it; never read/paste arbitrary clipboard data here.
            let mut command = if picker == Picker::IBus {
                let mut command = std::process::Command::new("/usr/bin/ibus");
                command.arg("emoji");
                command
            } else {
                let mut command = std::process::Command::new("/usr/bin/gio");
                command.args([
                    "launch",
                    "/usr/share/applications/org.kde.plasma.emojier.desktop",
                ]);
                command
            };
            native_process::launch(&mut command, Duration::from_secs(2), cancelled)
                .map_err(|_| PreferenceError::WorkerUnavailable)
        }
        #[cfg(target_os = "windows")]
        Picker::Windows => {
            use enigo::{Direction, Enigo, Key, Keyboard, Settings};
            let mut keyboard =
                Enigo::new(&Settings::default()).map_err(|_| PreferenceError::WorkerUnavailable)?;
            // Windows' documented emoji shortcut. OEM_PERIOD is a virtual key,
            // not Unicode text, so it invokes the panel instead of inserting '.'.
            keyboard
                .key(Key::Meta, Direction::Press)
                .map_err(|_| PreferenceError::WorkerUnavailable)?;
            let pressed = keyboard.key(Key::Other(0xbe), Direction::Click);
            let released = keyboard.key(Key::Meta, Direction::Release);
            pressed
                .and(released)
                .map_err(|_| PreferenceError::WorkerUnavailable)
        }
    }
}

#[cfg(any(target_os = "linux", test))]
fn parse_bool(value: &str) -> Option<bool> {
    match value.trim() {
        "true" => Some(true),
        "false" => Some(false),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn motion_observation_never_converts_unknown_into_permission_to_animate() {
        for value in ["", "1", "0", "yes", "True", "error\nfalse"] {
            assert_eq!(parse_bool(value), None);
        }
        assert_eq!(parse_bool("false\n"), Some(false));
        assert_eq!(parse_bool("true\n"), Some(true));
        assert!(Profile::default().reduced_motion);
    }

    #[test]
    fn cancelled_picker_result_cannot_become_the_next_contexts_error() {
        let mut state = State {
            in_flight: true,
            generation: 7,
            ..State::default()
        };
        state.cancel();
        state.finish(7, Err(PreferenceError::WorkerUnavailable));
        assert!(state.error.is_none());
        assert!(!state.in_flight);
    }

    #[test]
    fn cancelled_action_does_not_open_an_os_picker() {
        assert_eq!(
            open(Picker::Unavailable, || true),
            Err(PreferenceError::Closed)
        );
    }

    #[test]
    fn cancellation_at_generation_limit_still_rejects_the_old_result() {
        let mut state = State {
            generation: u64::MAX,
            in_flight: true,
            ..State::default()
        };
        state.cancel();
        state.finish(u64::MAX, Err(PreferenceError::ApplyFailed));
        assert!(state.error.is_none());
    }
}
