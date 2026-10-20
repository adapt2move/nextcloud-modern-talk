//! Allowlisted errors only. Never format a native, SQLite, HTTP or decoder error.
use talk_core::identity::{StorageBackend, StorageFailure};
use talk_core::ports::ErrorKind;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum IdentityError {
    Request(ErrorKind),
    Storage {
        backend: StorageBackend,
        failure: StorageFailure,
    },
    RecoveryRequired,
    SuppressionUnconfirmed,
    WorkerUnavailable,
}
impl From<ErrorKind> for IdentityError {
    fn from(value: ErrorKind) -> Self {
        Self::Request(value)
    }
}
pub(super) type Result<T> = std::result::Result<T, IdentityError>;
pub(super) fn storage(backend: StorageBackend, failure: StorageFailure) -> IdentityError {
    IdentityError::Storage { backend, failure }
}
pub(super) fn io(error: std::io::Error) -> IdentityError {
    use std::io::ErrorKind as K;
    let failure = match error.kind() {
        K::NotFound => StorageFailure::Missing,
        K::PermissionDenied => StorageFailure::AccessDenied,
        K::AlreadyExists => StorageFailure::Conflict,
        K::WouldBlock => StorageFailure::TemporarilyUnavailable,
        K::InvalidData => StorageFailure::Corrupt,
        K::Unsupported => StorageFailure::Unsupported,
        _ => StorageFailure::Other,
    };
    storage(StorageBackend::PrivateFile, failure)
}
pub(super) fn denied() -> IdentityError {
    storage(StorageBackend::PrivateFile, StorageFailure::AccessDenied)
}
pub(super) fn corrupt() -> IdentityError {
    storage(StorageBackend::PrivateFile, StorageFailure::Corrupt)
}
