//! OS browser handoff. URL never appears in child argv/environment or renderer values.
use super::error::Result;
use talk_core::{lifecycle::Cancellation, ports::ErrorKind};
#[cfg(target_os = "linux")]
pub(crate) async fn open(url: &str, cancel: &Cancellation) -> Result<()> {
    use std::collections::HashMap;
    let call = async {
        let connection = zbus::Connection::session()
            .await
            .map_err(|_| ErrorKind::Unsupported)?;
        let proxy = zbus::Proxy::new(
            &connection,
            "org.freedesktop.portal.Desktop",
            "/org/freedesktop/portal/desktop",
            "org.freedesktop.portal.OpenURI",
        )
        .await
        .map_err(|_| ErrorKind::Unsupported)?;
        let options: HashMap<&str, zbus::zvariant::Value<'_>> = HashMap::new();
        let _: zbus::zvariant::OwnedObjectPath = proxy
            .call("OpenURI", &("", url, options))
            .await
            .map_err(|_| ErrorKind::Unsupported)?;
        Ok(())
    };
    if cancel.is_cancelled() {
        return Err(ErrorKind::Cancelled.into());
    }
    let result = tokio::time::timeout(std::time::Duration::from_secs(3), call)
        .await
        .map_err(|_| ErrorKind::Deadline)?;
    if cancel.is_cancelled() {
        return Err(ErrorKind::Cancelled.into());
    }
    result
}
#[cfg(any(target_os = "macos", target_os = "windows"))]
pub(crate) async fn open(url: &str, cancel: &Cancellation) -> Result<()> {
    use std::{
        io::Write,
        process::{Command, Stdio},
        time::{Duration, Instant},
    };
    #[cfg(target_os = "macos")]
    let mut command = {
        let mut c = Command::new("/usr/bin/osascript");
        c.args(["-l","JavaScript","-e", "ObjC.import('AppKit'); ObjC.import('Foundation'); const data=$.NSFileHandle.fileHandleWithStandardInput.readDataToEndOfFile; const u=$.NSString.alloc.initWithDataEncoding(data,$.NSUTF8StringEncoding); if (!$.NSWorkspace.sharedWorkspace.openURL($.NSURL.URLWithString(u))) throw Error('browser');"]);
        c
    };
    #[cfg(target_os = "windows")]
    let mut command = {
        let mut c = Command::new("powershell.exe");
        c.args(["-NoProfile","-NonInteractive","-Command","$u=[Console]::In.ReadToEnd(); $p=New-Object System.Diagnostics.ProcessStartInfo; $p.FileName=$u; $p.UseShellExecute=$true; [System.Diagnostics.Process]::Start($p) | Out-Null"]);
        c
    };
    if cancel.is_cancelled() {
        return Err(ErrorKind::Cancelled.into());
    }
    let mut child = command
        .stdin(Stdio::piped())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .map_err(|_| ErrorKind::Unsupported)?;
    if let Some(mut stdin) = child.stdin.take()
        && stdin.write_all(url.as_bytes()).is_err()
    {
        let _ = child.kill();
        let _ = child.wait();
        return Err(ErrorKind::Unsupported.into());
    }
    let until = Instant::now() + Duration::from_secs(3);
    loop {
        if let Some(status) = child.try_wait().map_err(|_| ErrorKind::Unsupported)? {
            return if status.success() {
                Ok(())
            } else {
                Err(ErrorKind::Unsupported.into())
            };
        }
        if cancel.is_cancelled() || Instant::now() >= until {
            let _ = child.kill();
            let _ = child.wait();
            return Err(ErrorKind::Cancelled.into());
        }
        tokio::time::sleep(Duration::from_millis(25)).await;
    }
}
