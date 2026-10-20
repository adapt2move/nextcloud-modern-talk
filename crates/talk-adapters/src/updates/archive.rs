//! Pre-extraction bounds and closed package identity for our tar.gz/.zip layouts.
//! No extraction occurs here. The released updater performs the actual bundle swap.
use super::{
    UpdateError,
    install::InstallKind,
    transport::{Selection, valid_digest},
};
use sha2::{Digest, Sha256};
use std::{
    collections::{BTreeMap, BTreeSet},
    fs::{self, File},
    io::{Read, Seek, SeekFrom},
    path::Path,
    time::{Duration, Instant},
};
use talk_core::lifecycle::Cancellation;

pub(super) const MAX_EXPANDED: u64 = 1024 * 1024 * 1024;
const MAX_ENTRIES: usize = 30_000;
const MAX_PATH: usize = 1024;
const MAX_PROVENANCE: u64 = 2 * 1024 * 1024;
const VERIFY_DEADLINE: Duration = Duration::from_secs(180);

struct Budget<'a> {
    cancel: &'a Cancellation,
    until: Instant,
    total: u64,
    entries: usize,
}
impl<'a> Budget<'a> {
    fn new(cancel: &'a Cancellation) -> Self {
        Self {
            cancel,
            until: Instant::now() + VERIFY_DEADLINE,
            total: 0,
            entries: 0,
        }
    }
    fn check(&self) -> Result<(), UpdateError> {
        if self.cancel.is_cancelled() {
            Err(UpdateError::Cancelled)
        } else if Instant::now() >= self.until {
            Err(UpdateError::TooLarge)
        } else {
            Ok(())
        }
    }
    fn entry(&mut self, size: u64) -> Result<(), UpdateError> {
        self.check()?;
        self.entries += 1;
        self.total = self.total.checked_add(size).ok_or(UpdateError::TooLarge)?;
        if self.entries > MAX_ENTRIES || self.total > MAX_EXPANDED {
            return Err(UpdateError::TooLarge);
        }
        Ok(())
    }
}
struct LimitedReader<'a, R> {
    inner: R,
    left: u64,
    cancel: &'a Cancellation,
    until: Instant,
}
impl<R: Read> Read for LimitedReader<'_, R> {
    fn read(&mut self, output: &mut [u8]) -> std::io::Result<usize> {
        if self.cancel.is_cancelled() || Instant::now() >= self.until {
            return Err(std::io::Error::other(
                "update verification cancelled or expired",
            ));
        }
        if output.is_empty() {
            return Ok(0);
        }
        if self.left == 0 {
            return Err(std::io::Error::other("archive byte budget exceeded"));
        }
        let bound = output.len().min(self.left as usize);
        let count = self.inner.read(&mut output[..bound])?;
        self.left -= count as u64;
        Ok(count)
    }
}
#[derive(Debug)]
struct Record {
    sha256: String,
    head: Vec<u8>,
    mode: u32,
    content: Option<Vec<u8>>,
}
struct Inventory {
    files: BTreeMap<String, Record>,
    paths: BTreeMap<String, bool>,
    folded: BTreeSet<String>,
}
impl Inventory {
    fn new() -> Self {
        Self {
            files: BTreeMap::new(),
            paths: BTreeMap::new(),
            folded: BTreeSet::new(),
        }
    }
    fn path(&mut self, name: &str, directory: bool) -> Result<(), UpdateError> {
        safe_path(name)?;
        if self.paths.len() >= MAX_ENTRIES || !self.folded.insert(name.to_ascii_lowercase()) {
            return Err(UpdateError::UnsafeArchive);
        }
        let mut prefix = String::new();
        for part in name
            .split('/')
            .take(name.split('/').count().saturating_sub(1))
        {
            if !prefix.is_empty() {
                prefix.push('/');
            }
            prefix.push_str(part);
            if self.paths.get(&prefix) == Some(&false) {
                return Err(UpdateError::UnsafeArchive);
            }
        }
        if !directory {
            let prefix = format!("{name}/");
            if self
                .paths
                .range(prefix.clone()..)
                .next()
                .is_some_and(|(path, _)| path.starts_with(&prefix))
            {
                return Err(UpdateError::UnsafeArchive);
            }
        }
        self.paths.insert(name.to_owned(), directory);
        Ok(())
    }
    fn read(
        &mut self,
        name: String,
        size: u64,
        mode: u32,
        reader: &mut impl Read,
        budget: &mut Budget<'_>,
        provenance: &str,
    ) -> Result<(), UpdateError> {
        self.path(&name, false)?;
        if !matches!(mode, 0o644 | 0o755) {
            return Err(UpdateError::UnsafeArchive);
        }
        budget.entry(size)?;
        let collect = name == provenance;
        if collect && size > MAX_PROVENANCE {
            return Err(UpdateError::TooLarge);
        }
        let mut content = collect.then(Vec::new);
        let mut digest = Sha256::new();
        let mut head = Vec::with_capacity(32);
        let mut bytes = [0u8; 64 * 1024];
        let mut count = 0u64;
        loop {
            budget.check()?;
            let read = reader
                .read(&mut bytes)
                .map_err(|_| UpdateError::UnsafeArchive)?;
            if read == 0 {
                break;
            }
            count = count
                .checked_add(read as u64)
                .ok_or(UpdateError::TooLarge)?;
            if count > size {
                return Err(UpdateError::TooLarge);
            }
            digest.update(&bytes[..read]);
            if head.len() < 32 {
                head.extend_from_slice(&bytes[..read.min(32 - head.len())]);
            }
            if let Some(content) = &mut content {
                content.extend_from_slice(&bytes[..read]);
            }
        }
        if count != size {
            return Err(UpdateError::UnsafeArchive);
        }
        self.files.insert(
            name,
            Record {
                sha256: super::digest_hex(digest.finalize()),
                head,
                mode,
                content,
            },
        );
        Ok(())
    }
}
pub(super) fn safe_path(name: &str) -> Result<(), UpdateError> {
    if name.is_empty()
        || name.len() > MAX_PATH
        || !name.is_ascii()
        || name
            .bytes()
            .any(|b| b.is_ascii_control() || matches!(b, b'\\' | b':'))
        || name.split('/').any(|p| {
            p.is_empty() || matches!(p, "." | "..") || (p.ends_with(' ') || p.ends_with('.'))
        })
    {
        return Err(UpdateError::UnsafeArchive);
    }
    Ok(())
}
fn member_path(name: &str, root: &str, directory: bool) -> Result<String, UpdateError> {
    let normalized = if directory {
        name.strip_suffix('/').unwrap_or(name)
    } else {
        name
    };
    safe_path(normalized)?;
    if normalized == root && directory {
        return Ok(root.to_owned());
    }
    if !normalized.starts_with(&format!("{root}/")) {
        return Err(UpdateError::UnsafeArchive);
    }
    Ok(normalized.to_owned())
}
pub(super) fn sha256(
    path: &Path,
    limit: u64,
    cancel: &Cancellation,
) -> Result<String, UpdateError> {
    let meta = fs::symlink_metadata(path).map_err(|_| UpdateError::Storage)?;
    if !meta.is_file() || meta.file_type().is_symlink() || meta.len() > limit {
        return Err(UpdateError::Integrity);
    }
    let mut file = File::open(path).map_err(|_| UpdateError::Storage)?;
    let mut digest = Sha256::new();
    let mut bytes = [0u8; 64 * 1024];
    let mut count = 0u64;
    let until = Instant::now() + VERIFY_DEADLINE;
    loop {
        if cancel.is_cancelled() {
            return Err(UpdateError::Cancelled);
        }
        if Instant::now() >= until {
            return Err(UpdateError::TooLarge);
        }
        let n = file.read(&mut bytes).map_err(|_| UpdateError::Storage)?;
        if n == 0 {
            break;
        }
        count = count.checked_add(n as u64).ok_or(UpdateError::TooLarge)?;
        if count > limit {
            return Err(UpdateError::TooLarge);
        }
        digest.update(&bytes[..n]);
    }
    if count != meta.len() {
        return Err(UpdateError::Integrity);
    }
    Ok(super::digest_hex(digest.finalize()))
}
pub(super) fn inspect(
    path: &Path,
    selection: &Selection,
    kind: InstallKind,
    cancel: &Cancellation,
) -> Result<String, UpdateError> {
    let root = kind.archive_root(selection.info.version.as_str())?;
    let provenance = format!("{root}/{}provenance.json", kind.resources());
    let mut inventory = Inventory::new();
    let mut budget = Budget::new(cancel);
    match kind {
        InstallKind::Portable => {
            inspect_tar(path, &root, &provenance, &mut inventory, &mut budget)?
        }
        InstallKind::MacBundle => {
            inspect_zip(path, &root, &provenance, &mut inventory, &mut budget)?
        }
        InstallKind::Debian => return Err(UpdateError::UnsupportedInstall),
    }
    identity(&inventory, &root, kind, selection.info.version.as_str())
}
fn inspect_tar(
    path: &Path,
    root: &str,
    provenance: &str,
    inventory: &mut Inventory,
    budget: &mut Budget<'_>,
) -> Result<(), UpdateError> {
    let file = File::open(path).map_err(|_| UpdateError::Storage)?;
    let decoder = flate2::read::GzDecoder::new(file);
    let reader = LimitedReader {
        inner: decoder,
        left: MAX_EXPANDED + 32 * 1024 * 1024,
        cancel: budget.cancel,
        until: budget.until,
    };
    let mut archive = tar::Archive::new(reader);
    let mut pax_path: Option<String> = None;
    for entry in archive
        .entries()
        .map_err(|_| UpdateError::UnsafeArchive)?
        .raw(true)
    {
        budget.check()?;
        let mut entry = entry.map_err(|_| UpdateError::UnsafeArchive)?;
        let kind = entry.header().entry_type();
        let size = entry
            .header()
            .size()
            .map_err(|_| UpdateError::UnsafeArchive)?;
        if kind.is_pax_local_extensions() {
            if size > 8192 || pax_path.is_some() {
                return Err(UpdateError::UnsafeArchive);
            }
            budget.entry(size)?;
            let extensions = entry
                .pax_extensions()
                .map_err(|_| UpdateError::UnsafeArchive)?
                .ok_or(UpdateError::UnsafeArchive)?;
            for extension in extensions {
                let extension = extension.map_err(|_| UpdateError::UnsafeArchive)?;
                // Our package builder uses PAX only for long ASCII paths. Reject
                // size/sparse/link/global overrides before tar's extractor sees them.
                if extension.key().map_err(|_| UpdateError::UnsafeArchive)? != "path"
                    || pax_path.is_some()
                {
                    return Err(UpdateError::UnsafeArchive);
                }
                let path = extension.value().map_err(|_| UpdateError::UnsafeArchive)?;
                safe_path(path.strip_suffix('/').unwrap_or(path))?;
                pax_path = Some(path.to_owned());
            }
            if pax_path.is_none() {
                return Err(UpdateError::UnsafeArchive);
            }
            continue;
        }
        if !kind.is_file() && !kind.is_dir() {
            return Err(UpdateError::UnsafeArchive);
        }
        let name = match pax_path.take() {
            Some(path) => path,
            None => String::from_utf8(entry.path_bytes().into_owned())
                .map_err(|_| UpdateError::UnsafeArchive)?,
        };
        let name = member_path(&name, root, kind.is_dir())?;
        let mode = entry
            .header()
            .mode()
            .map_err(|_| UpdateError::UnsafeArchive)?;
        if !matches!(mode, 0o644 | 0o755) {
            return Err(UpdateError::UnsafeArchive);
        }
        if kind.is_dir() {
            if mode != 0o755 || size != 0 {
                return Err(UpdateError::UnsafeArchive);
            }
            budget.entry(0)?;
            inventory.path(&name, true)?;
        } else {
            inventory.read(name, size, mode, &mut entry, budget, provenance)?;
        }
    }
    if pax_path.is_some() {
        return Err(UpdateError::UnsafeArchive);
    }
    Ok(())
}
/// Bound central-directory allocation before constructing ZipArchive. Our packager
/// emits classic, single-disk ZIP without comment/ZIP64; unsupported containers are
/// rejected rather than trusting the remote entry count to allocate memory.
fn zip_preflight(file: &mut File) -> Result<usize, UpdateError> {
    let length = file.metadata().map_err(|_| UpdateError::Storage)?.len();
    if length < 22 {
        return Err(UpdateError::UnsafeArchive);
    }
    let tail_len = length.min(65_558) as usize;
    file.seek(SeekFrom::End(-(tail_len as i64)))
        .map_err(|_| UpdateError::UnsafeArchive)?;
    let mut tail = vec![0u8; tail_len];
    file.read_exact(&mut tail)
        .map_err(|_| UpdateError::UnsafeArchive)?;
    if tail.windows(4).filter(|v| *v == b"PK\x05\x06").count() != 1
        || tail
            .windows(4)
            .any(|v| v == b"PK\x06\x06" || v == b"PK\x06\x07")
    {
        return Err(UpdateError::UnsafeArchive);
    }
    let end = &tail[tail_len - 22..];
    let u16_at = |i| u16::from_le_bytes([end[i], end[i + 1]]);
    let u32_at = |i| u32::from_le_bytes([end[i], end[i + 1], end[i + 2], end[i + 3]]);
    if &end[..4] != b"PK\x05\x06"
        || u16_at(4) != 0
        || u16_at(6) != 0
        || u16_at(20) != 0
        || u16_at(8) != u16_at(10)
        || u16_at(10) as usize > MAX_ENTRIES
        || u16_at(10) == 0
        || u32_at(12) > 16 * 1024 * 1024
        || u32_at(16) as u64 + u32_at(12) as u64 != length - 22
    {
        return Err(UpdateError::UnsafeArchive);
    }
    file.rewind().map_err(|_| UpdateError::UnsafeArchive)?;
    Ok(u16_at(10) as usize)
}
fn inspect_zip(
    path: &Path,
    root: &str,
    provenance: &str,
    inventory: &mut Inventory,
    budget: &mut Budget<'_>,
) -> Result<(), UpdateError> {
    let mut file = File::open(path).map_err(|_| UpdateError::Storage)?;
    let declared_entries = zip_preflight(&mut file)?;
    let config = zip::read::Config {
        archive_offset: zip::read::ArchiveOffset::Known(0),
    };
    let mut archive =
        zip::ZipArchive::with_config(config, file).map_err(|_| UpdateError::UnsafeArchive)?;
    if archive.len() != declared_entries
        || archive.len() > MAX_ENTRIES
        || archive
            .has_overlapping_entries()
            .map_err(|_| UpdateError::UnsafeArchive)?
    {
        return Err(UpdateError::UnsafeArchive);
    }
    for index in 0..archive.len() {
        budget.check()?;
        let mut entry = archive
            .by_index(index)
            .map_err(|_| UpdateError::UnsafeArchive)?;
        let mode = entry.unix_mode().ok_or(UpdateError::UnsafeArchive)?;
        let directory = entry.is_dir();
        if entry.encrypted()
            || mode & 0o170000 != (if directory { 0o040000 } else { 0o100000 })
            || !matches!(
                entry.compression(),
                zip::CompressionMethod::Stored | zip::CompressionMethod::Deflated
            )
        {
            return Err(UpdateError::UnsafeArchive);
        }
        let name = member_path(
            &entry.name().map_err(|_| UpdateError::UnsafeArchive)?,
            root,
            directory,
        )?;
        let size = entry.size();
        if directory {
            if mode & 0o7777 != 0o755 || size != 0 {
                return Err(UpdateError::UnsafeArchive);
            }
            budget.entry(0)?;
            inventory.path(&name, true)?;
        } else {
            inventory.read(name, size, mode & 0o7777, &mut entry, budget, provenance)?;
        }
    }
    Ok(())
}
fn identity(
    inventory: &Inventory,
    root: &str,
    kind: InstallKind,
    version: &str,
) -> Result<String, UpdateError> {
    let resources = format!("{root}/{}", kind.resources());
    let provenance = inventory
        .files
        .get(&format!("{resources}provenance.json"))
        .and_then(|r| r.content.as_deref())
        .ok_or(UpdateError::UnsafeArchive)?;
    let value: serde_json::Value =
        serde_json::from_slice(provenance).map_err(|_| UpdateError::UnsafeArchive)?;
    validate_provenance(&value, kind, version)?;
    let files = value["files"]
        .as_object()
        .filter(|v| v.len() <= 20_000)
        .ok_or(UpdateError::UnsafeArchive)?;
    let mut expected = BTreeSet::new();
    for (name, hash) in files {
        safe_path(name)?;
        let hash = hash
            .as_str()
            .filter(|v| valid_digest(v))
            .ok_or(UpdateError::UnsafeArchive)?;
        let path = if name.starts_with("bin/") {
            if !matches!(name.as_str(), "bin/talk-desktop" | "bin/talk-native-helper") {
                return Err(UpdateError::UnsafeArchive);
            }
            format!(
                "{root}/{}{}",
                kind.binary_directory(),
                name.trim_start_matches("bin/")
            )
        } else {
            if !(name.starts_with("licenses/")
                || name.starts_with("assets/")
                || matches!(name.as_str(), "dependencies.json" | "source-manifest.json"))
            {
                return Err(UpdateError::UnsafeArchive);
            }
            if kind == InstallKind::MacBundle
                && name.starts_with("assets/sounds/")
                && name.ends_with(".wav")
            {
                format!(
                    "{resources}{}",
                    name.rsplit('/').next().ok_or(UpdateError::UnsafeArchive)?
                )
            } else {
                format!("{resources}{name}")
            }
        };
        if !expected.insert(path.clone())
            || inventory.files.get(&path).is_none_or(|r| r.sha256 != hash)
        {
            return Err(UpdateError::Integrity);
        }
    }
    for extra in ["provenance.json", "INSTALL.txt"] {
        expected.insert(format!("{resources}{extra}"));
    }
    if kind == InstallKind::MacBundle {
        expected.insert(format!("{root}/Contents/Info.plist"));
    }
    if inventory.files.keys().any(|path| !expected.contains(path))
        || expected.len() != inventory.files.len()
    {
        return Err(UpdateError::UnsafeArchive);
    }
    for required in [
        "licenses/project/LICENSE",
        "licenses/project/NOTICE.md",
        "licenses/desktop/NOTICE.md",
        "source-manifest.json",
        "dependencies.json",
    ] {
        if !files.contains_key(required) {
            return Err(UpdateError::UnsafeArchive);
        }
    }
    let executable = inventory
        .files
        .get(&format!("{root}/{}talk-desktop", kind.binary_directory()))
        .ok_or(UpdateError::UnsafeArchive)?;
    native_header(&executable.head, kind)?;
    if executable.mode & 0o111 != 0o111 {
        return Err(UpdateError::UnsafeArchive);
    }
    Ok(executable.sha256.clone())
}
pub(super) fn validate_provenance(
    value: &serde_json::Value,
    kind: InstallKind,
    version: &str,
) -> Result<(), UpdateError> {
    if value["schema"].as_u64() != Some(1)
        || value["kind"].as_str() != Some("development")
        || value["version"].as_str() != Some(version)
        || value["target"].as_str() != Some(kind.target())
        || value["build"]["profile"].as_str() != Some("release")
        || value["build"]["package"].as_str() != Some("talk-desktop")
        || value["build"]["features"]
            .as_array()
            .is_none_or(|v| !v.is_empty())
    {
        return Err(UpdateError::UnsupportedInstall);
    }
    Ok(())
}
pub(super) fn native_header(bytes: &[u8], kind: InstallKind) -> Result<(), UpdateError> {
    let valid = bytes.len() >= 32
        && match kind {
            InstallKind::Portable | InstallKind::Debian => {
                &bytes[..6] == b"\x7fELF\x02\x01"
                    && u16::from_le_bytes([bytes[18], bytes[19]]) == 62
            }
            InstallKind::MacBundle => {
                &bytes[..4] == b"\xcf\xfa\xed\xfe"
                    && u32::from_le_bytes([bytes[4], bytes[5], bytes[6], bytes[7]]) == 0x0100000c
            }
        };
    if valid {
        Ok(())
    } else {
        Err(UpdateError::UnsafeArchive)
    }
}
pub(super) fn verify_tree(
    root: &Path,
    kind: InstallKind,
    version: &str,
    cancel: &Cancellation,
) -> Result<String, UpdateError> {
    let name = "bundle";
    let provenance = format!("{name}/{}provenance.json", kind.resources());
    let mut inventory = Inventory::new();
    let mut budget = Budget::new(cancel);
    walk(root, name, &provenance, &mut inventory, &mut budget, 0)?;
    identity(&inventory, name, kind, version)
}
fn walk(
    path: &Path,
    name: &str,
    provenance: &str,
    inventory: &mut Inventory,
    budget: &mut Budget<'_>,
    depth: usize,
) -> Result<(), UpdateError> {
    if depth > 32 {
        return Err(UpdateError::UnsafeArchive);
    }
    let metadata = fs::symlink_metadata(path).map_err(|_| UpdateError::Storage)?;
    if metadata.is_dir() {
        #[cfg(unix)]
        {
            use std::os::unix::fs::MetadataExt;
            if metadata.mode() & 0o022 != 0 {
                return Err(UpdateError::UnsafeArchive);
            }
        }
        budget.entry(0)?;
        inventory.path(name, true)?;
        for entry in fs::read_dir(path).map_err(|_| UpdateError::Storage)? {
            budget.check()?;
            let entry = entry.map_err(|_| UpdateError::Storage)?;
            let child = entry
                .file_name()
                .into_string()
                .map_err(|_| UpdateError::UnsafeArchive)?;
            walk(
                &entry.path(),
                &format!("{name}/{child}"),
                provenance,
                inventory,
                budget,
                depth + 1,
            )?;
        }
    } else if metadata.is_file() && !metadata.file_type().is_symlink() {
        #[cfg(unix)]
        let mode = {
            use std::os::unix::fs::MetadataExt;
            if metadata.nlink() != 1 {
                return Err(UpdateError::UnsafeArchive);
            }
            metadata.mode() & 0o7777
        };
        #[cfg(not(unix))]
        let mode = 0o755;
        let mut file = File::open(path).map_err(|_| UpdateError::Storage)?;
        inventory.read(
            name.to_owned(),
            metadata.len(),
            mode,
            &mut file,
            budget,
            provenance,
        )?;
    } else {
        return Err(UpdateError::UnsafeArchive);
    }
    Ok(())
}
