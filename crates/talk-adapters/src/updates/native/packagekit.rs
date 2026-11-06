//! PackageKit D-Bus transactions, with explicit completion and uncertainty.
//! https://github.com/PackageKit/PackageKit/blob/v1.3.1/src/org.freedesktop.PackageKit.Transaction.xml
use super::UpdateError;
use std::{path::Path, time::Duration};
const SERVICE: &str = "org.freedesktop.PackageKit";
const CONTROL: &str = "/org/freedesktop/PackageKit";

pub(in crate::updates) async fn available() -> Result<(), UpdateError> {
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
pub(in crate::updates) async fn install(path: &Path) -> Result<(), UpdateError> {
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
async fn install_on_connection(
    connection: &zbus::Connection,
    path: &Path,
) -> Result<(), UpdateError> {
    use futures_util::StreamExt;
    let path = path.to_str().ok_or(UpdateError::Storage)?;
    let control = zbus::Proxy::new(connection, SERVICE, CONTROL, SERVICE)
        .await
        .map_err(|_| UpdateError::PackageManagerUnavailable)?;
    let transaction_path: zbus::OwnedObjectPath = tokio::time::timeout(
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
            zbus::proxy::MethodFlags::ALLOW_INTERACTIVE_AUTH,
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
        async fn create_transaction(&self) -> zbus::OwnedObjectPath {
            zbus::OwnedObjectPath::try_from("/synthetic/transaction").unwrap()
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
                .name(SERVICE)
                .serve_at(CONTROL, ControlFixture)
                .serve_at(
                    "/synthetic/transaction",
                    TransactionFixture { exit: exit.clone() },
                )
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
