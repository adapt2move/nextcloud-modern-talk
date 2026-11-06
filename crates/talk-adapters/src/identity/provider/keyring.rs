//! Native macOS/Windows entries and content-free error categories.
use super::{IdentityError, StorageBackend, StorageFailure, error};
#[cfg(any(target_os = "macos", target_os = "windows"))]
use super::{QualifiedItem, Result, key, native_backend};
#[cfg(any(target_os = "macos", target_os = "windows"))]
use keyring_core::{Entry, api::CredentialStoreApi};
#[cfg(any(target_os = "macos", target_os = "windows"))]
use std::collections::HashMap;
use zeroize::Zeroize;

#[cfg(any(target_os = "macos", target_os = "windows"))]
pub(super) fn entry(item: QualifiedItem) -> Result<Entry> {
    let backend = item.provider.backend;
    if backend != native_backend() {
        return Err(error::storage(backend, StorageFailure::Unsupported));
    }
    let service = "org.adapt2move.nextcloud-modern-talk";
    let user = key(item);
    #[cfg(target_os = "macos")]
    {
        let config = HashMap::from([("keychain", "User")]);
        let store = apple_native_keyring_store::keychain::Store::new_with_configuration(&config)
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

pub(super) fn native_error(backend: StorageBackend, mut e: keyring_core::Error) -> IdentityError {
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
    #[cfg(any(target_os = "macos", target_os = "windows"))]
    let mut category = category;
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
mod tests {
    use super::*;
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
