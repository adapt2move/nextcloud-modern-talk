#[cfg(unix)]
use super::install::{Staged, Target};
use super::*;
use super::{
    install::InstallKind,
    transport::{Selection, select},
};
use sha2::{Digest, Sha256};
use std::{
    collections::{BTreeMap, BTreeSet},
    fs::{self, File},
    io::Write,
    path::{Path, PathBuf},
};

fn release(version: &str, prerelease: bool, kind: InstallKind) -> serde_json::Value {
    let name = kind.asset_name(version).unwrap();
    serde_json::json!({"tag_name":format!("v{version}"),"draft":false,"prerelease":prerelease,"assets":[{
        "name":name,"size":100,"digest":format!("sha256:{}", "a".repeat(64)),
        "browser_download_url":format!("https://github.com/adapt2move/nextcloud-modern-talk/releases/download/v{version}/{name}")
    }]})
}
fn selected(
    releases: serde_json::Value,
    current: &str,
    channel: UpdateChannel,
    kind: InstallKind,
) -> Result<Option<Selection>, UpdateError> {
    select(
        &serde_json::to_vec(&releases).unwrap(),
        &Version::parse(current).unwrap(),
        channel,
        kind,
    )
}
#[test]
fn exact_target_channel_precedence_and_no_downgrade() {
    let releases = serde_json::json!([
        release("0.2.0-rc.2", true, InstallKind::Portable), release("0.1.10", false, InstallKind::Portable),
        release("0.1.9", false, InstallKind::Portable), release("0.2.0-alpha.10", false, InstallKind::Portable),
        {"tag_name":"v9.0.0","draft":true,"prerelease":false}
    ]);
    let stable = selected(
        releases.clone(),
        "0.1.9",
        UpdateChannel::Stable,
        InstallKind::Portable,
    )
    .unwrap()
    .unwrap();
    assert_eq!(stable.info.version.as_str(), "0.1.10");
    let pre = selected(
        releases.clone(),
        "0.1.9",
        UpdateChannel::Prerelease,
        InstallKind::Portable,
    )
    .unwrap()
    .unwrap();
    assert_eq!(pre.info.version.as_str(), "0.2.0-rc.2");
    assert!(
        selected(
            releases.clone(),
            "1.0.0",
            UpdateChannel::Prerelease,
            InstallKind::Portable
        )
        .unwrap()
        .is_none()
    );
    assert_eq!(
        selected(
            releases,
            "0.1.9",
            UpdateChannel::Stable,
            InstallKind::MacBundle
        ),
        Err(UpdateError::Unavailable)
    );
    assert!(
        selected(
            serde_json::json!([release("0.0.1", true, InstallKind::Portable)]),
            "0.0.0",
            UpdateChannel::Stable,
            InstallKind::Portable
        )
        .unwrap()
        .is_none()
    );
}
#[test]
fn missing_digest_foreign_url_duplicate_or_unbounded_assets_never_become_ready() {
    let good = release("1.0.0", false, InstallKind::Portable);
    let mut missing = good.clone();
    missing["assets"][0]["digest"] = serde_json::Value::Null;
    let mut bad_url = good.clone();
    bad_url["assets"][0]["browser_download_url"] =
        serde_json::json!("https://elsewhere.invalid/app.tar.gz");
    let mut duplicate = good.clone();
    let asset = duplicate["assets"][0].clone();
    duplicate["assets"].as_array_mut().unwrap().push(asset);
    let mut huge = good.clone();
    huge["assets"][0]["size"] = serde_json::json!(transport::MAX_DOWNLOAD + 1);
    for (fixture, expected) in [
        (missing, UpdateError::Integrity),
        (bad_url, UpdateError::InvalidResponse),
        (duplicate, UpdateError::InvalidResponse),
        (huge, UpdateError::TooLarge),
    ] {
        assert_eq!(
            selected(
                serde_json::json!([fixture]),
                "0.0.1",
                UpdateChannel::Stable,
                InstallKind::Portable
            ),
            Err(expected)
        );
    }
    assert_eq!(
        select(
            &vec![b' '; transport::MAX_BODY + 1],
            &Version::parse("0.0.1").unwrap(),
            UpdateChannel::Stable,
            InstallKind::Portable
        ),
        Err(UpdateError::InvalidResponse)
    );
}
#[test]
fn download_size_accounting_refuses_overflow_and_overrun() {
    assert_eq!(transport::checked_download_size(99, 1, 100), Ok(100));
    assert_eq!(
        transport::checked_download_size(99, 2, 100),
        Err(UpdateError::TooLarge)
    );
    assert_eq!(
        transport::checked_download_size(u64::MAX, 1, u64::MAX),
        Err(UpdateError::TooLarge)
    );
}
fn info() -> ReleaseInfo {
    ReleaseInfo {
        version: Text::new("0.0.2").unwrap(),
        prerelease: false,
        bytes: 100,
    }
}
fn dormant_owner() -> UpdateRuntime {
    let (changed, _) = watch::channel(1);
    UpdateRuntime {
        worker: None,
        shared: Arc::new(Shared {
            state: Mutex::new(State {
                snapshot: UpdateSnapshot {
                    revision: 1,
                    generation: 1,
                    current_version: Text::new("0.0.1").unwrap(),
                    channel: UpdateChannel::Stable,
                    state: UpdateState::Ready(info()),
                    next_check_unix_seconds: Some(1),
                    last_check: None,
                },
                armed: true,
                check_requested: false,
                install_requested: false,
                busy: false,
                reserved: false,
                poisoned: false,
                cancel: Cancellation::default(),
                relaunch: Some(PathBuf::from("/synthetic/installed/bin/talk-desktop")),
                previous_usable: false,
            }),
            wake: Condvar::new(),
            changed,
            stop: AtomicBool::new(false),
            stopped: AtomicBool::new(false),
        }),
    }
}
#[test]
fn synchronous_call_refusal_and_reservation_freeze_the_selection() {
    let owner = dormant_owner();
    let generation = owner.snapshot().generation;
    assert!(matches!(
        owner.reserve_install(generation, || Err(UpdateError::ActiveCall)),
        Err(UpdateError::ActiveCall)
    ));
    assert_eq!(owner.snapshot().state, UpdateState::Ready(info()));
    let reservation = owner.reserve_install(generation, || Ok(())).unwrap();
    assert!(
        owner
            .relaunch_path(&reservation)
            .unwrap()
            .ends_with("bin/talk-desktop")
    );
    assert_eq!(
        owner.set_channel(UpdateChannel::Prerelease),
        Err(UpdateError::Busy)
    );
    assert_eq!(owner.check(UpdateChannel::Stable), Err(UpdateError::Busy));
    assert_eq!(owner.snapshot().generation, generation);
    owner.abort_install(reservation).unwrap();
    assert_eq!(owner.snapshot().state, UpdateState::Ready(info()));
    assert!(owner.relaunch_after_failure().is_none());
    let reservation = owner.reserve_install(generation, || Ok(())).unwrap();
    owner.begin_install(reservation).unwrap();
    assert!(matches!(owner.snapshot().state, UpdateState::Applying(_)));
}
#[test]
fn channel_change_discards_stale_completion_and_manual_checks_are_bounded() {
    let owner = dormant_owner();
    owner.check(UpdateChannel::Stable).unwrap();
    {
        lock(&owner.shared).busy = true;
    }
    assert_eq!(owner.check(UpdateChannel::Stable), Err(UpdateError::Busy));
    let old = owner.snapshot().generation;
    owner.set_channel(UpdateChannel::Prerelease).unwrap();
    publish(&owner.shared, old, UpdateState::Ready(info()));
    assert_eq!(owner.snapshot().state, UpdateState::Idle);
    assert!(matches!(
        owner.reserve_install(old, || Ok(())),
        Err(UpdateError::Stale)
    ));
}
#[test]
fn start_is_offline_unarmed_and_shutdown_interrupts_pending_work() {
    let owner = UpdateRuntime::start("0.0.1").unwrap();
    assert_eq!(owner.snapshot().state, UpdateState::Idle);
    assert_eq!(owner.snapshot().next_check_unix_seconds, None);
    let generation = owner.snapshot().generation;
    owner.request_shutdown();
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap();
    assert_eq!(
        runtime.block_on(interruptible::<()>(
            &owner.shared,
            generation,
            std::future::pending()
        )),
        Err(UpdateError::Stopped)
    );
    drop(owner); // joins without creating staging or contacting any API
}

