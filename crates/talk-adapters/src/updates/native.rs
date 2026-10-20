//! Debian installation stays with PackageKit and its native polkit interaction.
//! D-Bus signatures/enums: PackageKit v1.3.1 Transaction.xml and pk-enum.h.
//! https://github.com/PackageKit/PackageKit/blob/v1.3.1/src/org.freedesktop.PackageKit.Transaction.xml
use super::UpdateError;
use std::path::Path;
#[cfg(target_os = "linux")]
use std::time::Duration;
use talk_core::lifecycle::Cancellation;

#[cfg(target_os = "linux")]
const SERVICE: &str = "org.freedesktop.PackageKit";
#[cfg(target_os = "linux")]
const CONTROL: &str = "/org/freedesktop/PackageKit";

#[cfg(target_os = "linux")]
pub(super) async fn available() -> Result<(), UpdateError> {
    tokio::time::timeout(Duration::from_secs(8), async {
        let connection = zbus::Connection::system()
            .await
            .map_err(|_| UpdateError::PackageManagerUnavailable)?;
        let proxy = zbus::Proxy::new(&connection, SERVICE, CONTROL, SERVICE)
            .await
            .map_err(|_| UpdateError::PackageManagerUnavailable)?;
        let roles: u64 = proxy
            .get_property("Roles")
            .await
            .map_err(|_| UpdateError::PackageManagerUnavailable)?;
        // PK_ROLE_ENUM_INSTALL_FILES is ordinal 10; Roles is its bitfield.
        if roles & (1u64 << 10) == 0 || !Path::new("/usr/bin/dpkg-deb").is_file() {
            return Err(UpdateError::PackageManagerUnavailable);
        }
        Ok(())
    })
    .await
    .map_err(|_| UpdateError::PackageManagerUnavailable)?
}
#[cfg(not(target_os = "linux"))]
pub(super) async fn available() -> Result<(), UpdateError> {
    Err(UpdateError::UnsupportedInstall)
}

