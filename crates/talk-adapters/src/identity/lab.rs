//! Default-off disposable-lab intake. Never called by ordinary sign-in.
use super::{error, ledger, login::Origin, private_fs::PrivateDir, provider};
use std::{path::Path, sync::Arc};
use talk_core::{identity::StorageBackend, ids::LoginAttemptId, ports::ErrorKind};

/// Installs one synthetic loopback account through the ordinary durable provider
/// transaction. No production origin/user is accepted and no secret is returned.
pub fn seed_private_lab(profile: &Path, fixture: &Path) -> error::Result<()> {
    let bytes = crate::nextcloud::test_support::private_file(fixture)
        .map_err(error::IdentityError::Request)?;
    let mut value: serde_json::Value =
        serde_json::from_slice(&bytes).map_err(|_| ErrorKind::Malformed)?;
    let origin = Origin::parse(value["origin"].as_str().ok_or(ErrorKind::Malformed)?, true)?;
    let user = value["user"]
        .as_str()
        .filter(|v| v.starts_with("lab_"))
        .ok_or(ErrorKind::Malformed)?
        .to_owned();
    let password = match value.get_mut("password").map(serde_json::Value::take) {
        Some(serde_json::Value::String(v)) => zeroize::Zeroizing::new(v),
        _ => return Err(ErrorKind::Malformed.into()),
    };
    let dir = Arc::new(PrivateDir::open(profile, true)?);
    let mut ledger = ledger::Ledger::open(dir.clone())?;
    if !ledger.accounts()?.is_empty() {
        return Err(ErrorKind::Conflict.into());
    }
    let account = ledger.reserve(origin.text)?.id;
    ledger.bind_user(account, &user)?;
    let operation = ledger.stage(
        account,
        LoginAttemptId::from_local(ledger::random()?),
        StorageBackend::PrivateFile,
        &user,
    )?;
    let providers = provider::Providers::new(dir, ledger.lock.clone());
    if !matches!(
        providers.write(operation.item, &password),
        provider::WriteResult::Present
    ) {
        return Err(ErrorKind::Transport.into());
    }
    ledger.settle_result(operation, true, None)?;
    let revision = ledger.account(account)?.revision;
    ledger.activate(operation, revision)
}
