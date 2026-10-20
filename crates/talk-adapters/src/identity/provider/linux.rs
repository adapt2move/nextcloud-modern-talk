//! Maintained native operations with no Unlock or Prompt dispatch, even after a race.
use super::*;
use dbus_secret_service::{Collection, EncryptionType, Item, SecretService};
use std::collections::HashMap;

const SERVICE: &str = "org.adapt2move.nextcloud-modern-talk";
fn failure(kind: StorageFailure) -> IdentityError {
    error::storage(StorageBackend::LinuxSecretService, kind)
}
fn mapped(error: dbus_secret_service::Error) -> IdentityError {
    use dbus_secret_service::Error as E;
    failure(match error {
        E::Locked | E::Prompt => StorageFailure::Locked,
        E::Unavailable => StorageFailure::Unavailable,
        E::NoResult => StorageFailure::Missing,
        E::Parse | E::UnsupportedSecretFormat | E::Crypto(_) => StorageFailure::Corrupt,
        // These are fixed protocol error identifiers, never message/address text.
        E::Dbus(e) => dbus_name(e.name()),
        _ => StorageFailure::Other,
    })
}
fn dbus_name(name: Option<&str>) -> StorageFailure {
    match name {
        Some("org.freedesktop.Secret.Error.IsLocked") => StorageFailure::Locked,
        Some(
            "org.freedesktop.DBus.Error.AccessDenied" | "org.freedesktop.DBus.Error.AuthFailed",
        ) => StorageFailure::AccessDenied,
        Some(
            "org.freedesktop.DBus.Error.ServiceUnknown"
            | "org.freedesktop.DBus.Error.NameHasNoOwner",
        ) => StorageFailure::Unavailable,
        Some(
            "org.freedesktop.DBus.Error.NoReply"
            | "org.freedesktop.DBus.Error.Disconnected"
            | "org.freedesktop.DBus.Error.Timeout",
        ) => StorageFailure::TemporarilyUnavailable,
        Some("org.freedesktop.DBus.Error.UnknownObject") => StorageFailure::Missing,
        _ => StorageFailure::Other,
    }
}
fn connect(providers: &Providers, item: QualifiedItem) -> Result<SecretService> {
    if item.provider.backend != StorageBackend::LinuxSecretService {
        return Err(error::storage(
            item.provider.backend,
            StorageFailure::Unsupported,
        ));
    }
    // Keep the previously qualified typed connection/absence categories. This
    // check is advisory: all subsequent operations independently forbid prompts.
    let map = |e| {
        native_error(
            StorageBackend::LinuxSecretService,
            keyring_core::Error::PlatformFailure(Box::new(e)),
        )
    };
    let native =
        secret_service::blocking::SecretService::connect(secret_service::EncryptionType::Dh)
            .map_err(map)?;
    let collection = native.get_default_collection().map_err(|e| match e {
        secret_service::Error::NoResult => failure(StorageFailure::Unavailable),
        other => map(other),
    })?;
    if collection.is_locked().map_err(map)? {
        return Err(failure(StorageFailure::Locked));
    }
    #[cfg(test)]
    let _barrier = providers.faults.after_preflight();
    #[cfg(not(test))]
    let _ = providers;
    // Zero means the maintained crate returns BEFORE calling Prompt. Do not use
    // ensure_unlocked/unlock/unlock_all, even with this setting.
    SecretService::connect_with_max_prompt_timeout(EncryptionType::Dh, 0).map_err(mapped)
}
fn collection(service: &SecretService) -> Result<Collection<'_>> {
    let collection = service.get_default_collection().map_err(|e| match e {
        dbus_secret_service::Error::NoResult => failure(StorageFailure::Unavailable),
        other => mapped(other),
    })?;
    if collection.is_locked().map_err(mapped)? {
        return Err(failure(StorageFailure::Locked));
    }
    Ok(collection)
}
fn attributes(user: &str) -> HashMap<&str, &str> {
    // Exact r3 attributes: old native P can be read without migration/enumeration.
    HashMap::from([
        ("service", SERVICE),
        ("username", user),
        ("target", "default"),
    ])
}
fn find<'a>(collection: &'a Collection<'_>, user: &str) -> Result<Option<Item<'a>>> {
    let mut found = collection.search_items(attributes(user)).map_err(mapped)?;
    if found.len() > 1 {
        return Err(failure(StorageFailure::Conflict));
    }
    let item = found.pop();
    if let Some(item) = &item
        && item.is_locked().map_err(mapped)?
    {
        return Err(failure(StorageFailure::Locked));
    }
    Ok(item)
}
pub(super) fn read(providers: &Providers, item: QualifiedItem) -> Result<Zeroizing<String>> {
    let service = connect(providers, item)?;
    let collection = collection(&service)?;
    let found = find(&collection, &key(item))?.ok_or_else(|| failure(StorageFailure::Missing))?;
    let bytes = Zeroizing::new(found.get_secret().map_err(mapped)?);
    if bytes.len() > RECORD_LIMIT {
        return Err(failure(StorageFailure::Corrupt));
    }
    let text = std::str::from_utf8(&bytes).map_err(|_| failure(StorageFailure::Corrupt))?;
    Ok(Zeroizing::new(text.to_owned()))
}
pub(super) fn create(providers: &Providers, item: QualifiedItem, value: &str) -> WriteResult {
    let prepared = (|| {
        let service = connect(providers, item)?;
        {
            let collection = collection(&service)?;
            if find(&collection, &key(item))?.is_some() {
                return Err(failure(StorageFailure::Conflict));
            }
        }
        Ok(service)
    })();
    let service = match prepared {
        Ok(s) => s,
        Err(e) => return WriteResult::NotWritten(e),
    };
    let collection = match collection(&service) {
        Ok(c) => c,
        Err(e) => return WriteResult::NotWritten(e),
    };
    // replace=false: a raced duplicate never overwrites P. The caller requires
    // exact unique fresh read-back before recording settlement/eligibility.
    match collection.create_item(
        "Nextcloud Modern Talk app credential",
        attributes(&key(item)),
        value.as_bytes(),
        false,
        "application/octet-stream",
    ) {
        Ok(_) => WriteResult::Present, // Setter completion only; caller still verifies.
        Err(e) => WriteResult::Unknown(mapped(e)),
    }
}
pub(super) fn delete(providers: &Providers, item: QualifiedItem) -> Result<()> {
    let service = connect(providers, item)?;
    let collection = collection(&service)?;
    if let Some(found) = find(&collection, &key(item))? {
        // The crate's delete operation never unlocks; a returned prompt is refused.
        found.delete().map_err(mapped)?;
    }
    Ok(()) // Caller independently verifies exact absence before acknowledging cleanup.
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn prompt_refusal_and_protocol_categories_never_infer_unavailability_from_denial() {
        assert_eq!(
            mapped(dbus_secret_service::Error::Prompt),
            failure(StorageFailure::Locked)
        );
        assert_eq!(
            mapped(dbus_secret_service::Error::Path(
                "private-native-value".into()
            )),
            failure(StorageFailure::Other)
        );
        assert_eq!(
            dbus_name(Some("org.freedesktop.DBus.Error.AccessDenied")),
            StorageFailure::AccessDenied
        );
        assert_eq!(
            dbus_name(Some("org.freedesktop.Secret.Error.IsLocked")),
            StorageFailure::Locked
        );
        assert_eq!(
            dbus_name(Some("org.freedesktop.DBus.Error.ServiceUnknown")),
            StorageFailure::Unavailable
        );
        assert_eq!(
            dbus_name(Some("org.freedesktop.DBus.Error.NoReply")),
            StorageFailure::TemporarilyUnavailable
        );
        assert_eq!(dbus_name(None), StorageFailure::Other);
        assert_eq!(
            dbus_name(Some("org.example.Unknown.AccessDenied")),
            StorageFailure::Other
        );
    }
}
