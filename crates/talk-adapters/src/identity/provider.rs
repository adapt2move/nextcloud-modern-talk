//! Synchronous OS/file operations. Called only by the two owned provider workers.
use super::{
    error::{self, IdentityError, Result},
    private_fs::PrivateDir,
};
#[cfg(any(target_os = "macos", target_os = "windows"))]
use keyring_core::{Entry, api::CredentialStoreApi};
#[cfg(any(target_os = "macos", target_os = "windows"))]
use std::collections::HashMap;
use std::sync::Arc;
use talk_core::identity::{QualifiedItem, StorageBackend, StorageFailure};
use zeroize::{Zeroize, Zeroizing};

#[cfg(target_os = "linux")]
mod linux;

pub(super) const RECORD_LIMIT: usize = 16 * 1024;
pub(super) fn native_backend() -> StorageBackend {
    #[cfg(target_os = "macos")]
    {
        StorageBackend::MacKeychain
    }
    #[cfg(target_os = "windows")]
    {
        StorageBackend::WindowsCredential
    }
    #[cfg(not(any(target_os = "windows", target_os = "macos")))]
    {
        StorageBackend::LinuxSecretService
    }
}
pub(super) fn key(item: QualifiedItem) -> String {
    format!(
        "p{:032x}-s{:032x}-a{:032x}-g{:032x}-app-v2",
        item.provider.profile.as_local(),
        item.provider.store,
        item.account.as_local(),
        item.generation.as_local()
    )
}
pub(super) fn filename(item: QualifiedItem) -> String {
    // Root already binds profile and store; filenames carry no account/server text.
    format!(
        "a{:032x}-g{:032x}.app",
        item.account.as_local(),
        item.generation.as_local()
    )
}
fn encode(item: QualifiedItem, password: &str) -> Result<Zeroizing<String>> {
    if password.is_empty() || password.len() > 8192 || password.chars().any(char::is_control) {
        return Err(error::storage(
            item.provider.backend,
            StorageFailure::Corrupt,
        ));
    }
    let value = Zeroizing::new(format!(
        "NMTAPP2\n{:?}\n{}\n{}",
        item.provider.backend,
        key(item),
        password
    ));
    if value.len() > RECORD_LIMIT {
        return Err(error::storage(
            item.provider.backend,
            StorageFailure::Corrupt,
        ));
    }
    Ok(value)
}
fn decode(item: QualifiedItem, data: Zeroizing<String>) -> Result<Zeroizing<String>> {
    if data.len() > RECORD_LIMIT {
        return Err(error::storage(
            item.provider.backend,
            StorageFailure::Corrupt,
        ));
    }
    let prefix = format!("NMTAPP2\n{:?}\n{}\n", item.provider.backend, key(item));
    let value = data
        .strip_prefix(&prefix)
        .ok_or_else(|| error::storage(item.provider.backend, StorageFailure::Corrupt))?;
    if value.is_empty() || value.len() > 8192 || value.chars().any(char::is_control) {
        return Err(error::storage(
            item.provider.backend,
            StorageFailure::Corrupt,
        ));
    }
    Ok(Zeroizing::new(value.to_owned()))
}

