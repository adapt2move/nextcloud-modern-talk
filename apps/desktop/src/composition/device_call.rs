//! Opt-in macOS sender for macos_device_call.py. The foreground runs AppKit;
//! the device worker retains ownership of its streams throughout the exercise.
use crate::devices::DeviceRuntime;
use gpui::Application;
use std::{
    path::{Path, PathBuf},
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    time::Duration,
};
use talk_adapters::{
    media::MediaService,
    nextcloud::test_support::device_call::{self, Scenario},
};
use talk_core::preferences::{RequestId, Revision, RuntimeState, devices::*};

pub(crate) fn run() -> i32 {
    let mut args = std::env::args_os().skip(2);
    let scenario = match args.next().as_deref().and_then(|arg| arg.to_str()) {
        Some("camera") => Scenario::Camera,
        Some("microphone") => Scenario::Microphone,
        _ => return 2,
    };
    let encrypted = match args.next().as_deref().and_then(|arg| arg.to_str()) {
        Some("required") => true,
        Some("disabled") => false,
        _ => return 2,
    };
    let Some(fixture) = args.next().map(PathBuf::from) else {
        return 2;
    };
    if args.next().is_some() || !cfg!(target_os = "macos") {
        return 2;
    }

    let passed = Arc::new(AtomicBool::new(false));
    let result = passed.clone();
    Application::new().run(move |cx| {
        let worker = cx
            .background_executor()
            .spawn(async move { send(&fixture, scenario, encrypted) });
        cx.spawn(async move |cx| {
            match worker.await {
                Ok(()) => result.store(true, Ordering::Release),
                Err(stage) => eprintln!("device call failed at {stage}"),
            }
            let _ = cx.update(|cx| cx.quit());
        })
        .detach();
    });
    i32::from(!passed.load(Ordering::Acquire))
}

fn send(fixture: &Path, scenario: Scenario, encrypted: bool) -> Result<(), &'static str> {
    let directory = fixture.parent().ok_or("fixture directory")?;
    let logs = directory.join("logs");
    let diagnostics = crate::diagnostics::start_with_path(move || Ok(logs));
    let media = MediaService::new();
    let devices = DeviceRuntime::start().map_err(|_| "device owner")?;
    if scenario == Scenario::Camera {
        let missing = DeviceId::new("fixture:absent-microphone").map_err(|_| "fixture device")?;
        devices
            .submit(DeviceIntent::ApplySelection {
                request: RequestId(1),
                revision: Revision(1),
                values: DevicePreferences {
                    microphone: DeviceSelection::Specific(missing),
                    ..Default::default()
                },
            })
            .map_err(|_| "device selection")?;
    }
    devices.attach_media(media.clone());
    let result = device_call::run(fixture, scenario, encrypted, media, |scope| {
        devices
            .pause_probe_microphone(scope)
            .map_err(|_| "pause native microphone")
    });
    devices.request_shutdown();
    let deadline = std::time::Instant::now() + Duration::from_secs(5);
    while devices.snapshot().runtime != RuntimeState::Stopped {
        if std::time::Instant::now() >= deadline {
            return Err("device shutdown");
        }
        std::thread::sleep(Duration::from_millis(30));
    }
    drop(diagnostics);
    result?;
    std::fs::write(directory.join("finished"), []).map_err(|_| "completion marker")
}
