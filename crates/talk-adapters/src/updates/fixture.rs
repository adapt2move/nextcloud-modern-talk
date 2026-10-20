//! Explicit backend-test-only running-image proof. Never built into shipping packages.
use super::{
    UpdateError, archive,
    install::{InstallKind, Staged, Target},
    store::Store,
    transport::{MAX_DOWNLOAD, Selection},
};
use std::{
    fs::File,
    io::{Read, Write},
    path::{Path, PathBuf},
};
use talk_core::{
    bounded::Text,
    lifecycle::Cancellation,
    preferences::{UpdateChannel, updates::ReleaseInfo},
};

/// Install only the labelled private fixture built by tools/updates/install_probe.py.
/// The running probe must itself be inside that owned fixture's bin directory.
/// This follows the same digest/archive/identity/apply path as the application,
/// with fixture input replacing the production-only public release selection.
pub fn install(archive_path: &Path, expected_digest: &str) -> Result<PathBuf, UpdateError> {
    let current =
        semver::Version::parse("0.0.0-fixture.1").map_err(|_| UpdateError::InvalidResponse)?;
    let cancel = Cancellation::default();
    let target = Target::detect(&current, &cancel)?;
    if target.kind != InstallKind::Portable {
        return Err(UpdateError::UnsupportedInstall);
    }
    let base = target
        .root
        .parent()
        .ok_or(UpdateError::UnsupportedInstall)?;
    let marker = base.join("synthetic-update-probe");
    let mut marker_bytes = Vec::new();
    File::open(&marker)
        .map_err(|_| UpdateError::UnsupportedInstall)?
        .take(64)
        .read_to_end(&mut marker_bytes)
        .map_err(|_| UpdateError::Storage)?;
    if marker_bytes != b"NMT PRIVATE UPDATE FIXTURE ONLY\n"
        || archive_path.parent() != Some(base)
        || !super::transport::valid_digest(expected_digest)
    {
        return Err(UpdateError::UnsupportedInstall);
    }
    let store = Store::at(&base.join("private-stage"))?;
    let mut file = store.create_payload(target.kind)?;
    let copied = std::io::copy(
        &mut File::open(archive_path)
            .map_err(|_| UpdateError::Storage)?
            .take(MAX_DOWNLOAD + 1),
        &mut file,
    )
    .map_err(|_| UpdateError::Storage)?;
    if copied > MAX_DOWNLOAD {
        return Err(UpdateError::TooLarge);
    }
    file.flush().map_err(|_| UpdateError::Storage)?;
    file.sync_all().map_err(|_| UpdateError::Storage)?;
    let stage = Staged {
        generation: 1,
        selection: Selection {
            info: ReleaseInfo {
                version: Text::new("0.0.0-fixture.2").map_err(|_| UpdateError::InvalidResponse)?,
                prerelease: true,
                bytes: copied,
            },
            name: target.kind.asset_name("0.0.0-fixture.2")?,
            url: "synthetic offline fixture".into(),
            sha256: expected_digest.into(),
        },
        target,
    };
    let original = stage.target.executable.clone();
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .map_err(|_| UpdateError::Unavailable)?;
    runtime.block_on(super::install::apply(
        &store,
        &stage,
        &current,
        UpdateChannel::Prerelease,
        None,
        &cancel,
    ))?;
    if archive::sha256(&original, archive::MAX_EXPANDED, &cancel)? == stage.target.old_digest {
        return Err(UpdateError::InstallFailed);
    }
    Ok(original)
}
