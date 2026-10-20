//! Per-user login registration. A saved switch is not evidence of OS application.
//! Entries are reconciled on startup, so a crash between the preference commit and
//! registration is repairable. No credentials, shell interpolation or privileged service.
use super::storage::{atomic_write, read_bounded};
use std::{
    env, fs, io,
    path::{Path, PathBuf},
};
use talk_core::preferences::{EffectState, LocalPreferences, PreferenceError, StartupSnapshot};

pub(super) trait StartupEffect: Send {
    fn apply(&mut self, values: &LocalPreferences) -> StartupSnapshot;
}
#[derive(Clone, Copy)]
pub(super) enum StartupFormat {
    DesktopEntry,
    LaunchAgent,
    WindowsStartup,
}
pub(super) struct StartupRegistration {
    pub path: PathBuf,
    pub executable: PathBuf,
    pub format: StartupFormat,
}

pub(super) fn current_paths() -> Result<(PathBuf, StartupRegistration), PreferenceError> {
    let home = env::var_os(if cfg!(windows) { "USERPROFILE" } else { "HOME" })
        .map(PathBuf::from)
        .filter(|path| path.is_absolute())
        .ok_or(PreferenceError::StorageUnavailable)?;
    let executable = env::current_exe().map_err(|_| PreferenceError::StartupUnavailable)?;
    let (file, path, format) = if cfg!(target_os = "macos") {
        (
            home.join("Library/Application Support/NextCloud Modern Talk/preferences.conf"),
            home.join("Library/LaunchAgents/com.adapt2move.nextcloud-modern-talk.plist"),
            StartupFormat::LaunchAgent,
        )
    } else if cfg!(windows) {
        let appdata = env::var_os("APPDATA")
            .map(PathBuf::from)
            .filter(|p| p.is_absolute())
            .ok_or(PreferenceError::StorageUnavailable)?;
        (
            appdata.join("NextCloud Modern Talk/preferences.conf"),
            appdata.join("Microsoft/Windows/Start Menu/Programs/Startup/NextCloud Modern Talk.cmd"),
            StartupFormat::WindowsStartup,
        )
    } else if cfg!(target_os = "linux") {
        let config = env::var_os("XDG_CONFIG_HOME")
            .map(PathBuf::from)
            .filter(|p| p.is_absolute())
            .unwrap_or_else(|| home.join(".config"));
        (
            config.join("nextcloud-modern-talk/preferences.conf"),
            config.join("autostart/nextcloud-modern-talk.desktop"),
            StartupFormat::DesktopEntry,
        )
    } else {
        return Err(PreferenceError::StartupUnavailable);
    };
    Ok((
        file,
        StartupRegistration {
            path,
            executable,
            format,
        },
    ))
}
const MARKER: &str = "NextCloud Modern Talk managed startup";
impl StartupRegistration {
    fn contents(&self) -> Result<String, PreferenceError> {
        let executable = self
            .executable
            .to_str()
            .filter(|s| s.len() <= 4096 && !s.chars().any(char::is_control))
            .ok_or(PreferenceError::StartupUnavailable)?;
        if !self.executable.is_absolute() || !self.executable.is_file() {
            return Err(PreferenceError::StartupUnavailable);
        }
        Ok(match self.format {
            StartupFormat::DesktopEntry => {
                // Desktop Exec has its own quoting rules; percent introduces field codes.
                let escaped = executable
                    .replace('\\', "\\\\\\\\")
                    .replace('"', "\\\"")
                    .replace('$', "\\$")
                    .replace('`', "\\`")
                    .replace('%', "%%");
                format!(
                    "# {MARKER}\n[Desktop Entry]\nType=Application\nName=NextCloud Modern Talk\nExec=\"{escaped}\" --autostart\nTerminal=false\nX-GNOME-Autostart-enabled=true\n"
                )
            }
            StartupFormat::LaunchAgent => format!(
                "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n<!-- {MARKER} -->\n<!DOCTYPE plist PUBLIC \"-//Apple//DTD PLIST 1.0//EN\" \"http://www.apple.com/DTDs/PropertyList-1.0.dtd\">\n<plist version=\"1.0\"><dict><key>Label</key><string>com.adapt2move.nextcloud-modern-talk</string><key>ProgramArguments</key><array><string>{}</string><string>--autostart</string></array><key>RunAtLoad</key><true/><key>LimitLoadToSessionType</key><string>Aqua</string></dict></plist>\n",
                xml(executable)
            ),
            StartupFormat::WindowsStartup => {
                if executable.contains('"') || executable.contains('%') {
                    return Err(PreferenceError::StartupUnavailable);
                }
                format!("@echo off\r\nREM {MARKER}\r\nstart \"\" \"{executable}\" --autostart\r\n")
            }
        })
    }
    fn observe(&self) -> Result<Option<Vec<u8>>, PreferenceError> {
        match read_bounded(&self.path, 16384) {
            Ok(bytes) if std::str::from_utf8(&bytes).is_ok_and(|s| s.contains(MARKER)) => {
                Ok(Some(bytes))
            }
            Ok(_) => Err(PreferenceError::StartupUnavailable),
            Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(None),
            Err(e) => Err(super::storage::io_error(e)),
        }
    }
}
impl StartupEffect for StartupRegistration {
    fn apply(&mut self, values: &LocalPreferences) -> StartupSnapshot {
        let existing = match self.observe() {
            Ok(value) => value,
            Err(error) => {
                return StartupSnapshot {
                    registered: None,
                    state: EffectState::Failed(error),
                };
            }
        };
        let actual = existing.is_some();
        let result = if values.launch_at_login {
            self.contents().and_then(|contents| {
                if existing.as_deref() == Some(contents.as_bytes()) {
                    return Ok(());
                }
                atomic_write(&self.path, contents.as_bytes()).map_err(super::storage::io_error)
            })
        } else if actual {
            fs::remove_file(&self.path)
                .map_err(super::storage::io_error)
                .and_then(|()| sync_parent(&self.path))
        } else {
            Ok(())
        };
        match result {
            Ok(()) => StartupSnapshot {
                registered: Some(values.launch_at_login),
                state: EffectState::Applied,
            },
            Err(error) => StartupSnapshot {
                registered: self.observe().ok().map(|v| v.is_some()),
                state: EffectState::Failed(error),
            },
        }
    }
}
fn sync_parent(path: &Path) -> Result<(), PreferenceError> {
    #[cfg(unix)]
    {
        fs::File::open(path.parent().ok_or(PreferenceError::StartupUnavailable)?)
            .and_then(|file| file.sync_all())
            .map_err(super::storage::io_error)?;
    }
    #[cfg(not(unix))]
    let _ = path;
    Ok(())
}
fn xml(value: &str) -> String {
    value
        .replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
        .replace('\'', "&apos;")
}

