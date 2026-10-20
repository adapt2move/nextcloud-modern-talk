//! Owner DACLs and pinned, non-reparse Windows handles. No unsafe application code.
use super::super::error::{self, Result};
use std::{
    fs::{File, OpenOptions},
    io::{Read, Write},
    os::windows::fs::OpenOptionsExt,
    path::{Component, Path, PathBuf, Prefix},
};
use windows_permissions::{
    LocalBox, SecurityDescriptor,
    constants::{AccessRights, AceFlags, AceType, SeObjectType, SecurityInformation},
    wrappers,
};
use zeroize::Zeroizing;
const READ_CONTROL: u32 = 0x00020000;
const WRITE_DAC: u32 = 0x00040000;
const READ_ATTRIBUTES: u32 = 0x80;
const OPEN_REPARSE: u32 = 0x00200000;
const BACKUP_SEMANTICS: u32 = 0x02000000;
const WRITE_THROUGH: u32 = 0x80000000;
const REPARSE: u64 = 0x400;
const DIRECTORY: u64 = 0x10;

pub(in crate::identity) struct PrivateDir {
    path: PathBuf,
    owner: String,
    chain: Vec<File>,
}
fn open_dir(path: &Path, write_dacl: bool) -> Result<File> {
    OpenOptions::new()
        .access_mode(READ_CONTROL | READ_ATTRIBUTES | if write_dacl { WRITE_DAC } else { 0 })
        // Pin every ancestor against rename/delete for this provider lifetime.
        .share_mode(3)
        .custom_flags(OPEN_REPARSE | BACKUP_SEMANTICS)
        .security_qos_flags(0x00100000)
        .open(path)
        .map_err(error::io)
}
fn security(file: &File) -> Result<LocalBox<SecurityDescriptor>> {
    wrappers::GetSecurityInfo(
        file,
        SeObjectType::SE_FILE_OBJECT,
        SecurityInformation::Owner | SecurityInformation::Dacl,
    )
    .map_err(error::io)
}
fn inspect(file: &File, owner: &str, private: bool, directory: bool) -> Result<()> {
    let info = winapi_util::file::information(file).map_err(error::io)?;
    if !winapi_util::file::typ(file).map_err(error::io)?.is_disk()
        || info.file_attributes() & REPARSE != 0
        || (info.file_attributes() & DIRECTORY != 0) != directory
        || (!directory && info.number_of_links() != 1)
    {
        return Err(error::denied());
    }
    let sd = security(file)?;
    let actual = sd.owner().ok_or_else(error::denied)?.to_string();
    if actual != owner && (private || !matches!(actual.as_str(), "S-1-5-18" | "S-1-5-32-544")) {
        return Err(error::denied());
    }
    let acl = sd.dacl().ok_or_else(error::denied)?;
    if acl.len() == 0 || acl.len() > 64 {
        return Err(error::denied());
    }
    let mut owner_access = false;
    for i in 0..acl.len() {
        let ace = acl.get_ace(i).ok_or_else(error::denied)?;
        match ace.ace_type() {
            AceType::ACCESS_DENIED_ACE_TYPE => continue,
            AceType::ACCESS_ALLOWED_ACE_TYPE => {}
            _ => return Err(error::denied()),
        }
        if ace.flags().contains(AceFlags::InheritOnly) {
            continue;
        }
        let sid = ace.sid().ok_or_else(error::denied)?.to_string();
        if sid == owner {
            owner_access |= ace.mask().contains(AccessRights::FileAllAccess);
            continue;
        }
        if private {
            return Err(error::denied());
        }
        if matches!(sid.as_str(), "S-1-5-18" | "S-1-5-32-544") {
            continue;
        }
        // No other principal may replace a pinned ancestor or change its ACL/owner.
        let dangerous = AccessRights::GenericAll
            | AccessRights::GenericWrite
            | AccessRights::WriteDac
            | AccessRights::WriteOwner
            | AccessRights::Delete
            | AccessRights::Bit6;
        if ace.mask().intersects(dangerous) {
            return Err(error::denied());
        }
    }
    if private && !owner_access {
        return Err(error::denied());
    }
    Ok(())
}
fn restrict(file: &mut File, owner: &str) -> Result<()> {
    let desired: LocalBox<SecurityDescriptor> = format!("O:{owner}D:P(A;OICI;FA;;;{owner})")
        .parse()
        .map_err(error::io)?;
    wrappers::SetSecurityInfo(
        file,
        SeObjectType::SE_FILE_OBJECT,
        SecurityInformation::Dacl | SecurityInformation::ProtectedDacl,
        None,
        None,
        desired.dacl(),
        None,
    )
    .map_err(error::io)
}
impl PrivateDir {
    pub(in crate::identity) fn open(path: &Path, _fixture: bool) -> Result<Self> {
        let parts: Vec<_> = path.components().collect();
        if !path.is_absolute()
            || parts.len() < 3
            || parts.len() > 64
            || !matches!(parts[0],Component::Prefix(p) if matches!(p.kind(),Prefix::Disk(_)|Prefix::VerbatimDisk(_)))
            || parts
                .iter()
                .any(|p| matches!(p, Component::ParentDir | Component::CurDir))
        {
            return Err(error::denied());
        }
        let owner = windows_permissions::utilities::current_process_sid()
            .map_err(error::io)?
            .to_string();
        let mut walked = PathBuf::new();
        let mut chain = Vec::new();
        for (i, part) in parts.iter().enumerate() {
            walked.push(part.as_os_str());
            if matches!(part, Component::Prefix(_)) {
                continue;
            }
            let mut created = false;
            match std::fs::symlink_metadata(&walked) {
                Ok(_) => {}
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                    std::fs::create_dir(&walked).map_err(error::io)?;
                    created = true;
                }
                Err(e) => return Err(error::io(e)),
            }
            let mut handle = open_dir(&walked, created)?;
            // A newly created empty directory is restricted before any credential or metadata.
            if created {
                inspect(&handle, &owner, false, true)?;
                restrict(&mut handle, &owner)?;
            }
            inspect(&handle, &owner, created || i + 1 == parts.len(), true)?;
            chain.push(handle);
        }
        Ok(Self {
            path: path.to_owned(),
            owner,
            chain,
        })
    }
    pub(in crate::identity) fn path(&self) -> &Path {
        &self.path
    }
    pub(in crate::identity) fn check(&self) -> Result<()> {
        let directory = self.chain.last().ok_or_else(error::denied)?;
        inspect(directory, &self.owner, true, true)?;
        let live = open_dir(&self.path, false)?;
        inspect(&live, &self.owner, true, true)?;
        let original = winapi_util::file::information(directory).map_err(error::io)?;
        let current = winapi_util::file::information(&live).map_err(error::io)?;
        if original.volume_serial_number() != current.volume_serial_number()
            || original.file_index() != current.file_index()
        {
            return Err(error::denied());
        }
        Ok(())
    }
    pub(in crate::identity) fn binding(&self) -> Result<String> {
        self.check()?;
        let i = winapi_util::file::information(self.chain.last().ok_or_else(error::denied)?)
            .map_err(error::io)?;
        Ok(format!(
            "windows:{}:{}:{}",
            self.owner,
            i.volume_serial_number(),
            i.file_index()
        ))
    }
    fn name(name: &str) -> Result<()> {
        let stem = name.split('.').next().unwrap_or("").to_ascii_uppercase();
        if matches!(stem.as_str(), "CON" | "PRN" | "AUX" | "NUL")
            || stem
                .strip_prefix("COM")
                .or_else(|| stem.strip_prefix("LPT"))
                .is_some_and(|n| n.len() == 1 && matches!(n.as_bytes()[0], b'1'..=b'9'))
        {
            return Err(error::denied());
        }
        if name.is_empty()
            || name.len() > 160
            || name.ends_with('.')
            || !name
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || b"-_.".contains(&b))
            || matches!(name, "." | "..")
        {
            return Err(error::denied());
        }
        Ok(())
    }
    fn file(&self, name: &str, write: bool, create: bool, exclusive: bool) -> Result<File> {
        Self::name(name)?;
        self.check()?;
        let mut options = OpenOptions::new();
        options
            .read(true)
            .write(write)
            .share_mode(7)
            .custom_flags(OPEN_REPARSE | if write { WRITE_THROUGH } else { 0 })
            .security_qos_flags(0x00100000);
        if exclusive {
            options.create_new(true);
        } else {
            options.create(create);
        }
        let f = options.open(self.path.join(name)).map_err(error::io)?;
        inspect(&f, &self.owner, true, false)?;
        Ok(f)
    }
    pub(in crate::identity) fn lock(&self) -> Result<File> {
        let f = self.file("profile.lock", true, true, false)?;
        f.try_lock().map_err(|_| {
            error::storage(
                talk_core::identity::StorageBackend::PrivateFile,
                talk_core::identity::StorageFailure::Conflict,
            )
        })?;
        Ok(f)
    }
    pub(in crate::identity) fn read(&self, name: &str, limit: usize) -> Result<Zeroizing<Vec<u8>>> {
        let mut f = self.file(name, false, false, false)?;
        if f.metadata().map_err(error::io)?.len() > limit as u64 {
            return Err(error::corrupt());
        }
        let mut bytes = Zeroizing::new(Vec::with_capacity(limit.min(16384)));
        (&mut f)
            .take(limit as u64 + 1)
            .read_to_end(&mut bytes)
            .map_err(error::io)?;
        inspect(&f, &self.owner, true, false)?;
        if bytes.len() > limit {
            return Err(error::corrupt());
        }
        Ok(bytes)
    }
    pub(in crate::identity) fn prepare_database(&self, name: &str) -> Result<()> {
        self.file(name, true, true, false)?;
        for suffix in ["-journal", "-wal", "-shm"] {
            let n = format!("{name}{suffix}");
            if self.path.join(&n).symlink_metadata().is_ok() {
                self.file(&n, true, false, false)?;
            }
        }
        Ok(())
    }
    pub(in crate::identity) fn create(&self, name: &str, bytes: &[u8]) -> Result<()> {
        Self::name(name)?;
        if bytes.len() > 16384 {
            return Err(error::corrupt());
        }
        let stage = format!("{name}.stage");
        let mut f = self.file(&stage, true, false, true)?;
        f.write_all(bytes).map_err(error::io)?;
        f.sync_all().map_err(error::io)?;
        inspect(&f, &self.owner, true, false)?;
        self.check()?;
        atomicwrites::move_atomic(&self.path.join(stage), &self.path.join(name))
            .map_err(error::io)?;
        f.sync_all().map_err(error::io)?;
        self.file(name, false, false, false)?;
        Ok(())
    }
    pub(in crate::identity) fn remove(&self, name: &str) -> Result<()> {
        let f = match self.file(name, true, false, false) {
            Ok(f) => f,
            Err(super::super::error::IdentityError::Storage {
                failure: talk_core::identity::StorageFailure::Missing,
                ..
            }) => return Ok(()),
            Err(e) => return Err(e),
        };
        std::fs::remove_file(self.path.join(name)).map_err(error::io)?;
        f.sync_all().map_err(error::io)?;
        drop(f);
        match std::fs::symlink_metadata(self.path.join(name)) {
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
            _ => Err(error::denied()),
        }
    }
}