pub(super) enum WriteResult {
    /// No setter/create dispatched; a new deliberate choice may safely proceed.
    NotWritten(IdentityError),
    /// Worker returned, read-back matched, file installation flushed if applicable.
    Present,
    /// Dispatched without sufficient settlement/presence evidence. Never retry blindly.
    Unknown(IdentityError),
}
pub(super) struct Providers {
    pub directory: Arc<PrivateDir>,
    pub _lock: Arc<std::fs::File>,
    #[cfg(test)]
    pub faults: Arc<faults::Faults>,
}
impl Providers {
    pub fn new(directory: Arc<PrivateDir>, lock: Arc<std::fs::File>) -> Self {
        Self {
            directory,
            _lock: lock,
            #[cfg(test)]
            faults: Arc::default(),
        }
    }
    #[cfg(any(target_os = "macos", target_os = "windows"))]
    fn entry(&self, item: QualifiedItem) -> Result<Entry> {
        let backend = item.provider.backend;
        if backend != native_backend() {
            return Err(error::storage(backend, StorageFailure::Unsupported));
        }
        let service = "org.adapt2move.nextcloud-modern-talk";
        let user = key(item);
        #[cfg(target_os = "macos")]
        {
            let config = HashMap::from([("keychain", "User")]);
            let store =
                apple_native_keyring_store::keychain::Store::new_with_configuration(&config)
                    .map_err(|e| native_error(backend, e))?;
            store
                .build(service, &user, None)
                .map_err(|e| native_error(backend, e))
        }
        #[cfg(target_os = "windows")]
        {
            let target = format!("{service}.{user}");
            let mods = HashMap::from([("target", target.as_str()), ("persistence", "Local")]);
            let store =
                windows_native_keyring_store::Store::new().map_err(|e| native_error(backend, e))?;
            store
                .build(service, &user, Some(&mods))
                .map_err(|e| native_error(backend, e))
        }
    }
    pub fn read(&self, item: QualifiedItem) -> Result<Zeroizing<String>> {
        let value = if item.provider.backend == StorageBackend::PrivateFile {
            let bytes = self.directory.read(&filename(item), RECORD_LIMIT)?;
            let s = std::str::from_utf8(&bytes).map_err(|_| error::corrupt())?;
            Zeroizing::new(s.to_owned())
        } else {
            #[cfg(target_os = "linux")]
            {
                linux::read(self, item)?
            }
            #[cfg(any(target_os = "macos", target_os = "windows"))]
            {
                Zeroizing::new(
                    self.entry(item)?
                        .get_password()
                        .map_err(|e| native_error(item.provider.backend, e))?,
                )
            }
        };
        decode(item, value)
    }
    pub fn write(&self, item: QualifiedItem, password: &str) -> WriteResult {
        #[cfg(test)]
        let _fault = self.faults.before_write();
        #[cfg(test)]
        if self
            .faults
            .not_written
            .swap(false, std::sync::atomic::Ordering::AcqRel)
        {
            return WriteResult::NotWritten(error::storage(
                item.provider.backend,
                StorageFailure::AccessDenied,
            ));
        }
        let value = match encode(item, password) {
            Ok(v) => v,
            Err(e) => return WriteResult::NotWritten(e),
        };
        if item.provider.backend == StorageBackend::PrivateFile {
            // A collision is never overwritten; staged partials remain ledger-owned.
            if let Err(e) = self.directory.check() {
                return WriteResult::NotWritten(e);
            }
            if let Err(e) = self.directory.create(&filename(item), value.as_bytes()) {
                return WriteResult::Unknown(e);
            }
        } else {
            #[cfg(target_os = "linux")]
            match linux::create(self, item, &value) {
                WriteResult::Present => {}
                other => return other,
            }
            #[cfg(any(target_os = "macos", target_os = "windows"))]
            {
                let entry = match self.entry(item) {
                    Ok(e) => e,
                    Err(e) => return WriteResult::NotWritten(e),
                };
                match entry.get_password() {
                    Err(keyring_core::Error::NoEntry) => {}
                    Ok(mut existing) => {
                        existing.zeroize();
                        return WriteResult::NotWritten(error::storage(
                            item.provider.backend,
                            StorageFailure::Conflict,
                        ));
                    }
                    Err(e) => {
                        return WriteResult::NotWritten(native_error(item.provider.backend, e));
                    }
                }
                if let Err(e) = entry.set_password(&value) {
                    return WriteResult::Unknown(native_error(item.provider.backend, e));
                }
            }
        }
        match self.read(item) {
            Ok(actual) if actual.as_str() == password => WriteResult::Present,
            Ok(_) => WriteResult::Unknown(error::storage(
                item.provider.backend,
                StorageFailure::Corrupt,
            )),
            Err(e) => WriteResult::Unknown(e),
        }
    }
    /// Caller must own exact cleanup authority and already-proved no-writer settlement.
    pub fn delete(&self, item: QualifiedItem) -> Result<()> {
        if item.provider.backend == StorageBackend::PrivateFile {
            self.directory.remove(&filename(item))?;
            self.directory
                .remove(&format!("{}.stage", filename(item)))?;
        } else {
            #[cfg(target_os = "linux")]
            linux::delete(self, item)?;
            #[cfg(any(target_os = "macos", target_os = "windows"))]
            match self.entry(item)?.delete_credential() {
                Ok(()) | Err(keyring_core::Error::NoEntry) => {}
                Err(e) => return Err(native_error(item.provider.backend, e)),
            }
        }
        match self.read(item) {
            Err(IdentityError::Storage {
                failure: StorageFailure::Missing,
                ..
            }) => Ok(()),
            Err(e) => Err(e),
            Ok(_) => Err(error::storage(item.provider.backend, StorageFailure::Other)),
        }
    }
}