/// Synthetic bytes have native header identity but are deliberately not runnable.
/// These fixtures are never launched or passed to a system package manager.
const MAC_RUNTIME_FIXTURE: [(&str, &str, u32); 3] = [
    (
        "lib/libgstreamer-1.0.0.dylib",
        "Contents/Frameworks/libgstreamer-1.0.0.dylib",
        6,
    ),
    (
        "lib/gstreamer-1.0/libgstcoreelements.dylib",
        "Contents/PlugIns/libgstcoreelements.dylib",
        6,
    ),
    (
        "libexec/gstreamer-1.0/gst-plugin-scanner",
        "Contents/Helpers/gst-plugin-scanner",
        2,
    ),
];
const MAC_PROVENANCE_FIXTURE: &str = "Contents/Resources/provenance.json";
const MAC_MANIFEST_FIXTURE: &str = "Contents/Resources/assets/gstreamer-runtime.json";
pub(super) const MAC_SIGNATURE_FIXTURE: &[u8] = b"SYNTHETIC-NONNATIVE-CODE-SIGNATURE";

pub(super) fn fixture_files(
    kind: InstallKind,
    version: &str,
    label: &[u8],
) -> BTreeMap<String, (Vec<u8>, u32)> {
    let mut binary = vec![0u8; 64];
    if kind == InstallKind::MacBundle {
        binary[..4].copy_from_slice(b"\xcf\xfa\xed\xfe");
        binary[4..8].copy_from_slice(&0x0100000cu32.to_le_bytes());
        binary[12..16].copy_from_slice(&2u32.to_le_bytes());
    } else {
        binary[..6].copy_from_slice(b"\x7fELF\x02\x01");
        binary[18..20].copy_from_slice(&62u16.to_le_bytes());
    }
    binary.extend_from_slice(label);
    let mut inputs = BTreeMap::from([("bin/talk-desktop".to_owned(), binary)]);
    for name in [
        "licenses/project/LICENSE",
        "licenses/project/NOTICE.md",
        "licenses/desktop/NOTICE.md",
        "source-manifest.json",
        "dependencies.json",
    ] {
        inputs.insert(name.to_owned(), b"SYNTHETIC update fixture only".to_vec());
    }
    // Match the package builder's PAX long-path route used by dependency notices.
    inputs.insert(
        format!(
            "licenses/desktop/{}/LICENSE",
            "synthetic-long-name-".repeat(8)
        ),
        b"SYNTHETIC long notice".to_vec(),
    );
    if kind == InstallKind::MacBundle {
        let mut runtime_files = BTreeMap::new();
        for (suffix, path, file_type) in MAC_RUNTIME_FIXTURE {
            let mut bytes = inputs["bin/talk-desktop"].clone();
            bytes[12..16].copy_from_slice(&file_type.to_le_bytes());
            bytes.extend_from_slice(suffix.as_bytes());
            let digest = super::digest_hex(Sha256::digest(&bytes));
            runtime_files.insert(
                suffix,
                serde_json::json!({
                    "source_sha256":digest,"sha256":digest,
                    "bundle_path":path,"dependencies":[]
                }),
            );
            inputs.insert(format!("runtime/gstreamer/{suffix}"), bytes);
        }
        inputs.insert(
            "assets/gstreamer-runtime.json".into(),
            serde_json::to_vec(&serde_json::json!({
                "schema":1,"version":"1.28.4","files":runtime_files,
                "runtime_archive":{"url":"https://synthetic.invalid/runtime.pkg","sha256":"a".repeat(64)}
            }))
            .unwrap(),
        );
        inputs.insert(
            "assets/macos/AppIcon.icns".into(),
            b"SYNTHETIC icon".to_vec(),
        );
        inputs.insert(
            "licenses/gstreamer/NOTICE.txt".into(),
            b"SYNTHETIC runtime notice".to_vec(),
        );
    }
    let hashes: BTreeMap<_, _> = inputs
        .iter()
        .map(|(name, data)| (name.clone(), super::digest_hex(Sha256::digest(data))))
        .collect();
    let mut provenance = serde_json::json!({"schema":1,"kind":"development","version":version,"target":kind.target(),
        "build":{"profile":"release","package":"talk-desktop","features":[]},"files":hashes});
    if kind == InstallKind::MacBundle {
        provenance["build"]["macos_main_identity"] = archive::MAC_MAIN_IDENTITY.into();
        provenance["build"]["bundled_runtime"] = serde_json::json!({
            "name":"GStreamer","version":"1.28.4","manifest":"assets/gstreamer-runtime.json",
            "manifest_sha256":provenance["files"]["assets/gstreamer-runtime.json"]
        });
    }
    let mut files = BTreeMap::new();
    for (name, mut bytes) in inputs {
        let runtime = name.strip_prefix("runtime/gstreamer/").map(|suffix| {
            MAC_RUNTIME_FIXTURE
                .iter()
                .find(|(fixture, _, _)| *fixture == suffix)
                .unwrap()
                .1
        });
        let executable = name == "bin/talk-desktop" || runtime.is_some();
        if kind == InstallKind::MacBundle && name == "bin/talk-desktop" {
            // Model the signing-induced change while keeping provenance bound
            // to the canonical bytes. This is not a real Mach-O signature.
            bytes.extend_from_slice(MAC_SIGNATURE_FIXTURE);
        }
        let path = if let Some(runtime) = runtime {
            runtime.to_owned()
        } else if executable {
            format!("{}talk-desktop", kind.binary_directory())
        } else if kind == InstallKind::MacBundle && name == "assets/macos/AppIcon.icns" {
            "Contents/Resources/AppIcon.icns".to_owned()
        } else {
            format!("{}{name}", kind.resources())
        };
        files.insert(path, (bytes, if executable { 0o755 } else { 0o644 }));
    }
    files.insert(
        if kind == InstallKind::MacBundle {
            MAC_PROVENANCE_FIXTURE.to_owned()
        } else {
            format!("{}provenance.json", kind.resources())
        },
        (serde_json::to_vec(&provenance).unwrap(), 0o644),
    );
    files.insert(
        format!("{}INSTALL.txt", kind.resources()),
        (b"SYNTHETIC fixture; do not run".to_vec(), 0o644),
    );
    if kind == InstallKind::MacBundle {
        files.insert(
            "Contents/Info.plist".into(),
            (b"SYNTHETIC fixture".to_vec(), 0o644),
        );
        files.insert(
            "Contents/_CodeSignature/CodeResources".into(),
            (
                Sha256::digest(&files[MAC_PROVENANCE_FIXTURE].0).to_vec(),
                0o644,
            ),
        );
    }
    files
}
pub(super) fn verify_fixture_tree(
    root: &Path,
    kind: InstallKind,
    version: &str,
    cancel: &Cancellation,
) -> Result<String, UpdateError> {
    archive::verify_tree_with_mac_identity(root, kind, version, cancel, |root, raw_main, _, _| {
        // An explicit injected native boundary, never selected by production.
        // The resource sentinel models sealed provenance; only macOS CI can
        // establish actual codesign and signature-removal behavior.
        let provenance =
            fs::read(root.join(MAC_PROVENANCE_FIXTURE)).map_err(|_| UpdateError::Storage)?;
        let seal = fs::read(root.join("Contents/_CodeSignature/CodeResources"))
            .map_err(|_| UpdateError::Storage)?;
        let main =
            fs::read(root.join("Contents/MacOS/talk-desktop")).map_err(|_| UpdateError::Storage)?;
        if seal != Sha256::digest(&provenance).to_vec()
            || super::digest_hex(Sha256::digest(&main)) != raw_main
        {
            return Err(UpdateError::Integrity);
        }
        let canonical = main
            .strip_suffix(MAC_SIGNATURE_FIXTURE)
            .ok_or(UpdateError::Integrity)?;
        Ok(super::digest_hex(Sha256::digest(canonical)))
    })
}
pub(super) fn write_bundle(root: &Path, kind: InstallKind, version: &str, label: &[u8]) {
    write_bundle_files(root, &fixture_files(kind, version, label));
}
pub(super) fn write_bundle_files(root: &Path, files: &BTreeMap<String, (Vec<u8>, u32)>) {
    for (name, (bytes, mode)) in files {
        let path = root.join(name);
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(&path, bytes).unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            fs::set_permissions(&path, fs::Permissions::from_mode(*mode)).unwrap();
            for parent in path
                .parent()
                .unwrap()
                .ancestors()
                .take_while(|p| p.starts_with(root))
            {
                fs::set_permissions(parent, fs::Permissions::from_mode(0o755)).unwrap();
            }
        }
        #[cfg(not(unix))]
        let _ = mode;
    }
}
pub(super) fn write_archive(
    path: &Path,
    kind: InstallKind,
    version: &str,
    label: &[u8],
) -> Selection {
    let files = fixture_files(kind, version, label);
    write_archive_files(path, kind, version, files)
}
pub(super) fn write_archive_files(
    path: &Path,
    kind: InstallKind,
    version: &str,
    files: BTreeMap<String, (Vec<u8>, u32)>,
) -> Selection {
    let root = kind.archive_root(version).unwrap();
    let mut directories = BTreeSet::from([root.clone()]);
    for name in files.keys() {
        for (index, _) in name.match_indices('/') {
            directories.insert(format!("{root}/{}", &name[..index]));
        }
    }
    if kind == InstallKind::Portable {
        let encoder = flate2::write::GzEncoder::new(
            File::create(path).unwrap(),
            flate2::Compression::default(),
        );
        let mut archive = tar::Builder::new(encoder);
        for name in directories {
            append_tar_member(&mut archive, &name, &[], 0o755, true);
        }
        for (name, (bytes, mode)) in files {
            append_tar_member(&mut archive, &format!("{root}/{name}"), &bytes, mode, false);
        }
        archive.into_inner().unwrap().finish().unwrap();
    } else {
        let mut archive = zip::ZipWriter::new(File::create(path).unwrap());
        for name in directories {
            archive
                .add_directory(
                    format!("{name}/"),
                    zip::write::SimpleFileOptions::default().unix_permissions(0o755),
                )
                .unwrap();
        }
        for (name, (bytes, mode)) in files {
            let options = zip::write::SimpleFileOptions::default()
                .compression_method(zip::CompressionMethod::Deflated)
                .unix_permissions(mode);
            archive
                .start_file(format!("{root}/{name}"), options)
                .unwrap();
            archive.write_all(&bytes).unwrap();
        }
        archive.finish().unwrap();
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(path, fs::Permissions::from_mode(0o600)).unwrap();
    }
    selection_for_file(path, kind, version)
}
fn append_tar_member(
    archive: &mut tar::Builder<impl Write>,
    name: &str,
    bytes: &[u8],
    mode: u32,
    directory: bool,
) {
    let mut header = tar::Header::new_ustar();
    header.set_entry_type(if directory {
        tar::EntryType::Directory
    } else {
        tar::EntryType::Regular
    });
    header.set_size(bytes.len() as u64);
    header.set_mode(mode);
    header.set_mtime(1);
    header.set_cksum();
    if name.len() > 100 {
        archive
            .append_pax_extensions([("path", name.as_bytes())])
            .unwrap();
        archive
            .append_data(&mut header, "pax-member", bytes)
            .unwrap();
    } else {
        archive.append_data(&mut header, name, bytes).unwrap();
    }
}
pub(super) fn selection_for_file(path: &Path, kind: InstallKind, version: &str) -> Selection {
    Selection {
        info: ReleaseInfo {
            version: Text::new(version).unwrap(),
            prerelease: version.contains('-'),
            bytes: fs::metadata(path).unwrap().len(),
        },
        name: kind.asset_name(version).unwrap(),
        url: "unused fixture".into(),
        sha256: archive::sha256(path, transport::MAX_DOWNLOAD, &Cancellation::default()).unwrap(),
    }
}
#[test]
fn bounded_tar_and_zip_provenance_checks_use_native_identity_without_execution() {
    let temporary = tempfile::tempdir().unwrap();
    for kind in [InstallKind::Portable, InstallKind::MacBundle] {
        let path = temporary.path().join(kind.asset_name("0.0.2").unwrap());
        let selection = write_archive(&path, kind, "0.0.2", b"NEW-SYNTHETIC");
        let hash = archive::inspect(&path, &selection, kind, &Cancellation::default()).unwrap();
        assert!(transport::valid_digest(&hash));
        let mut wrong_version = selection.clone();
        wrong_version.info.version = Text::new("0.0.3").unwrap();
        assert!(archive::inspect(&path, &wrong_version, kind, &Cancellation::default()).is_err());
    }
}

