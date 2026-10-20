//! One bounded pipe reader per active native operation, killed/reaped on cancellation.
use std::{
    io::Read,
    path::PathBuf,
    process::{Command, Stdio},
    sync::mpsc,
    time::{Duration, Instant},
};
use talk_core::preferences::permissions::PermissionError;

pub(super) fn mac_helper() -> Result<PathBuf, PermissionError> {
    let executable = std::env::current_exe().map_err(|_| PermissionError::Unavailable)?;
    let directory = executable.parent().ok_or(PermissionError::Unavailable)?;
    // Only the app-bundled sibling participates in the application's OS identity.
    if directory.file_name().is_none_or(|name| name != "MacOS")
        || directory
            .parent()
            .and_then(|p| p.file_name())
            .is_none_or(|name| name != "Contents")
    {
        return Err(PermissionError::Unavailable);
    }
    let path = directory.join("talk-native-helper");
    if !path.is_file() {
        return Err(PermissionError::Unavailable);
    }
    Ok(path)
}

/// Launchers can pass inherited handles to another app. Do not create a pipe
/// whose reader would then wait for that unrelated application's lifetime.
pub(super) fn launch(
    command: &mut Command,
    timeout: Duration,
    cancelled: impl Fn() -> bool,
) -> Result<(), PermissionError> {
    if cancelled() {
        return Err(PermissionError::Cancelled);
    }
    let mut child = command
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .map_err(|_| PermissionError::Unavailable)?;
    let deadline = Instant::now() + timeout;
    let result = loop {
        match child.try_wait() {
            Ok(Some(status)) => {
                break if status.success() {
                    Ok(())
                } else {
                    Err(PermissionError::Failed)
                };
            }
            Err(_) => break Err(PermissionError::Failed),
            _ => {}
        }
        if cancelled() {
            break Err(PermissionError::Cancelled);
        }
        if Instant::now() >= deadline {
            break Err(PermissionError::TimedOut);
        }
        std::thread::sleep(Duration::from_millis(20));
    };
    if result.is_err() {
        let _ = child.kill();
    }
    let _ = child.wait();
    result
}

pub(super) fn capture(
    command: &mut Command,
    timeout: Duration,
    cancelled: impl Fn() -> bool,
) -> Result<String, PermissionError> {
    if cancelled() {
        return Err(PermissionError::Cancelled);
    }
    let mut child = command
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .map_err(|_| PermissionError::Unavailable)?;
    let stdout = child.stdout.take().ok_or(PermissionError::Failed)?;
    let (send, receive) = mpsc::sync_channel(1);
    let reader = match std::thread::Builder::new()
        .name("talk-native-output".into())
        .spawn(move || {
            let mut bytes = Vec::with_capacity(256);
            let result = stdout.take(4097).read_to_end(&mut bytes).map(|_| bytes);
            let _ = send.send(result);
        }) {
        Ok(reader) => reader,
        Err(_) => {
            let _ = child.kill();
            let _ = child.wait();
            return Err(PermissionError::Unavailable);
        }
    };
    let deadline = Instant::now() + timeout;
    let mut bytes = None;
    let result = loop {
        if let Ok(output) = receive.try_recv() {
            match output {
                Ok(output) if output.len() <= 4096 => bytes = Some(output),
                _ => break Err(PermissionError::Failed),
            }
        }
        match child.try_wait() {
            Ok(Some(status)) if !status.success() => break Err(PermissionError::Failed),
            Ok(Some(_)) if bytes.is_some() => break Ok(()),
            Err(_) => break Err(PermissionError::Failed),
            _ => {}
        }
        if cancelled() {
            break Err(PermissionError::Cancelled);
        }
        if Instant::now() >= deadline {
            break Err(PermissionError::TimedOut);
        }
        std::thread::sleep(Duration::from_millis(20));
    };
    if result.is_err() {
        let _ = child.kill();
    }
    let _ = child.wait();
    let _ = reader.join();
    result?;
    String::from_utf8(bytes.ok_or(PermissionError::Failed)?).map_err(|_| PermissionError::Failed)
}