/// Fault barriers exist only in unit-test builds, never backend-test/production.
#[cfg(test)]
pub(super) mod faults {
    #[cfg(target_os = "linux")]
    pub fn legacy_create(
        item: talk_core::identity::QualifiedItem,
        password: &str,
    ) -> super::Result<()> {
        use keyring_core::api::CredentialStoreApi;
        let store = zbus_secret_service_keyring_store::Store::new()
            .map_err(|e| super::native_error(item.provider.backend, e))?;
        let modifiers = std::collections::HashMap::from([("target", "default")]);
        let entry = store
            .build(
                "org.adapt2move.nextcloud-modern-talk",
                &super::key(item),
                Some(&modifiers),
            )
            .map_err(|e| super::native_error(item.provider.backend, e))?;
        entry
            .set_password(&super::encode(item, password)?)
            .map_err(|e| super::native_error(item.provider.backend, e))
    }
    use std::sync::{
        Arc, Condvar, Mutex,
        atomic::{AtomicBool, Ordering},
    };
    #[derive(Default)]
    pub struct Gate {
        pub entered: AtomicBool,
        pub completed: AtomicBool,
        released: Mutex<bool>,
        changed: Condvar,
    }
    impl Gate {
        pub fn release(&self) {
            *self.released.lock().unwrap() = true;
            self.changed.notify_all();
        }
        fn wait(self: Arc<Self>) -> Completion {
            self.entered.store(true, Ordering::Release);
            let mut released = self.released.lock().unwrap();
            while !*released {
                released = self.changed.wait(released).unwrap();
            }
            drop(released);
            Completion(self)
        }
    }
    pub struct Completion(Arc<Gate>);
    impl Drop for Completion {
        fn drop(&mut self) {
            self.0.completed.store(true, Ordering::Release);
        }
    }
    #[derive(Default)]
    pub struct Faults {
        pub not_written: AtomicBool,
        pub settled: std::sync::atomic::AtomicUsize,
        gates: Mutex<std::collections::VecDeque<Arc<Gate>>>,
        replies: Mutex<Option<Arc<ReplyGate>>>,
        #[cfg(target_os = "linux")]
        preflight: Mutex<Option<Arc<Gate>>>,
        suppression: Mutex<Option<Arc<Gate>>>,
    }
    #[derive(Default)]
    pub struct ReplyGate {
        pub entered: AtomicBool,
        released: AtomicBool,
        changed: tokio::sync::Notify,
    }
    impl ReplyGate {
        pub fn release(&self) {
            self.released.store(true, Ordering::Release);
            self.changed.notify_one();
        }
        async fn wait(&self) {
            self.entered.store(true, Ordering::Release);
            while !self.released.load(Ordering::Acquire) {
                self.changed.notified().await;
            }
        }
    }
    impl Faults {
        #[cfg(unix)]
        pub fn arm_suppression(&self) -> Arc<Gate> {
            let gate = Arc::new(Gate::default());
            assert!(
                self.suppression
                    .lock()
                    .unwrap()
                    .replace(gate.clone())
                    .is_none()
            );
            gate
        }
        pub fn before_suppression(&self) -> Option<Completion> {
            let gate = self.suppression.lock().unwrap().take();
            gate.map(Gate::wait)
        }
        pub fn arm_reply(&self) -> Arc<ReplyGate> {
            let gate = Arc::new(ReplyGate::default());
            assert!(self.replies.lock().unwrap().replace(gate.clone()).is_none());
            gate
        }
        pub async fn after_reply(&self) {
            let gate = self.replies.lock().unwrap().take();
            if let Some(gate) = gate {
                gate.wait().await;
            }
        }
        #[cfg(target_os = "linux")]
        pub fn arm_preflight(&self) -> Arc<Gate> {
            let gate = Arc::new(Gate::default());
            assert!(
                self.preflight
                    .lock()
                    .unwrap()
                    .replace(gate.clone())
                    .is_none()
            );
            gate
        }
        #[cfg(target_os = "linux")]
        pub fn after_preflight(&self) -> Option<Completion> {
            let gate = self.preflight.lock().unwrap().take();
            gate.map(Gate::wait)
        }
        pub fn arm(&self) -> Arc<Gate> {
            let gate = Arc::new(Gate::default());
            let mut gates = self.gates.lock().unwrap();
            assert!(gates.len() < 2);
            gates.push_back(gate.clone());
            gate
        }
        pub fn before_write(&self) -> Option<Completion> {
            let gate = self.gates.lock().unwrap().pop_front();
            gate.map(Gate::wait)
        }
    }
}

