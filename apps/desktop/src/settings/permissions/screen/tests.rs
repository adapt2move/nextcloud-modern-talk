use super::*;
use std::{
    cell::Cell,
    collections::HashMap,
    path::PathBuf,
    process::{Command, Stdio},
    sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    },
};

struct DropProbe<'a>(&'a Cell<bool>);
impl Drop for DropProbe<'_> {
    fn drop(&mut self) {
        self.0.set(true);
    }
}

#[test]
fn cancellation_prevents_io_and_drops_the_unpolled_probe() {
    let polled = Cell::new(false);
    let dropped = Cell::new(false);
    let guard = DropProbe(&dropped);
    let read = async {
        let _guard = guard;
        polled.set(true);
        Ok(4)
    };
    assert_eq!(
        observe(read, || true, CHECK_TIMEOUT),
        Err(PermissionError::Cancelled)
    );
    assert!(!polled.get());
    assert!(dropped.get());
}

#[test]
fn cancellation_drops_an_inflight_probe_within_the_poll_window() {
    let cancel = Cell::new(false);
    let dropped = Cell::new(false);
    let guard = DropProbe(&dropped);
    let read = async {
        let _guard = guard;
        cancel.set(true);
        std::future::pending().await
    };
    let started = Instant::now();
    assert_eq!(
        observe(read, || cancel.get(), CHECK_TIMEOUT),
        Err(PermissionError::Cancelled)
    );
    // Scheduling tolerance; a lost cancellation would instead take three seconds.
    assert!(started.elapsed() < Duration::from_secs(1));
    assert!(dropped.get());
}

#[test]
fn timeout_drops_the_probe_and_cancellation_wins_a_ready_result() {
    let dropped = Cell::new(false);
    let guard = DropProbe(&dropped);
    assert_eq!(
        observe(
            async {
                let _guard = guard;
                std::future::pending().await
            },
            || false,
            Duration::from_millis(30),
        ),
        Err(PermissionError::TimedOut)
    );
    assert!(dropped.get());

    let cancel = Cell::new(false);
    assert_eq!(
        observe(
            async {
                cancel.set(true);
                Ok(4)
            },
            || cancel.get(),
            CHECK_TIMEOUT,
        ),
        Err(PermissionError::Cancelled)
    );
}

