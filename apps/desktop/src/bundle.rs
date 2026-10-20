//! Configure the private macOS media runtime before application threads exist.
//!
//! `Command::env` changes only the replacement process environment. This avoids
//! process-global environment mutation (unsafe in Rust 2024), a shell launcher,
//! and user-installed GStreamer. Library load commands are already relative to
//! the bundle; GStreamer's separately loaded plugins/scanner need these paths.
#[cfg(any(target_os = "macos", test))]
use std::{
    ffi::OsString,
    path::{Path, PathBuf},
};

#[cfg(any(target_os = "macos", test))]
fn environment(executable: &Path) -> Option<[(&'static str, OsString); 6]> {
    let macos = executable.parent()?;
    let contents = macos.parent()?;
    let app = contents.parent()?;
    if executable.file_name()? != "talk-desktop"
        || macos.file_name()? != "MacOS"
        || contents.file_name()? != "Contents"
        || app.extension()? != "app"
    {
        return None;
    }
    let plugins = contents.join("PlugIns").into_os_string();
    let scanner = contents.join("Helpers/gst-plugin-scanner").into_os_string();
    Some([
        ("GST_PLUGIN_SYSTEM_PATH_1_0", plugins.clone()),
        ("GST_PLUGIN_SYSTEM_PATH", plugins),
        ("GST_PLUGIN_PATH_1_0", OsString::new()),
        ("GST_PLUGIN_PATH", OsString::new()),
        ("GST_PLUGIN_SCANNER_1_0", scanner.clone()),
        ("GST_PLUGIN_SCANNER", scanner),
    ])
}

#[cfg(any(target_os = "macos", test))]
fn matches_environment(
    expected: &[(&str, OsString)],
    mut read: impl FnMut(&str) -> Option<OsString>,
) -> bool {
    expected
        .iter()
        .all(|(key, value)| read(key).as_ref() == Some(value))
}

#[cfg(any(target_os = "macos", test))]
fn required_files(executable: &Path) -> Option<[PathBuf; 3]> {
    let contents = executable.parent()?.parent()?;
    Some([
        contents.join("Frameworks/libgstreamer-1.0.0.dylib"),
        contents.join("PlugIns/libgstcoreelements.dylib"),
        contents.join("Helpers/gst-plugin-scanner"),
    ])
}

pub(crate) fn prepare() -> std::io::Result<()> {
    #[cfg(target_os = "macos")]
    {
        use std::os::unix::process::CommandExt;
        let executable = std::env::current_exe()?;
        let Some(expected) = environment(&executable) else {
            return Ok(()); // Ordinary Cargo/developer binaries retain SDK discovery.
        };
        let required = required_files(&executable)
            .ok_or_else(|| std::io::Error::from(std::io::ErrorKind::InvalidInput))?;
        for path in required {
            let metadata = std::fs::symlink_metadata(path)?;
            if !metadata.is_file() || metadata.file_type().is_symlink() {
                return Err(std::io::ErrorKind::InvalidInput.into());
            }
        }
        if !matches_environment(&expected, |key| std::env::var_os(key)) {
            // exec preserves PID, cwd and OS arguments; it returns only on error.
            let mut arguments = std::env::args_os();
            let mut replacement = std::process::Command::new(&executable);
            if let Some(name) = arguments.next() {
                replacement.arg0(name);
            }
            return Err(replacement.args(arguments).envs(expected).exec());
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn paths_follow_a_moved_bundle_and_preserve_non_ascii_names() {
        let executable =
            Path::new("/Applications/Team Ü/NextCloud Modern Talk.app/Contents/MacOS/talk-desktop");
        let values = environment(executable).unwrap();
        assert_eq!(
            values[0].1,
            "/Applications/Team Ü/NextCloud Modern Talk.app/Contents/PlugIns"
        );
        assert_eq!(
            values[4].1,
            "/Applications/Team Ü/NextCloud Modern Talk.app/Contents/Helpers/gst-plugin-scanner"
        );
        assert_eq!(
            required_files(executable).unwrap()[0],
            PathBuf::from(
                "/Applications/Team Ü/NextCloud Modern Talk.app/Contents/Frameworks/libgstreamer-1.0.0.dylib"
            )
        );
    }

    #[test]
    fn ordinary_developer_executables_are_unchanged() {
        for path in [
            "/build/target/release/talk-desktop",
            "/tmp/Example/Contents/MacOS/talk-desktop",
            "/tmp/Example.app/Contents/Helpers/gst-plugin-scanner",
        ] {
            assert!(environment(Path::new(path)).is_none());
        }
    }

    #[test]
    fn every_variable_must_match_before_skipping_reexec() {
        let expected = environment(Path::new("/tmp/Talk.app/Contents/MacOS/talk-desktop")).unwrap();
        assert!(matches_environment(&expected, |key| expected
            .iter()
            .find(|(name, _)| *name == key)
            .map(|(_, value)| value.clone())));
        for changed in 0..expected.len() {
            assert!(!matches_environment(&expected, |key| expected
                .iter()
                .enumerate()
                .find(|(_, (name, _))| *name == key)
                .map(|(index, (_, value))| if index == changed {
                    OsString::from("/old/runtime")
                } else {
                    value.clone()
                })));
        }
        assert!(!matches_environment(&expected, |_| None));
    }
}