fn native_error(backend: StorageBackend, mut e: keyring_core::Error) -> IdentityError {
    use keyring_core::Error as E;
    let category = match &e {
        E::NoEntry => StorageFailure::Missing,
        E::BadEncoding(_) | E::BadDataFormat(_, _) | E::BadStoreFormat(_) => {
            StorageFailure::Corrupt
        }
        E::Ambiguous(_) => StorageFailure::Conflict,
        E::NotSupportedByStore(_) | E::NoDefaultStore => StorageFailure::Unsupported,
        E::Invalid(_, _) | E::TooLong(_, _) => StorageFailure::Other,
        _ => StorageFailure::Other,
    };
    #[cfg(any(target_os = "linux", target_os = "macos", target_os = "windows"))]
    let mut category = category;
    #[cfg(target_os = "linux")]
    if let E::PlatformFailure(source) | E::NoStorageAccess(source) = &e
        && let Some(s) = source.downcast_ref::<secret_service::Error>()
    {
        category = match s {
            secret_service::Error::Locked => StorageFailure::Locked,
            secret_service::Error::Prompt => StorageFailure::Cancelled,
            secret_service::Error::Unavailable => StorageFailure::Unavailable,
            secret_service::Error::PromptDisconnected => StorageFailure::TemporarilyUnavailable,
            secret_service::Error::Zbus(z) => dbus_error(z),
            secret_service::Error::ZbusFdo(z) => fdo_error(z),
            _ => StorageFailure::Other,
        };
    }
    #[cfg(target_os = "macos")]
    if let E::PlatformFailure(source) | E::NoStorageAccess(source) = &e
        && let Some(status) = source.downcast_ref::<security_framework::base::Error>()
    {
        category = mac_status(status.code());
    }
    #[cfg(target_os = "windows")]
    if matches!(&e, E::NoStorageAccess(_)) {
        // Pinned windows-native-keyring-store 1.1.0 maps only
        // ERROR_NO_SUCH_LOGON_SESSION to this variant. Its private numeric error
        // wrapper has no public accessor; other opaque failures remain Other.
        category = StorageFailure::Unavailable;
    }
    // Native Error owns raw bytes on these paths. No Debug/Display/source chain escapes.
    if let E::BadEncoding(bytes) | E::BadDataFormat(bytes, _) = &mut e {
        bytes.zeroize();
    }
    error::storage(backend, category)
}
#[cfg(target_os = "linux")]
fn fdo_error(e: &zbus::fdo::Error) -> StorageFailure {
    match e {
        zbus::fdo::Error::ServiceUnknown(_) | zbus::fdo::Error::NameHasNoOwner(_) => {
            StorageFailure::Unavailable
        }
        zbus::fdo::Error::AccessDenied(_) | zbus::fdo::Error::AuthFailed(_) => {
            StorageFailure::AccessDenied
        }
        zbus::fdo::Error::NoReply(_) | zbus::fdo::Error::Disconnected(_) => {
            StorageFailure::TemporarilyUnavailable
        }
        _ => StorageFailure::Other,
    }
}
#[cfg(target_os = "linux")]
fn dbus_error(e: &zbus::Error) -> StorageFailure {
    match e {
        zbus::Error::FDO(e) => fdo_error(e),
        zbus::Error::Address(_) => StorageFailure::Unavailable,
        zbus::Error::InputOutput(e) | zbus::Error::Connection(e, _) => match e.kind() {
            std::io::ErrorKind::NotFound | std::io::ErrorKind::ConnectionRefused => {
                StorageFailure::Unavailable
            }
            std::io::ErrorKind::PermissionDenied => StorageFailure::AccessDenied,
            std::io::ErrorKind::TimedOut
            | std::io::ErrorKind::WouldBlock
            | std::io::ErrorKind::Interrupted
            | std::io::ErrorKind::NotConnected
            | std::io::ErrorKind::ConnectionAborted
            | std::io::ErrorKind::ConnectionReset
            | std::io::ErrorKind::BrokenPipe => StorageFailure::TemporarilyUnavailable,
            _ => StorageFailure::Other,
        },
        zbus::Error::MethodError(name, _, _) => match name.as_str() {
            "org.freedesktop.DBus.Error.ServiceUnknown"
            | "org.freedesktop.DBus.Error.NameHasNoOwner" => StorageFailure::Unavailable,
            "org.freedesktop.DBus.Error.AccessDenied" | "org.freedesktop.DBus.Error.AuthFailed" => {
                StorageFailure::AccessDenied
            }
            _ => StorageFailure::Other,
        },
        _ => StorageFailure::Other,
    }
}