/// Resolved finite observation; ordered preferred languages stay off the render
/// boundary. A failed OS query falls back to English, never changes saved System.
pub(super) fn system_locale() -> talk_core::preferences::Locale {
    use talk_core::preferences::LocalePreference;
    let raw = if cfg!(target_os = "macos") {
        language_command("/usr/bin/defaults", &["read", "-g", "AppleLanguages"])
    } else if cfg!(windows) {
        language_command(
            "powershell.exe",
            &[
                "-NoProfile",
                "-NonInteractive",
                "-Command",
                "(Get-WinUserLanguageList).LanguageTag",
            ],
        )
    } else {
        let mut values = String::new();
        for key in ["LANGUAGE", "LC_ALL", "LC_MESSAGES", "LANG"] {
            if let Ok(value) = env::var(key)
                && value.len() <= 1024
            {
                values.push_str(&value);
                values.push(':');
            }
        }
        Some(values)
    };
    let raw = raw.unwrap_or_default();
    let tags: Vec<&str> = raw
        .split(|c: char| c.is_whitespace() || matches!(c, ':' | ',' | '"' | '(' | ')'))
        .filter(|s| !s.is_empty())
        .take(32)
        .collect();
    LocalePreference::System.resolve(&tags)
}
fn language_command(program: &str, args: &[&str]) -> Option<String> {
    use std::{
        io::Read,
        process::{Command, Stdio},
        time::{Duration, Instant},
    };
    let mut child = Command::new(program)
        .args(args)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .ok()?;
    let stdout = child.stdout.take()?;
    // One bounded reader during initial observation; timeout kills/reaps producer.
    let reader = match std::thread::Builder::new()
        .name("talk-languages".into())
        .spawn(move || {
            let mut bytes = Vec::with_capacity(4097);
            stdout.take(4097).read_to_end(&mut bytes).ok()?;
            (bytes.len() <= 4096).then_some(bytes)
        }) {
        Ok(reader) => reader,
        Err(_) => {
            let _ = child.kill();
            let _ = child.wait();
            return None;
        }
    };
    let deadline = Instant::now() + Duration::from_secs(3);
    let success = loop {
        match child.try_wait() {
            Ok(Some(status)) => break status.success(),
            Err(_) => break false,
            Ok(None) => {}
        }
        if Instant::now() >= deadline {
            break false;
        }
        std::thread::sleep(Duration::from_millis(20));
    };
    if !success {
        let _ = child.kill();
        let _ = child.wait();
    }
    let bytes = reader.join().ok()??;
    if success {
        String::from_utf8(bytes).ok()
    } else {
        None
    }
}
