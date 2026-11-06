//! Bounded Debian metadata inspection; no network or package installation.
use super::UpdateError;
#[cfg(target_os = "linux")]
use std::{path::Path, time::Duration};
#[cfg(target_os = "linux")]
use talk_core::lifecycle::Cancellation;

#[cfg(target_os = "linux")]
pub(in crate::updates) async fn verify_package(
    path: &Path,
    version: &str,
    cancel: &Cancellation,
) -> Result<(), UpdateError> {
    use std::process::Stdio;
    use tokio::io::AsyncReadExt;
    if cancel.is_cancelled() {
        return Err(UpdateError::Cancelled);
    }
    let mut child = tokio::process::Command::new("/usr/bin/dpkg-deb")
        .arg("--field")
        .arg(path)
        .args(["Package", "Version", "Architecture"])
        .env("LC_ALL", "C")
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .kill_on_drop(true)
        .spawn()
        .map_err(|_| UpdateError::PackageManagerUnavailable)?;
    let Some(stdout) = child.stdout.take() else {
        let _ = child.kill().await;
        let _ = child.wait().await;
        return Err(UpdateError::PackageManagerUnavailable);
    };
    let mut bytes = Vec::new();
    let read_fields = async {
        // Read the extra byte before waiting so a flooding child is killed rather
        // than blocking on a full stdout pipe. No stderr or stdin is accumulated.
        stdout
            .take(8193)
            .read_to_end(&mut bytes)
            .await
            .map_err(|_| UpdateError::UnsafeArchive)?;
        if bytes.len() > 8192 {
            return Err(UpdateError::TooLarge);
        }
        if !child
            .wait()
            .await
            .map_err(|_| UpdateError::UnsafeArchive)?
            .success()
        {
            return Err(UpdateError::UnsafeArchive);
        }
        Ok(())
    };
    let result = tokio::select! {
        result = tokio::time::timeout(Duration::from_secs(10), read_fields) => result.unwrap_or(Err(UpdateError::UnsafeArchive)),
        // Cancellation is an atomic core value, not an async runtime primitive.
        // This timer exists only during the finite child operation (at most 100
        // sleeps); the idle update owner has no polling timer.
        _ = async {
            while !cancel.is_cancelled() { tokio::time::sleep(Duration::from_millis(100)).await; }
        } => Err(UpdateError::Cancelled),
    };
    if let Err(error) = result {
        let _ = child.kill().await;
        let _ = child.wait().await;
        return Err(error);
    }
    if cancel.is_cancelled() {
        return Err(UpdateError::Cancelled);
    }
    verify_fields(&bytes, version)
}
#[cfg(any(target_os = "linux", test))]
fn verify_fields(bytes: &[u8], version: &str) -> Result<(), UpdateError> {
    let fields = std::str::from_utf8(bytes).map_err(|_| UpdateError::UnsafeArchive)?;
    let expected_version = version.replacen('-', "~", 1);
    let mut package = None;
    let mut package_version = None;
    let mut architecture = None;
    for line in fields.lines() {
        let (key, value) = line.split_once(": ").ok_or(UpdateError::UnsafeArchive)?;
        let field = match key {
            "Package" => &mut package,
            "Version" => &mut package_version,
            "Architecture" => &mut architecture,
            _ => return Err(UpdateError::UnsafeArchive),
        };
        if field.replace(value).is_some() {
            return Err(UpdateError::UnsafeArchive);
        }
    }
    if package != Some("nextcloud-modern-talk")
        || package_version != Some(expected_version.as_str())
        || architecture != Some("amd64")
    {
        return Err(UpdateError::UnsafeArchive);
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn native_fields_are_exact_and_prereleases_map_to_debian_tilde() {
        assert_eq!(
            verify_fields(
                b"Package: nextcloud-modern-talk\nVersion: 0.2.0~rc.1\nArchitecture: amd64\n",
                "0.2.0-rc.1"
            ),
            Ok(())
        );
        for fields in [
            "Package: other\nVersion: 0.2.0\nArchitecture: amd64\n",
            "Package: nextcloud-modern-talk\nVersion: 0.1.0\nArchitecture: amd64\n",
            "Package: nextcloud-modern-talk\nVersion: 0.2.0\nArchitecture: arm64\n",
            "Package: nextcloud-modern-talk\nPackage: nextcloud-modern-talk\nVersion: 0.2.0\nArchitecture: amd64\n",
        ] {
            assert_eq!(
                verify_fields(fields.as_bytes(), "0.2.0"),
                Err(UpdateError::UnsafeArchive)
            );
        }
    }
}