#[cfg(target_os = "linux")]
pub(super) async fn verify_package(
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
#[cfg(not(target_os = "linux"))]
pub(super) async fn verify_package(_: &Path, _: &str, _: &Cancellation) -> Result<(), UpdateError> {
    Err(UpdateError::UnsupportedInstall)
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

#[cfg(target_os = "linux")]
pub(super) async fn install(path: &Path) -> Result<(), UpdateError> {
    tokio::time::timeout(Duration::from_secs(18 * 60), async {
        let connection = tokio::time::timeout(Duration::from_secs(8), zbus::Connection::system())
            .await
            .map_err(|_| UpdateError::PackageManagerUnavailable)?
            .map_err(|_| UpdateError::PackageManagerUnavailable)?;
        install_on_connection(&connection, path).await
    })
    .await
    .map_err(|_| UpdateError::InstallUncertain)?
}
#[cfg(target_os = "linux")]
async fn install_on_connection(
    connection: &zbus::Connection,
    path: &Path,
) -> Result<(), UpdateError> {
    use futures_util::StreamExt;
    let path = path.to_str().ok_or(UpdateError::Storage)?;
    let control = zbus::Proxy::new(connection, SERVICE, CONTROL, SERVICE)
        .await
        .map_err(|_| UpdateError::PackageManagerUnavailable)?;
    let transaction_path: zbus::zvariant::OwnedObjectPath = tokio::time::timeout(
        Duration::from_secs(8),
        control.call("CreateTransaction", &()),
    )
    .await
    .map_err(|_| UpdateError::PackageManagerUnavailable)?
    .map_err(|_| UpdateError::PackageManagerUnavailable)?;
    let transaction = zbus::Proxy::new(
        connection,
        SERVICE,
        transaction_path.as_str(),
        "org.freedesktop.PackageKit.Transaction",
    )
    .await
    .map_err(|_| UpdateError::PackageManagerUnavailable)?;
    // Subscribe before sending InstallFiles so even an immediate failure is observed.
    let mut finished = transaction
        .receive_signal("Finished")
        .await
        .map_err(|_| UpdateError::PackageManagerUnavailable)?;
    let hints: Result<(), _> = tokio::time::timeout(
        Duration::from_secs(8),
        transaction.call("SetHints", &(vec!["interactive=true"],)),
    )
    .await
    .map_err(|_| UpdateError::PackageManagerUnavailable)?;
    hints.map_err(|_| UpdateError::PackageManagerUnavailable)?;
    // No ALLOW_DOWNGRADE or ALLOW_REINSTALL flag is set. A local unsigned .deb
    // still goes through PackageKit's explicit polkit authentication policy.
    let submitted: Result<Option<()>, _> = match tokio::time::timeout(
        Duration::from_secs(120),
        transaction.call_with_flags(
            "InstallFiles",
            zbus::proxy::MethodFlags::AllowInteractiveAuth.into(),
            &(0u64, vec![path]),
        ),
    )
    .await
    {
        Ok(result) => result,
        Err(_) => return cancel_and_settle(&transaction, &mut finished).await,
    };
    if !matches!(submitted, Ok(Some(()))) {
        // A method error can race authentication/transaction startup. Establish a
        // terminal result before declaring that a previous installation is usable.
        return cancel_and_settle(&transaction, &mut finished).await;
    }
    match tokio::time::timeout(Duration::from_secs(15 * 60), finished.next()).await {
        Ok(Some(message)) => finished_result(&message),
        _ => cancel_and_settle(&transaction, &mut finished).await,
    }
}
#[cfg(target_os = "linux")]
fn finished_result(message: &zbus::Message) -> Result<(), UpdateError> {
    let (exit, _runtime): (u32, u32) = message
        .body()
        .deserialize()
        .map_err(|_| UpdateError::InstallUncertain)?;
    if exit == 1 {
        Ok(())
    } else {
        Err(UpdateError::PackageManagerFailed)
    }
}
#[cfg(target_os = "linux")]
async fn cancel_and_settle(
    transaction: &zbus::Proxy<'_>,
    finished: &mut zbus::proxy::SignalStream<'_>,
) -> Result<(), UpdateError> {
    use futures_util::StreamExt;
    let _ = tokio::time::timeout(
        Duration::from_secs(5),
        transaction.call::<_, _, ()>("Cancel", &()),
    )
    .await;
    match tokio::time::timeout(Duration::from_secs(30), finished.next()).await {
        Ok(Some(message)) => finished_result(&message),
        _ => Err(UpdateError::InstallUncertain),
    }
}
#[cfg(not(target_os = "linux"))]
pub(super) async fn install(_: &Path) -> Result<(), UpdateError> {
    Err(UpdateError::UnsupportedInstall)
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

#[cfg(all(test, target_os = "linux"))]
mod private_bus_tests {
    use super::*;
    use std::sync::{
        Arc,
        atomic::{AtomicU32, Ordering},
    };
    struct ControlFixture;
    #[zbus::interface(name = "org.freedesktop.PackageKit")]
    impl ControlFixture {
        async fn create_transaction(&self) -> zbus::zvariant::OwnedObjectPath {
            zbus::zvariant::OwnedObjectPath::try_from("/synthetic/transaction").unwrap()
        }
    }
    struct TransactionFixture {
        exit: Arc<AtomicU32>,
    }
    #[zbus::interface(name = "org.freedesktop.PackageKit.Transaction")]
    impl TransactionFixture {
        async fn set_hints(&self, hints: Vec<String>) -> zbus::fdo::Result<()> {
            if hints == ["interactive=true"] {
                Ok(())
            } else {
                Err(zbus::fdo::Error::InvalidArgs("unexpected hints".into()))
            }
        }
        async fn install_files(
            &self,
            flags: u64,
            paths: Vec<String>,
            #[zbus(signal_emitter)] emitter: zbus::object_server::SignalEmitter<'_>,
        ) -> zbus::fdo::Result<()> {
            if flags != 0 || paths != ["/synthetic/release.deb"] {
                return Err(zbus::fdo::Error::InvalidArgs(
                    "unexpected fixture request".into(),
                ));
            }
            // Immediate completion intentionally precedes the method reply; the
            // production owner must have subscribed before submitting InstallFiles.
            emitter
                .emit(
                    "org.freedesktop.PackageKit.Transaction",
                    "Finished",
                    &(self.exit.load(Ordering::Acquire), 0u32),
                )
                .await
                .map_err(|_| zbus::fdo::Error::Failed("fixture signal".into()))
        }
    }
    #[test]
    #[ignore = "run only under an explicitly created private dbus-run-session"]
    fn packagekit_private_service_failure_and_success_are_terminal() {
        assert_eq!(
            std::env::var("NMT_PRIVATE_PACKAGEKIT_FIXTURE").as_deref(),
            Ok("1")
        );
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();
        runtime.block_on(async {
            let client = zbus::Connection::session().await.unwrap();
            let missing = tokio::time::timeout(
                Duration::from_secs(12),
                install_on_connection(&client, Path::new("/synthetic/release.deb")),
            )
            .await
            .unwrap();
            assert_eq!(missing, Err(UpdateError::PackageManagerUnavailable));
            let exit = Arc::new(AtomicU32::new(2));
            let _server = zbus::connection::Builder::session()
                .unwrap()
                .name(SERVICE)
                .unwrap()
                .serve_at(CONTROL, ControlFixture)
                .unwrap()
                .serve_at(
                    "/synthetic/transaction",
                    TransactionFixture { exit: exit.clone() },
                )
                .unwrap()
                .build()
                .await
                .unwrap();
            assert_eq!(
                tokio::time::timeout(
                    Duration::from_secs(12),
                    install_on_connection(&client, Path::new("/synthetic/release.deb"))
                )
                .await
                .unwrap(),
                Err(UpdateError::PackageManagerFailed)
            );
            exit.store(1, Ordering::Release);
            assert_eq!(
                tokio::time::timeout(
                    Duration::from_secs(12),
                    install_on_connection(&client, Path::new("/synthetic/release.deb"))
                )
                .await
                .unwrap(),
                Ok(())
            );
        });
    }
}