// Apple's SecBase.h numerical meanings; interaction denial is deliberately not
// inferred to mean Locked. Only typed OSStatus is inspected, never error text.
#[cfg(any(target_os = "macos", test))]
fn mac_status(code: i32) -> StorageFailure {
    match code {
        -128 => StorageFailure::Cancelled,
        -25291 | -25294 | -25307 => StorageFailure::Unavailable,
        -61 | -25292 | -25293 => StorageFailure::AccessDenied,
        -25295 | -26275 => StorageFailure::Corrupt,
        -25299 => StorageFailure::Conflict,
        -25300 => StorageFailure::Missing,
        _ => StorageFailure::Other,
    }
}
#[cfg(test)]
mod error_tests {
    use super::*;
    #[cfg(target_os = "linux")]
    #[test]
    fn dbus_connection_failures_use_typed_io_categories_without_exposing_addresses() {
        use std::io::ErrorKind;
        for (kind, expected) in [
            (ErrorKind::NotFound, StorageFailure::Unavailable),
            (ErrorKind::ConnectionRefused, StorageFailure::Unavailable),
            (ErrorKind::PermissionDenied, StorageFailure::AccessDenied),
            (ErrorKind::TimedOut, StorageFailure::TemporarilyUnavailable),
            (ErrorKind::Other, StorageFailure::Other),
            (ErrorKind::InvalidData, StorageFailure::Other),
            (ErrorKind::Unsupported, StorageFailure::Other),
        ] {
            assert_eq!(
                dbus_error(&zbus::Error::InputOutput(Arc::new(std::io::Error::from(
                    kind
                )))),
                expected
            );
            let source = zbus::Error::Connection(
                Arc::new(std::io::Error::from(kind)),
                "unix:path=/run/identity-fixture-bus".parse().unwrap(),
            );
            assert_eq!(dbus_error(&source), expected);
            assert_eq!(
                native_error(
                    StorageBackend::LinuxSecretService,
                    keyring_core::Error::PlatformFailure(Box::new(secret_service::Error::Zbus(
                        source
                    )))
                ),
                error::storage(StorageBackend::LinuxSecretService, expected)
            );
        }
    }
    #[test]
    fn platform_categories_do_not_infer_a_locked_store_or_expose_raw_errors() {
        assert_eq!(mac_status(-25308), StorageFailure::Other);
        assert_eq!(mac_status(-25307), StorageFailure::Unavailable);
        assert_eq!(mac_status(-25300), StorageFailure::Missing);
        assert_eq!(mac_status(-25293), StorageFailure::AccessDenied);
        assert_eq!(mac_status(-128), StorageFailure::Cancelled);
        let error = native_error(
            StorageBackend::MacKeychain,
            keyring_core::Error::BadEncoding(vec![1, 2, 3]),
        );
        assert_eq!(
            error,
            error::storage(StorageBackend::MacKeychain, StorageFailure::Corrupt)
        );
    }
}
