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
        let options: HashMap<&str, zbus::Variant<'_>> = HashMap::new();
        let _: zbus::OwnedObjectPath = proxy
            .call("OpenURI", &("", url, options))
            .await
            .map_err(|_| ErrorKind::Unsupported)?;
        Ok(())
    };
    bounded_handoff(cancel, std::time::Duration::from_secs(3), call).await
}

#[cfg(target_os = "linux")]
async fn bounded_handoff(
    cancel: &Cancellation,
    deadline: std::time::Duration,
    handoff: impl std::future::Future<Output = Result<()>>,
) -> Result<()> {
    if cancel.is_cancelled() {
        return Err(ErrorKind::Cancelled.into());
    }
    // The core token is atomic. This timer exists only for this finite handoff,
    // never as an idle task. Retire the connection future on cancellation too.
    let cancelled = async {
        while !cancel.is_cancelled() {
            tokio::time::sleep(std::time::Duration::from_millis(25)).await;
        }
    };
    tokio::select! {
        biased;
        _ = cancelled => Err(ErrorKind::Cancelled.into()),
        result = tokio::time::timeout(deadline, handoff) => {
            if cancel.is_cancelled() {
                Err(ErrorKind::Cancelled.into())
            } else {
                result.map_err(|_| ErrorKind::Deadline)?
            }
        }
    }
}

#[cfg(all(test, target_os = "linux"))]
mod tests {
    use super::*;
    use std::time::Duration;

    #[tokio::test]
    async fn cancelled_handoff_never_dispatches_and_inflight_work_retires_before_its_deadline() {
        let cancel = Cancellation::default();
        cancel.cancel();
        let result = bounded_handoff(&cancel, Duration::from_secs(3), async {
            panic!("cancelled browser handoff was dispatched")
        })
        .await;
        assert_eq!(result, Err(ErrorKind::Cancelled.into()));

        let cancel = Cancellation::default();
        let entered = tokio::sync::Notify::new();
        let handoff = bounded_handoff(&cancel, Duration::from_secs(3), async {
            entered.notify_one();
            std::future::pending().await
        });
        let cancelling = async {
            entered.notified().await;
            cancel.cancel();
        };
        let (result, ()) = tokio::time::timeout(Duration::from_secs(1), async {
            tokio::join!(handoff, cancelling)
        })
        .await
        .unwrap();
        assert_eq!(result, Err(ErrorKind::Cancelled.into()));
    }
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
