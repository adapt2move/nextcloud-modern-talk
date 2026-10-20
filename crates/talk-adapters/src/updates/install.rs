//! Shipping install detection and the released updater's offline bundle transaction.
use super::{
    ArtifactVerifier, ReleaseInfo, UpdateChannel, UpdateError, archive, native,
    store::{self, Store},
    transport::Selection,
};
use self_update::{
    Release, ReleaseAsset, ReleaseSource,
    http_client::{HeaderMap, HttpClient, HttpResponse},
};
use semver::Version;
use std::{
    fs::File,
    io::Read,
    path::{Path, PathBuf},
    sync::Arc,
    time::Duration,
};
use talk_core::lifecycle::Cancellation;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum InstallKind {
    Portable,
    Debian,
    MacBundle,
}
impl InstallKind {
    pub fn target(self) -> &'static str {
        match self {
            Self::Portable | Self::Debian => "x86_64-unknown-linux-gnu",
            Self::MacBundle => "aarch64-apple-darwin",
        }
    }
    pub fn asset_name(self, version: &str) -> Result<String, UpdateError> {
        if super::transport::parsed_tag(version).is_none() {
            return Err(UpdateError::InvalidResponse);
        }
        Ok(match self {
            Self::Portable => format!("nextcloud-modern-talk-{version}-linux-x86_64.tar.gz"),
            Self::Debian => format!("nextcloud-modern-talk-{version}-linux-x86_64.deb"),
            Self::MacBundle => format!("nextcloud-modern-talk-{version}-macos-arm64-unsigned.zip"),
        })
    }
    pub fn archive_root(self, version: &str) -> Result<String, UpdateError> {
        Ok(match self {
            Self::Portable => self
                .asset_name(version)?
                .trim_end_matches(".tar.gz")
                .to_owned(),
            Self::MacBundle => "NextCloud Modern Talk.app".to_owned(),
            Self::Debian => return Err(UpdateError::UnsupportedInstall),
        })
    }
    pub fn resources(self) -> &'static str {
        match self {
            Self::MacBundle => "Contents/Resources/",
            _ => "share/nextcloud-modern-talk/",
        }
    }
    pub fn binary_directory(self) -> &'static str {
        match self {
            Self::MacBundle => "Contents/MacOS/",
            _ => "bin/",
        }
    }
}
#[derive(Clone, Debug, Eq, PartialEq)]
pub(super) struct Target {
    pub kind: InstallKind,
    pub root: PathBuf,
    pub executable: PathBuf,
    pub old_digest: String,
}
impl Target {
    pub fn detect(current: &Version, cancel: &Cancellation) -> Result<Self, UpdateError> {
        let executable = std::env::current_exe().map_err(|_| UpdateError::UnsupportedInstall)?;
        let (kind, root) = detect_location(&executable)?;
        let old_digest = installed_identity(&root, kind, &current.to_string(), cancel)?;
        Ok(Self {
            kind,
            root,
            executable,
            old_digest,
        })
    }
    pub async fn preflight(&self) -> Result<(), UpdateError> {
        match self.kind {
            InstallKind::Debian => native::available().await,
            _ => {
                let parent = self.root.parent().ok_or(UpdateError::UnsupportedInstall)?;
                // Probe actual create permission on the same filesystem used for the
                // standard updater's staging/stash; never escalate or rewrite /usr.
                let probe = tempfile::Builder::new()
                    .prefix(".nmt-update-probe-")
                    .tempdir_in(parent)
                    .map_err(|_| UpdateError::UnsupportedInstall)?;
                probe.close().map_err(|_| UpdateError::Storage)
            }
        }
    }
    pub fn previous_usable(&self, current: &Version) -> bool {
        archive::verify_tree(
            &self.root,
            self.kind,
            &current.to_string(),
            &Cancellation::default(),
        )
        .is_ok_and(|digest| digest == self.old_digest)
    }
    fn revalidate(&self, current: &Version, cancel: &Cancellation) -> Result<(), UpdateError> {
        let (kind, root) = detect_location(&self.executable)?;
        if kind != self.kind
            || root != self.root
            || installed_identity(&self.root, self.kind, &current.to_string(), cancel)?
                != self.old_digest
        {
            return Err(UpdateError::Stale);
        }
        Ok(())
    }
}
/// Only an actual package layout with matching provenance qualifies. User-writable
/// portable packages must live below the home directory; system Debian ownership
/// is kept on its exact /usr/lib route. Source builds and translocated apps fail.
fn detect_location(executable: &Path) -> Result<(InstallKind, PathBuf), UpdateError> {
    if !executable.is_absolute()
        || executable.file_name().and_then(|v| v.to_str()) != Some("talk-desktop")
        || executable
            .canonicalize()
            .map_err(|_| UpdateError::UnsupportedInstall)?
            != executable
    {
        return Err(UpdateError::UnsupportedInstall);
    }
    let home = dirs::home_dir().ok_or(UpdateError::UnsupportedInstall)?;
    #[cfg(all(target_os = "linux", target_arch = "x86_64"))]
    {
        let deb = Path::new("/usr/lib/nextcloud-modern-talk");
        if executable == deb.join("bin/talk-desktop") {
            return Ok((InstallKind::Debian, deb.to_owned()));
        }
        let bin = executable.parent().ok_or(UpdateError::UnsupportedInstall)?;
        let root = bin.parent().ok_or(UpdateError::UnsupportedInstall)?;
        if bin.file_name().and_then(|p| p.to_str()) != Some("bin")
            || root == home
            || !root.starts_with(&home)
        {
            return Err(UpdateError::UnsupportedInstall);
        }
        user_owned_root(root)?;
        Ok((InstallKind::Portable, root.to_owned()))
    }
    #[cfg(all(target_os = "macos", target_arch = "aarch64"))]
    {
        let macos = executable.parent().ok_or(UpdateError::UnsupportedInstall)?;
        let contents = macos.parent().ok_or(UpdateError::UnsupportedInstall)?;
        let root = contents.parent().ok_or(UpdateError::UnsupportedInstall)?;
        if macos.file_name().and_then(|p| p.to_str()) != Some("MacOS")
            || contents.file_name().and_then(|p| p.to_str()) != Some("Contents")
            || root.extension().and_then(|p| p.to_str()) != Some("app")
            || !(root.starts_with(&home) || root.parent() == Some(Path::new("/Applications")))
            || root
                .components()
                .any(|p| p.as_os_str() == "AppTranslocation")
        {
            return Err(UpdateError::UnsupportedInstall);
        }
        user_owned_root(root)?;
        Ok((InstallKind::MacBundle, root.to_owned()))
    }
    #[cfg(not(any(
        all(target_os = "linux", target_arch = "x86_64"),
        all(target_os = "macos", target_arch = "aarch64")
    )))]
    {
        let _ = home;
        Err(UpdateError::UnsupportedInstall)
    }
}
#[cfg(any(
    all(target_os = "linux", target_arch = "x86_64"),
    all(target_os = "macos", target_arch = "aarch64")
))]
fn user_owned_root(root: &Path) -> Result<(), UpdateError> {
    let metadata = std::fs::symlink_metadata(root).map_err(|_| UpdateError::UnsupportedInstall)?;
    if !metadata.is_dir() || metadata.file_type().is_symlink() {
        return Err(UpdateError::UnsupportedInstall);
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        if metadata.uid() != rustix::process::geteuid().as_raw() || metadata.mode() & 0o022 != 0 {
            return Err(UpdateError::UnsupportedInstall);
        }
    }
    Ok(())
}
fn installed_identity(
    root: &Path,
    kind: InstallKind,
    version: &str,
    cancel: &Cancellation,
) -> Result<String, UpdateError> {
    archive::verify_tree(root, kind, version, cancel)
}
pub(super) struct Staged {
    pub generation: u64,
    pub selection: Selection,
    pub target: Target,
}
impl Staged {
    pub async fn verify(
        &self,
        store: &Store,
        verifier: Option<&dyn ArtifactVerifier>,
        cancel: &Cancellation,
    ) -> Result<Option<String>, UpdateError> {
        store.check()?;
        let path = store.payload(self.target.kind);
        let metadata = store::open_regular(&path)?
            .metadata()
            .map_err(|_| UpdateError::Storage)?;
        if metadata.len() != self.selection.info.bytes
            || archive::sha256(&path, self.selection.info.bytes, cancel)? != self.selection.sha256
        {
            return Err(UpdateError::Integrity);
        }
        if let Some(verifier) = verifier {
            verifier.verify(&path, &self.selection.info)?;
        }
        match self.target.kind {
            InstallKind::Debian => {
                native::verify_package(&path, self.selection.info.version.as_str(), cancel).await?;
                Ok(None)
            }
            kind => archive::inspect(&path, &self.selection, kind, cancel).map(Some),
        }
    }
}
pub(super) async fn apply(
    store: &Store,
    stage: &Staged,
    current: &Version,
    channel: UpdateChannel,
    verifier: Option<&dyn ArtifactVerifier>,
    cancel: &Cancellation,
) -> Result<ReleaseInfo, UpdateError> {
    let next = Version::parse(stage.selection.info.version.as_str())
        .map_err(|_| UpdateError::InvalidResponse)?;
    if !next.cmp_precedence(current).is_gt()
        || (channel == UpdateChannel::Stable
            && (stage.selection.info.prerelease || !next.pre.is_empty()))
    {
        return Err(UpdateError::Stale);
    }
    stage.target.revalidate(current, cancel)?;
    stage.target.preflight().await?;
    let verified_binary = stage.verify(store, verifier, cancel).await?;
    if cancel.is_cancelled() {
        return Err(UpdateError::Cancelled);
    }
    match stage.target.kind {
        InstallKind::Debian => {
            // The package manager owns native dependency/ownership/authentication
            // semantics. No app process overwrites /usr or invokes a shell/sudo.
            native::install(&store.payload(stage.target.kind)).await?;
            installed_identity(
                &stage.target.root,
                stage.target.kind,
                stage.selection.info.version.as_str(),
                cancel,
            )
            .map_err(|_| UpdateError::InstallUncertain)?;
        }
        _ => install_bundle(
            &store.payload(stage.target.kind),
            stage,
            current,
            verified_binary.ok_or(UpdateError::Integrity)?,
            cancel.clone(),
        )?,
    }
    // Committed installation must not be reported as failed solely because a
    // stale private download could not be removed. Startup retries that cleanup.
    let _ = store.remove_payload();
    Ok(stage.selection.info.clone())
}
fn install_bundle(
    path: &Path,
    stage: &Staged,
    current: &Version,
    binary_digest: String,
    cancel: Cancellation,
) -> Result<(), UpdateError> {
    let selection = &stage.selection;
    let release = Release::builder()
        .version(selection.info.version.as_str())
        .asset(
            ReleaseAsset::new(&selection.name, OFFLINE_URL)
                .with_digest(format!("sha256:{}", selection.sha256)),
        )
        .build()
        .map_err(|_| UpdateError::InvalidResponse)?;
    let client = OfflineClient {
        path: path.to_owned(),
        size: selection.info.bytes,
    };
    let kind = stage.target.kind;
    let version = selection.info.version.as_str().to_owned();
    let verify_cancel = cancel.clone();
    let updater = self_update::backends::custom::Update::configure()
        .source(OneRelease(release))
        .http_client(Arc::new(client))
        .bin_name("talk-desktop")
        .current_version(current.to_string())
        .target(kind.target())
        .asset_matcher(|assets| assets.first().cloned())
        .bundle_path_in_archive(kind.archive_root(&version)?)
        .bundle_install_path(&stage.target.root)
        .verify_checksum(self_update::Checksum::Sha256(selection.sha256.clone()))
        .verify_binary(move |root| {
            if archive::verify_tree(root, kind, &version, &verify_cancel)
                .is_ok_and(|hash| hash == binary_digest)
            {
                Ok(())
            } else {
                Err(self_update::Error::verification_rejected(
                    "package identity or extraction changed",
                ))
            }
        })
        .check_install_path_writable(true)
        .show_output(false)
        .no_confirm(true)
        .retries(0)
        .build()
        .map_err(|_| UpdateError::InstallFailed)?;
    let result = updater.update();
    match result {
        Ok(status) if status.is_updated() => Ok(()),
        _ => {
            // The standard swap attempts rollback; never misreport success if the
            // filesystem prevented that rollback. The original app remains running.
            let preserved = stage.target.previous_usable(current);
            Err(if preserved {
                UpdateError::InstallFailed
            } else {
                UpdateError::InstallUncertain
            })
        }
    }
}
const OFFLINE_URL: &str = "https://staged.invalid/exact-release";
struct OneRelease(Release);
impl ReleaseSource for OneRelease {
    fn get_releases(&self) -> self_update::Result<Vec<Release>> {
        Ok(vec![self.0.clone()])
    }
}
struct OfflineClient {
    path: PathBuf,
    size: u64,
}
impl HttpClient for OfflineClient {
    fn get(
        &self,
        url: &str,
        _: &HeaderMap,
        _: Option<Duration>,
    ) -> self_update::Result<Box<dyn HttpResponse>> {
        if url != OFFLINE_URL {
            return Err(self_update::Error::verification_rejected(
                "offline update source mismatch",
            ));
        }
        let file = store::open_regular(&self.path)
            .map_err(|_| self_update::Error::verification_rejected("staged file unavailable"))?;
        if file.metadata()?.len() != self.size {
            return Err(self_update::Error::verification_rejected(
                "staged size changed",
            ));
        }
        Ok(Box::new(OfflineResponse {
            file,
            headers: HeaderMap::new(),
            size: self.size,
        }))
    }
}
struct OfflineResponse {
    file: File,
    headers: HeaderMap,
    size: u64,
}
impl HttpResponse for OfflineResponse {
    fn headers(&self) -> &HeaderMap {
        &self.headers
    }
    fn body(self: Box<Self>) -> Box<dyn Read> {
        Box::new(self.file.take(self.size.saturating_add(1)))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::updates::tests::{write_archive, write_bundle};
    use std::fs;

    fn stage_fixture(temporary: &Path) -> (PathBuf, Staged, Version) {
        let kind = InstallKind::Portable;
        let root = temporary.join("installed");
        write_bundle(&root, kind, "0.0.1", b"OLD-SYNTHETIC");
        let current = Version::parse("0.0.1").unwrap();
        let old_digest =
            installed_identity(&root, kind, "0.0.1", &Cancellation::default()).unwrap();
        let path = temporary.join("release.tar.gz");
        let selection = write_archive(&path, kind, "0.0.2", b"NEW-SYNTHETIC");
        let target = Target {
            kind,
            executable: root.join("bin/talk-desktop"),
            root,
            old_digest,
        };
        (
            path,
            Staged {
                generation: 1,
                selection,
                target,
            },
            current,
        )
    }
    #[test]
    fn offline_standard_bundle_swap_preserves_resources_and_rejects_bad_identity() {
        let temporary = tempfile::tempdir().unwrap();
        let (path, stage, current) = stage_fixture(temporary.path());
        let before = fs::read(stage.target.executable.clone()).unwrap();
        let new_hash = archive::inspect(
            &path,
            &stage.selection,
            stage.target.kind,
            &Cancellation::default(),
        )
        .unwrap();
        assert_eq!(
            install_bundle(
                &path,
                &stage,
                &current,
                "0".repeat(64),
                Cancellation::default()
            ),
            Err(UpdateError::InstallFailed)
        );
        assert_eq!(fs::read(&stage.target.executable).unwrap(), before);
        assert!(stage.target.previous_usable(&current));
        install_bundle(
            &path,
            &stage,
            &current,
            new_hash.clone(),
            Cancellation::default(),
        )
        .unwrap();
        assert_eq!(
            archive::verify_tree(
                &stage.target.root,
                stage.target.kind,
                "0.0.2",
                &Cancellation::default()
            )
            .unwrap(),
            new_hash
        );
        assert!(
            fs::read(&stage.target.executable)
                .unwrap()
                .ends_with(b"NEW-SYNTHETIC")
        );
        assert!(!stage.target.previous_usable(&current));
        assert_eq!(
            stage.target.revalidate(&current, &Cancellation::default()),
            Err(UpdateError::UnsupportedInstall)
        );
    }
    #[test]
    fn standard_updater_rolls_back_a_failure_after_the_old_bundle_is_stashed() {
        let temporary = tempfile::tempdir().unwrap();
        let (path, stage, current) = stage_fixture(temporary.path());
        let before = fs::read(&stage.target.executable).unwrap();
        let release = Release::builder()
            .version("0.0.2")
            .asset(ReleaseAsset::new(&stage.selection.name, OFFLINE_URL))
            .build()
            .unwrap();
        let updater = self_update::backends::custom::Update::configure()
            .source(OneRelease(release))
            .http_client(Arc::new(OfflineClient {
                path,
                size: stage.selection.info.bytes,
            }))
            .bin_name("talk-desktop")
            .current_version(current.to_string())
            .asset_matcher(|assets| assets.first().cloned())
            .bundle_path_in_archive(stage.target.kind.archive_root("0.0.2").unwrap())
            .bundle_install_path(&stage.target.root)
            .verify_checksum(self_update::Checksum::Sha256(
                stage.selection.sha256.clone(),
            ))
            // Safe test-only fault: remove the private extracted tree after verify
            // but before rename. The old tree is then stashed and must roll back.
            .verify_binary(|new_root| {
                fs::remove_dir_all(new_root)?;
                Ok(())
            })
            .show_output(false)
            .no_confirm(true)
            .build()
            .unwrap();
        assert!(updater.update().is_err());
        assert_eq!(fs::read(&stage.target.executable).unwrap(), before);
        assert!(stage.target.previous_usable(&current));
    }
    #[test]
    fn offline_transport_refuses_any_unselected_request() {
        let temporary = tempfile::tempdir().unwrap();
        let (path, stage, _) = stage_fixture(temporary.path());
        let client = OfflineClient {
            path,
            size: stage.selection.info.bytes,
        };
        assert!(
            client
                .get("https://github.com/unselected", &HeaderMap::new(), None)
                .is_err()
        );
        assert!(client.get(OFFLINE_URL, &HeaderMap::new(), None).is_ok());
    }
    #[test]
    fn source_builds_deleted_paths_and_noncanonical_locations_are_not_install_targets() {
        assert!(detect_location(Path::new("/does-not-exist/target/debug/talk-desktop")).is_err());
        assert!(detect_location(Path::new("relative/bin/talk-desktop")).is_err());
        assert!(detect_location(Path::new("/tmp/stash/talk-desktop (deleted)")).is_err());
        assert!(
            Target::detect(&Version::parse("0.0.1").unwrap(), &Cancellation::default()).is_err()
        ); // cargo test executable
    }
    #[test]
    #[cfg(all(target_os = "linux", target_arch = "x86_64"))]
    fn owned_home_bundle_is_supported_but_links_and_shared_write_access_are_not() {
        use std::os::unix::fs::{PermissionsExt, symlink};
        let home = dirs::home_dir().unwrap();
        let temporary = tempfile::Builder::new()
            .prefix(".nmt-install-location-fixture-")
            .tempdir_in(home)
            .unwrap();
        let root = temporary.path().join("installed");
        write_bundle(&root, InstallKind::Portable, "0.0.1", b"NOT-RUN");
        let executable = root.join("bin/talk-desktop");
        assert_eq!(
            detect_location(&executable).unwrap(),
            (InstallKind::Portable, root.clone())
        );
        let link = temporary.path().join("linked");
        symlink(&root, &link).unwrap();
        assert_eq!(
            detect_location(&link.join("bin/talk-desktop")),
            Err(UpdateError::UnsupportedInstall)
        );
        fs::set_permissions(&root, fs::Permissions::from_mode(0o777)).unwrap();
        assert_eq!(
            detect_location(&executable),
            Err(UpdateError::UnsupportedInstall)
        );
        fs::set_permissions(&root, fs::Permissions::from_mode(0o755)).unwrap();
        fs::remove_file(&executable).unwrap();
        assert_eq!(
            detect_location(&executable),
            Err(UpdateError::UnsupportedInstall)
        );
    }
}
