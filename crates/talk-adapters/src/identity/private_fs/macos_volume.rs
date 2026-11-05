//! Read the persistent volume UUID through macOS's system disk utility. This
//! runs once on the identity worker, before any credential/provider operation.
//! stdout is bounded and never logged; the child is killed/reaped on timeout.
use super::super::error::{self, Result};
use std::{
    io::Read,
    process::{Child, Command, Stdio},
    time::{Duration, Instant},
};

fn uuid(output: &[u8]) -> Option<String> {
    let xml = std::str::from_utf8(output).ok()?;
    let key = "<key>VolumeUUID</key>";
    let (_, value) = xml.split_once(key)?;
    if value.contains(key) {
        return None;
    }
    let value = value
        .trim_start()
        .strip_prefix("<string>")?
        .split_once("</string>")?
        .0;
    if value.len() != 36
        || !value.bytes().enumerate().all(|(index, byte)| {
            if [8, 13, 18, 23].contains(&index) {
                byte == b'-'
            } else {
                byte.is_ascii_hexdigit()
            }
        })
        || value.bytes().all(|b| b == b'0' || b == b'-')
    {
        return None;
    }
    Some(value.to_ascii_lowercase())
}

#[cfg(target_os = "macos")]
pub(super) fn read(directory: &std::fs::File) -> Result<String> {
    use std::os::unix::ffi::OsStringExt;
    // diskutil expects a mounted volume, not an arbitrary descendant path.
    // fstatfs resolves the volume from the already validated directory handle,
    // including the separate APFS Data volume behind a /Users firmlink.
    let stat = rustix::fs::fstatfs(directory).map_err(|error| error::io(error.into()))?;
    let bytes: Vec<u8> = stat
        .f_mntonname
        .iter()
        .take_while(|byte| **byte != 0)
        .map(|byte| *byte as u8)
        .collect();
    let mount = std::path::PathBuf::from(std::ffi::OsString::from_vec(bytes));
    if !mount.is_absolute() {
        return Err(error::denied());
    }
    let mut command = Command::new("/usr/sbin/diskutil");
    command.args(["info", "-plist"]).arg(mount);
    run(command, Duration::from_secs(3))
}

struct OwnedChild(Child);
impl Drop for OwnedChild {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

fn run(mut command: Command, timeout: Duration) -> Result<String> {
    let unavailable = || {
        error::storage(
            talk_core::identity::StorageBackend::PrivateFile,
            talk_core::identity::StorageFailure::Unavailable,
        )
    };
    let mut child = OwnedChild(
        command
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()
            .map_err(|_| unavailable())?,
    );
    let mut output = child.0.stdout.take().ok_or_else(unavailable)?;
    let flags = rustix::fs::fcntl_getfl(&output).map_err(|_| unavailable())?;
    rustix::fs::fcntl_setfl(&output, flags | rustix::fs::OFlags::NONBLOCK)
        .map_err(|_| unavailable())?;
    let deadline = Instant::now() + timeout;
    let mut bytes = Vec::new();
    let mut status = None;
    loop {
        let mut buffer = [0u8; 4096];
        loop {
            match output.read(&mut buffer) {
                Ok(0) => break,
                Ok(count) => {
                    if bytes.len() + count > 32768 {
                        return Err(unavailable());
                    }
                    bytes.extend_from_slice(&buffer[..count]);
                }
                Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => break,
                Err(error) if error.kind() == std::io::ErrorKind::Interrupted => continue,
                Err(_) => return Err(unavailable()),
            }
        }
        if let Some(status) = status {
            return if status {
                uuid(&bytes).ok_or_else(unavailable)
            } else {
                Err(unavailable())
            };
        }
        // Drain once more after observing process exit; the last bytes may have
        // arrived between the nonblocking read and try_wait. Never wait on EOF
        // from a descendant that inherited the pipe.
        status = child
            .0
            .try_wait()
            .map_err(|_| unavailable())?
            .map(|status| status.success());
        if status.is_some() {
            continue;
        }
        if Instant::now() >= deadline {
            return Err(unavailable());
        }
        std::thread::sleep(Duration::from_millis(10));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    const VALID: &str = "<plist><dict><key>VolumeUUID</key>\n<string>ABCDEF01-2345-6789-ABCD-EF0123456789</string></dict></plist>";
    #[test]
    fn accepts_only_one_nonzero_uuid_without_retaining_other_disk_metadata() {
        assert_eq!(
            uuid(VALID.as_bytes()),
            Some("abcdef01-2345-6789-abcd-ef0123456789".into())
        );
        for invalid in [
            "",
            "<key>VolumeUUID</key><string>not-a-uuid</string>",
            "<key>VolumeUUID</key><string>00000000-0000-0000-0000-000000000000</string>",
            "<key>VolumeUUID</key><string>11111111-2222-3333-4444-555555555555</string><key>VolumeUUID</key>",
        ] {
            assert_eq!(uuid(invalid.as_bytes()), None);
        }
    }
    #[test]
    fn process_observation_is_bounded_and_rejects_failure_or_excess_output() {
        let mut echo = Command::new("/bin/echo");
        echo.arg(VALID);
        assert_eq!(
            run(echo, Duration::from_secs(2)).unwrap(),
            "abcdef01-2345-6789-abcd-ef0123456789"
        );
        let mut excess = Command::new("/bin/echo");
        excess.arg("x".repeat(32769));
        assert!(run(excess, Duration::from_secs(2)).is_err());
        assert!(run(Command::new("/usr/bin/false"), Duration::from_secs(2)).is_err());
        let mut slow = Command::new("/bin/sleep");
        slow.arg("10");
        let start = Instant::now();
        assert!(run(slow, Duration::from_millis(30)).is_err());
        assert!(start.elapsed() < Duration::from_secs(3));
    }
}