// Separate processes are necessary because ashpd keeps a process-wide session
// bus connection. The bus has no activation directories, so an absent service
// cannot accidentally start the host's real portal or any permission UI.
struct PrivateBusConfig(PathBuf);
impl PrivateBusConfig {
    fn new() -> Self {
        let path = std::env::temp_dir().join(format!(
            "talk-screen-portal-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir(&path).unwrap();
        std::fs::write(
            path.join("bus.conf"),
            r#"<busconfig>
<type>session</type><listen>unix:tmpdir=/tmp</listen><auth>EXTERNAL</auth>
<policy context="default">
<allow send_destination="*"/><allow receive_sender="*"/><allow own="*"/>
</policy>
</busconfig>"#,
        )
        .unwrap();
        Self(path)
    }
}
impl Drop for PrivateBusConfig {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

#[test]
fn typed_portal_fixtures_on_private_buses() {
    let bus = PrivateBusConfig::new();
    for case in [
        "no-bus",
        "no-service",
        "no-interface",
        "no-version",
        "zero-version",
        "bad-version",
        "available",
        "refused",
        "cancelled",
        "timeout",
    ] {
        // GNU timeout bounds the fixture's whole process group, including its
        // private bus, even if a test or a dependency regresses and stops yielding.
        let mut command = Command::new("timeout");
        command.args(["--kill-after=1s", "15s"]);
        if case != "no-bus" {
            command
                .args(["dbus-run-session", "--config-file"])
                .arg(bus.0.join("bus.conf"))
                .arg("--");
        }
        command
            .arg(std::env::current_exe().unwrap())
            .args([
                "--exact",
                "settings::permissions::screen::tests::portal_fixture_child",
                "--ignored",
                "--nocapture",
            ])
            .env("TALK_SCREEN_PORTAL_FIXTURE", case)
            .stdin(Stdio::null());
        if case == "no-bus" {
            command.env(
                "DBUS_SESSION_BUS_ADDRESS",
                format!("unix:path={}/absent-bus", bus.0.display()),
            );
        }
        let status = command
            .status()
            .expect("fixture requires GNU timeout and D-Bus tools");
        assert!(status.success(), "private portal fixture {case}: {status}");
    }
}

const PORTAL_PATH: &str = "/org/freedesktop/portal/desktop";
const PORTAL_NAME: &str = "org.freedesktop.portal.Desktop";

struct PortalFixture {
    case: String,
    reads: Arc<AtomicUsize>,
    session_calls: Arc<AtomicUsize>,
}
impl PortalFixture {
    fn reject_session<T>(&self) -> zbus::fdo::Result<T> {
        self.session_calls.fetch_add(1, Ordering::SeqCst);
        Err(zbus::fdo::Error::NotSupported(
            "no capture in fixture".into(),
        ))
    }
}

#[zbus::interface(name = "org.freedesktop.portal.ScreenCast", crate = "ashpd::zbus")]
impl PortalFixture {
    #[zbus(property, name = "version")]
    async fn version(&self) -> zbus::fdo::Result<u32> {
        self.reads.fetch_add(1, Ordering::SeqCst);
        match self.case.as_str() {
            "no-version" => Err(zbus::fdo::Error::UnknownProperty("missing version".into())),
            "zero-version" => Ok(0),
            "refused" => Err(zbus::fdo::Error::AccessDenied("property refused".into())),
            "cancelled" | "timeout" => std::future::pending().await,
            _ => Ok(4),
        }
    }

    fn create_session(
        &self,
        _options: HashMap<String, zbus::zvariant::OwnedValue>,
    ) -> zbus::fdo::Result<zbus::zvariant::OwnedObjectPath> {
        self.reject_session()
    }

    fn select_sources(
        &self,
        _session: zbus::zvariant::ObjectPath<'_>,
        _options: HashMap<String, zbus::zvariant::OwnedValue>,
    ) -> zbus::fdo::Result<zbus::zvariant::OwnedObjectPath> {
        self.reject_session()
    }

    fn start(
        &self,
        _session: zbus::zvariant::ObjectPath<'_>,
        _parent_window: &str,
        _options: HashMap<String, zbus::zvariant::OwnedValue>,
    ) -> zbus::fdo::Result<zbus::zvariant::OwnedObjectPath> {
        self.reject_session()
    }

    fn open_pipe_wire_remote(
        &self,
        _session: zbus::zvariant::ObjectPath<'_>,
        _options: HashMap<String, zbus::zvariant::OwnedValue>,
    ) -> zbus::fdo::Result<zbus::zvariant::OwnedFd> {
        self.reject_session()
    }
}

struct OtherInterface;
#[zbus::interface(name = "org.example.Other", crate = "ashpd::zbus")]
impl OtherInterface {
    #[zbus(property)]
    fn unrelated(&self) -> u32 {
        1
    }
}

struct BadVersion;
#[zbus::interface(name = "org.freedesktop.portal.ScreenCast", crate = "ashpd::zbus")]
impl BadVersion {
    #[zbus(property, name = "version")]
    fn version(&self) -> &str {
        "4"
    }
}

#[test]
#[ignore = "child process of typed_portal_fixtures_on_private_buses; requires fixture environment"]
fn portal_fixture_child() {
    let case = std::env::var("TALK_SCREEN_PORTAL_FIXTURE").expect("private fixture case");
    let reads = Arc::new(AtomicUsize::new(0));
    let session_calls = Arc::new(AtomicUsize::new(0));
    let service = if matches!(case.as_str(), "no-bus" | "no-service") {
        None
    } else {
        let builder = zbus::connection::Builder::session()
            .unwrap()
            .name(PORTAL_NAME)
            .unwrap();
        let builder = match case.as_str() {
            "no-interface" => builder.serve_at(PORTAL_PATH, OtherInterface),
            "bad-version" => builder.serve_at(PORTAL_PATH, BadVersion),
            _ => builder.serve_at(
                PORTAL_PATH,
                PortalFixture {
                    case: case.clone(),
                    reads: reads.clone(),
                    session_calls: session_calls.clone(),
                },
            ),
        }
        .unwrap();
        Some(async_io::block_on(builder.build()).unwrap())
    };
    let started = Instant::now();
    let result = check(|| case == "cancelled" && reads.load(Ordering::SeqCst) > 0);
    match case.as_str() {
        "cancelled" => {
            assert_eq!(result, Err(PermissionError::Cancelled));
            assert!(started.elapsed() < Duration::from_secs(1));
        }
        "timeout" => assert_eq!(result, Err(PermissionError::TimedOut)),
        _ => {
            let observation = result.unwrap();
            let expected = match case.as_str() {
                "no-bus" | "no-service" | "no-interface" | "zero-version" => {
                    PermissionStatus::Unavailable
                }
                "no-version" | "bad-version" | "refused" => PermissionStatus::Unknown,
                "available" => PermissionStatus::PerSession,
                _ => panic!("unknown fixture"),
            };
            assert_eq!(observation.status, expected, "case {case}");
            assert_eq!(observation.kind, PermissionKind::ScreenSharing);
            assert!(!observation.can_request);
            assert!(!observation.can_open_settings);
            assert!(!observation.allows_device_preview());
        }
    }
    assert_eq!(session_calls.load(Ordering::SeqCst), 0);
    if let Some(service) = service {
        async_io::block_on(service.close()).unwrap();
    }
}
