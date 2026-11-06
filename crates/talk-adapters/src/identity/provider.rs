//! Synchronous OS/file operations. Called only by the two owned provider workers.
use super::{
    error::{self, IdentityError, Result},
    private_fs::PrivateDir,
};
#[cfg(any(target_os = "macos", target_os = "windows"))]
use keyring::native_error;
use std::sync::Arc;
use talk_core::identity::{QualifiedItem, StorageBackend, StorageFailure};
#[cfg(any(target_os = "macos", target_os = "windows"))]
use zeroize::Zeroize;
use zeroize::Zeroizing;

/// Fault barriers exist only in unit-test builds, never backend-test/production.
#[cfg(test)]
pub(super) mod faults;
#[cfg(any(target_os = "macos", target_os = "windows", test))]
mod keyring;
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
    pub fn read(&self, item: QualifiedItem) -> Result<Zeroizing<String>> {
        #[cfg(test)]
        let _fault = self.faults.before_read();
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
                    keyring::entry(item)?
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
                let entry = match keyring::entry(item) {
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
            match keyring::entry(item)?.delete_credential() {
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