#[test]
fn mac_preflight_binds_raw_bytes_and_installed_check_requires_canonical_code_and_seal() {
    let temporary = tempfile::tempdir().unwrap();
    let original = fixture_files(InstallKind::MacBundle, "0.0.4", b"CANONICAL-MAIN");
    for code in [true, false] {
        let mut tampered = original.clone();
        let path = if code {
            "Contents/MacOS/talk-desktop"
        } else {
            "Contents/_CodeSignature/CodeResources"
        };
        // Preserve the native header and synthetic signature trailer so a code
        // change must fail the canonical digest, not merely the fixture parser.
        tampered.get_mut(path).unwrap().0[if code { 64 } else { 0 }] ^= 1;
        let archive_path = temporary
            .path()
            .join(if code { "code.zip" } else { "seal.zip" });
        let selection = write_archive_files(
            &archive_path,
            InstallKind::MacBundle,
            "0.0.4",
            tampered.clone(),
        );
        let raw_main = archive::inspect(
            &archive_path,
            &selection,
            InstallKind::MacBundle,
            &Cancellation::default(),
        )
        .unwrap();
        assert_eq!(
            raw_main,
            super::digest_hex(Sha256::digest(&tampered["Contents/MacOS/talk-desktop"].0))
        );
        let root = temporary
            .path()
            .join(if code { "code.app" } else { "seal.app" });
        write_bundle_files(&root, &tampered);
        assert_eq!(
            verify_fixture_tree(
                &root,
                InstallKind::MacBundle,
                "0.0.4",
                &Cancellation::default()
            ),
            Err(UpdateError::Integrity)
        );
    }
    let mut missing_seal = original.clone();
    missing_seal.remove("Contents/_CodeSignature/CodeResources");
    assert_mac_fixture_refused(missing_seal, UpdateError::Integrity);
    for identity in [serde_json::Value::Null, "raw-sha256".into()] {
        let mut downgraded = original.clone();
        edit_mac_provenance(&mut downgraded, |value| {
            value["build"]["macos_main_identity"] = identity;
        });
        assert_mac_fixture_refused(downgraded, UpdateError::UnsupportedInstall);
    }
    let root = temporary.path().join("extra-root.app");
    write_bundle_files(&root, &original);
    fs::create_dir(root.join("extra")).unwrap();
    assert_eq!(
        verify_fixture_tree(
            &root,
            InstallKind::MacBundle,
            "0.0.4",
            &Cancellation::default()
        ),
        Err(UpdateError::UnsafeArchive)
    );
}
fn edit_mac_provenance(
    files: &mut BTreeMap<String, (Vec<u8>, u32)>,
    edit: impl FnOnce(&mut serde_json::Value),
) {
    let bytes = &mut files.get_mut(MAC_PROVENANCE_FIXTURE).unwrap().0;
    let mut value = serde_json::from_slice(bytes).unwrap();
    edit(&mut value);
    *bytes = serde_json::to_vec(&value).unwrap();
}
fn edit_mac_manifest(
    files: &mut BTreeMap<String, (Vec<u8>, u32)>,
    edit: impl FnOnce(&mut serde_json::Value),
) {
    let bytes = &mut files.get_mut(MAC_MANIFEST_FIXTURE).unwrap().0;
    let mut value = serde_json::from_slice(bytes).unwrap();
    edit(&mut value);
    *bytes = serde_json::to_vec(&value).unwrap();
    let digest = super::digest_hex(Sha256::digest(bytes));
    edit_mac_provenance(files, |provenance| {
        provenance["files"]["assets/gstreamer-runtime.json"] = digest.clone().into();
        provenance["build"]["bundled_runtime"]["manifest_sha256"] = digest.into();
    });
}
fn assert_mac_fixture_refused(files: BTreeMap<String, (Vec<u8>, u32)>, expected: UpdateError) {
    let temporary = tempfile::tempdir().unwrap();
    let path = temporary.path().join("runtime.zip");
    let selection = write_archive_files(&path, InstallKind::MacBundle, "0.0.4", files.clone());
    assert_eq!(
        archive::inspect(
            &path,
            &selection,
            InstallKind::MacBundle,
            &Cancellation::default()
        ),
        Err(expected)
    );
    let root = temporary.path().join("installed.app");
    write_bundle_files(&root, &files);
    assert_eq!(
        verify_fixture_tree(
            &root,
            InstallKind::MacBundle,
            "0.0.4",
            &Cancellation::default()
        ),
        Err(expected)
    );
}
#[test]
fn mac_bundle_requires_the_complete_declared_runtime_in_archive_and_installed_tree() {
    let original = fixture_files(InstallKind::MacBundle, "0.0.4", b"NEW-RUNTIME");
    for (_, path, _) in MAC_RUNTIME_FIXTURE {
        let mut missing = original.clone();
        missing.remove(path).unwrap();
        assert_mac_fixture_refused(missing, UpdateError::Integrity);

        let mut tampered = original.clone();
        tampered
            .get_mut(path)
            .unwrap()
            .0
            .extend_from_slice(b"TAMPER");
        assert_mac_fixture_refused(tampered, UpdateError::Integrity);

        let mut misplaced = original.clone();
        let bytes = misplaced.remove(path).unwrap();
        misplaced.insert(format!("Contents/Resources/{path}"), bytes);
        assert_mac_fixture_refused(misplaced, UpdateError::Integrity);
    }
    let mut extra = original.clone();
    extra.insert(
        "Contents/Frameworks/libundeclared.dylib".into(),
        original[MAC_RUNTIME_FIXTURE[0].1].clone(),
    );
    assert_mac_fixture_refused(extra, UpdateError::UnsafeArchive);

    let mut missing_manifest = original.clone();
    missing_manifest.remove(MAC_MANIFEST_FIXTURE).unwrap();
    assert_mac_fixture_refused(missing_manifest, UpdateError::Integrity);

    let mut runtime_free = original.clone();
    for (_, path, _) in MAC_RUNTIME_FIXTURE {
        runtime_free.remove(path).unwrap();
    }
    runtime_free.remove(MAC_MANIFEST_FIXTURE).unwrap();
    edit_mac_provenance(&mut runtime_free, |value| {
        value["build"]
            .as_object_mut()
            .unwrap()
            .remove("bundled_runtime");
        value["files"].as_object_mut().unwrap().retain(|name, _| {
            !name.starts_with("runtime/") && name != "assets/gstreamer-runtime.json"
        });
    });
    assert_mac_fixture_refused(runtime_free, UpdateError::UnsafeArchive);

    // Even a consistently reduced inventory cannot omit the minimal core,
    // scanner or every plugin and silently fall back to a system installation.
    for (suffix, path, _) in MAC_RUNTIME_FIXTURE {
        let mut reduced = original.clone();
        reduced.remove(path).unwrap();
        edit_mac_provenance(&mut reduced, |value| {
            value["files"]
                .as_object_mut()
                .unwrap()
                .remove(&format!("runtime/gstreamer/{suffix}"));
        });
        edit_mac_manifest(&mut reduced, |value| {
            value["files"].as_object_mut().unwrap().remove(suffix);
        });
        assert_mac_fixture_refused(reduced, UpdateError::UnsafeArchive);
    }
}
#[test]
fn mac_runtime_manifest_is_bound_to_the_exact_provenance_inventory_and_layout() {
    let original = fixture_files(InstallKind::MacBundle, "0.0.4", b"NEW-RUNTIME");
    let suffix = MAC_RUNTIME_FIXTURE[0].0;
    for field in ["sha256", "source_sha256", "bundle_path"] {
        let mut inconsistent = original.clone();
        edit_mac_manifest(&mut inconsistent, |value| {
            value["files"][suffix][field] = "incorrect".into();
        });
        assert_mac_fixture_refused(inconsistent, UpdateError::Integrity);
    }
    let mut extra = original.clone();
    edit_mac_manifest(&mut extra, |value| {
        value["files"]["lib/libextra.dylib"] = value["files"][suffix].clone();
    });
    assert_mac_fixture_refused(extra, UpdateError::UnsafeArchive);

    let mut missing = original.clone();
    edit_mac_manifest(&mut missing, |value| {
        value["files"].as_object_mut().unwrap().remove(suffix);
    });
    assert_mac_fixture_refused(missing, UpdateError::Integrity);

    let mut wrong_version = original.clone();
    edit_mac_manifest(&mut wrong_version, |value| {
        value["version"] = "other-runtime".into();
    });
    assert_mac_fixture_refused(wrong_version, UpdateError::UnsafeArchive);

    let mut wrong_digest = original.clone();
    edit_mac_provenance(&mut wrong_digest, |value| {
        value["build"]["bundled_runtime"]["manifest_sha256"] = "0".repeat(64).into();
    });
    assert_mac_fixture_refused(wrong_digest, UpdateError::UnsafeArchive);
}
#[test]
fn mac_runtime_admits_only_regular_arm64_code_in_the_supported_code_directories() {
    let original = fixture_files(InstallKind::MacBundle, "0.0.4", b"NEW-RUNTIME");
    for name in [
        "runtime/other/libexample.dylib",
        "runtime/gstreamer/lib/nested/libexample.dylib",
        "runtime/gstreamer/lib/libexample.framework/example",
        "runtime/gstreamer/lib/libexample.txt",
        "runtime/gstreamer/lib/gstreamer-1.0/libexample.dylib",
        "runtime/gstreamer/libexec/gstreamer-1.0/other-helper",
        "runtime/gstreamer/lib/../libexample.dylib",
    ] {
        let mut unsafe_path = original.clone();
        edit_mac_provenance(&mut unsafe_path, |value| {
            value["files"][name] = "a".repeat(64).into();
        });
        assert_mac_fixture_refused(unsafe_path, UpdateError::UnsafeArchive);
    }
    for (_, path, _) in MAC_RUNTIME_FIXTURE {
        let mut foreign = original.clone();
        foreign.get_mut(path).unwrap().0[4..8].copy_from_slice(&0x01000007u32.to_le_bytes());
        assert_mac_fixture_refused(foreign, UpdateError::UnsafeArchive);

        let mut wrong_type = original.clone();
        wrong_type.get_mut(path).unwrap().0[12..16].copy_from_slice(&1u32.to_le_bytes());
        assert_mac_fixture_refused(wrong_type, UpdateError::UnsafeArchive);
    }
    let mut scanner_mode = original.clone();
    scanner_mode
        .get_mut("Contents/Helpers/gst-plugin-scanner")
        .unwrap()
        .1 = 0o644;
    assert_mac_fixture_refused(scanner_mode, UpdateError::UnsafeArchive);

    let mut legacy_location = original.clone();
    let provenance = legacy_location.remove(MAC_PROVENANCE_FIXTURE).unwrap();
    legacy_location.insert(
        "Contents/_CodeSignature/PackageProvenance.json".into(),
        provenance,
    );
    assert_mac_fixture_refused(legacy_location, UpdateError::UnsafeArchive);

    let mut signature_extra = original.clone();
    signature_extra.insert(
        "Contents/_CodeSignature/unreviewed-file".into(),
        (b"SYNTHETIC extra".to_vec(), 0o644),
    );
    assert_mac_fixture_refused(signature_extra, UpdateError::UnsafeArchive);

    for path in [
        MAC_MANIFEST_FIXTURE,
        "Contents/_CodeSignature/CodeResources",
    ] {
        let mut oversized = original.clone();
        oversized.get_mut(path).unwrap().0 = vec![0; 2 * 1024 * 1024 + 1];
        assert_mac_fixture_refused(oversized, UpdateError::TooLarge);
    }
}
#[test]
#[cfg(unix)]
fn installed_mac_runtime_rejects_symbolic_and_hard_links() {
    use std::os::unix::fs::symlink;
    let temporary = tempfile::tempdir().unwrap();
    for symbolic in [true, false] {
        let root = temporary
            .path()
            .join(if symbolic { "symbolic.app" } else { "hard.app" });
        write_bundle(&root, InstallKind::MacBundle, "0.0.4", b"LINK-FIXTURE");
        let scanner = root.join("Contents/Helpers/gst-plugin-scanner");
        let library = root.join("Contents/Frameworks/libgstreamer-1.0.0.dylib");
        fs::remove_file(&scanner).unwrap();
        if symbolic {
            symlink("../Frameworks/libgstreamer-1.0.0.dylib", &scanner).unwrap();
        } else {
            fs::hard_link(&library, &scanner).unwrap();
        }
        assert_eq!(
            archive::verify_tree(
                &root,
                InstallKind::MacBundle,
                "0.0.4",
                &Cancellation::default()
            ),
            Err(UpdateError::UnsafeArchive)
        );
    }
}
#[test]
#[cfg(unix)]
fn private_slot_is_exclusive_and_stage_tamper_is_detected() {
    let temporary = tempfile::tempdir().unwrap();
    let root = temporary.path().join("private");
    let store = store::Store::at(&root).unwrap();
    assert!(matches!(store::Store::at(&root), Err(UpdateError::Busy)));
    let path = store.payload(InstallKind::Portable);
    let selection = write_archive(&path, InstallKind::Portable, "0.0.2", b"NEW-SYNTHETIC");
    let stage = Staged {
        generation: 1,
        selection,
        target: Target {
            kind: InstallKind::Portable,
            root: temporary.path().join("installed"),
            executable: temporary.path().join("installed/bin/talk-desktop"),
            old_digest: "a".repeat(64),
        },
    };
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap();
    assert!(
        runtime
            .block_on(stage.verify(&store, None, &Cancellation::default()))
            .is_ok()
    );
    let mut bytes = fs::read(&path).unwrap();
    let last = bytes.len() - 1;
    bytes[last] ^= 1;
    fs::write(&path, bytes).unwrap();
    assert_eq!(
        runtime.block_on(stage.verify(&store, None, &Cancellation::default())),
        Err(UpdateError::Integrity)
    );
    let schedule = schedule::Schedule::new(20, 100);
    store.save_schedule(schedule).unwrap();
    drop(store);
    let reopened = store::Store::at(&root).unwrap();
    assert_eq!(reopened.schedule().unwrap(), Some(schedule));
    assert!(!reopened.payload(InstallKind::Portable).exists());
}
#[test]
fn archive_paths_and_nonregular_members_are_rejected() {
    for path in [
        "/absolute",
        "a/../b",
        "a//b",
        "a/./b",
        "a\\b",
        "C:thing",
        "a/evil.",
        "a/evil ",
        "a/\n",
    ] {
        assert_eq!(
            archive::safe_path(path),
            Err(UpdateError::UnsafeArchive),
            "{path}"
        );
    }
    let temporary = tempfile::tempdir().unwrap();
    let path = temporary.path().join("link.tar.gz");
    let encoder =
        flate2::write::GzEncoder::new(File::create(&path).unwrap(), flate2::Compression::default());
    let mut tar = tar::Builder::new(encoder);
    let mut header = tar::Header::new_ustar();
    header.set_entry_type(tar::EntryType::Symlink);
    header.set_size(0);
    header.set_mode(0o777);
    header.set_link_name("../../escape").unwrap();
    header.set_cksum();
    tar.append_data(
        &mut header,
        "nextcloud-modern-talk-0.0.2-linux-x86_64/link",
        std::io::empty(),
    )
    .unwrap();
    tar.into_inner().unwrap().finish().unwrap();
    let selection = selection_for_file(&path, InstallKind::Portable, "0.0.2");
    assert_eq!(
        archive::inspect(
            &path,
            &selection,
            InstallKind::Portable,
            &Cancellation::default()
        ),
        Err(UpdateError::UnsafeArchive)
    );
    for (name, target) in [
        ("link", "../../escape"),
        (
            "Contents/Helpers/gst-plugin-scanner",
            "../Frameworks/libgstreamer-1.0.0.dylib",
        ),
    ] {
        let path = temporary.path().join("link.zip");
        let mut zip = zip::ZipWriter::new(File::create(&path).unwrap());
        zip.add_symlink(
            format!("NextCloud Modern Talk.app/{name}"),
            target,
            zip::write::SimpleFileOptions::default(),
        )
        .unwrap();
        zip.finish().unwrap();
        let selection = selection_for_file(&path, InstallKind::MacBundle, "0.0.2");
        assert_eq!(
            archive::inspect(
                &path,
                &selection,
                InstallKind::MacBundle,
                &Cancellation::default()
            ),
            Err(UpdateError::UnsafeArchive)
        );
    }
}
#[test]
fn expanded_size_is_bounded_before_reading_or_allocating_payload() {
    let temporary = tempfile::tempdir().unwrap();
    let path = temporary.path().join("huge.tar.gz");
    let mut encoder =
        flate2::write::GzEncoder::new(File::create(&path).unwrap(), flate2::Compression::default());
    let mut header = tar::Header::new_ustar();
    header
        .set_path("nextcloud-modern-talk-0.0.2-linux-x86_64/huge")
        .unwrap();
    header.set_size(archive::MAX_EXPANDED + 1);
    header.set_mode(0o644);
    header.set_cksum();
    encoder.write_all(header.as_bytes()).unwrap();
    encoder.finish().unwrap();
    let selection = selection_for_file(&path, InstallKind::Portable, "0.0.2");
    assert_eq!(
        archive::inspect(
            &path,
            &selection,
            InstallKind::Portable,
            &Cancellation::default()
        ),
        Err(UpdateError::TooLarge)
    );
}

#[test]
fn cancellation_during_pending_download_does_not_publish_a_completion() {
    let owner = dormant_owner();
    let generation = owner.snapshot().generation;
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap();
    runtime.block_on(async {
        let (result, ()) = tokio::join!(
            interruptible::<()>(&owner.shared, generation, std::future::pending()),
            async {
                tokio::task::yield_now().await;
                owner.request_shutdown();
            }
        );
        assert_eq!(result, Err(UpdateError::Stopped));
    });
    let before = owner.snapshot();
    publish(&owner.shared, generation, UpdateState::Ready(info()));
    assert_eq!(owner.snapshot(), before);
}
