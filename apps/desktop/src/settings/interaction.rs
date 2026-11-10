//! Cached OS presentation preferences. The existing settings worker owns all OS work.
#[cfg(target_os = "linux")]
use super::native_process;
#[cfg(target_os = "linux")]
use std::time::Duration;

#[derive(Clone, Copy)]
pub(super) struct Profile {
    pub reduced_motion: bool,
    pub reduced_transparency: bool,
}
impl Default for Profile {
    fn default() -> Self {
        Self {
            reduced_motion: true,
            reduced_transparency: true,
        }
    }
}

pub(super) struct State {
    pub profile: Profile,
    pub refresh: bool,
}
impl Default for State {
    fn default() -> Self {
        Self {
            profile: Profile::default(),
            refresh: !cfg!(test),
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
            reduced_transparency: cancelled()
                || objc2_app_kit::NSWorkspace::sharedWorkspace()
                    .accessibilityDisplayShouldReduceTransparency(),
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
            reduced_transparency: (!cancelled())
                .then(windows::UI::ViewManagement::UISettings::new)
                .and_then(Result::ok)
                .and_then(|settings| settings.AdvancedEffectsEnabled().ok())
                .map(|value| !value)
                .unwrap_or(true),
        }
    }
    #[cfg(target_os = "linux")]
    {
        use std::process::Command;
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
        Profile {
            reduced_motion,
            // No common Linux desktop setting reliably describes transparency.
            reduced_transparency: true,
        }
    }
    #[cfg(not(any(target_os = "macos", target_os = "windows", target_os = "linux")))]
    {
        let _ = cancelled;
        Profile::default()
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
        assert!(Profile::default().reduced_transparency);
    }
}
