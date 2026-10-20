//! Private regular files only. Unix operations are anchored to a no-follow
//! directory handle. Windows pins the directory and rejects reparse points;
//! inherited LOCALAPPDATA ACLs alone do not guarantee owner-only access.
use std::{fs::File, io, path::Path};

fn absent(result: io::Result<()>) -> io::Result<()> {
    match result {
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(()),
        result => result,
    }
}
fn denied() -> io::Error {
    io::ErrorKind::PermissionDenied.into()
}

#[cfg(unix)]
pub(super) struct Directory {
    file: File,
    uid: u32,
}
#[cfg(unix)]
impl Directory {
    pub(super) fn open(path: &Path) -> io::Result<Self> {
        use rustix::fs::{self, Mode, OFlags};
        use std::{os::unix::fs::MetadataExt, path::Component};
        if !path.is_absolute()
            || path
                .components()
                .any(|c| matches!(c, Component::ParentDir | Component::CurDir))
        {
            return Err(denied());
        }
        let flags = OFlags::RDONLY | OFlags::DIRECTORY | OFlags::NOFOLLOW | OFlags::CLOEXEC;
        let uid = rustix::process::geteuid().as_raw();
        let mut file = File::from(fs::open("/", flags, Mode::empty())?);
        let mut parts = path
            .components()
            .filter_map(|c| {
                if let Component::Normal(n) = c {
                    Some(n)
                } else {
                    None
                }
            })
            .peekable();
        while let Some(part) = parts.next() {
            let next = match fs::openat(&file, part, flags, Mode::empty()) {
                Ok(next) => next,
                Err(rustix::io::Errno::NOENT) => {
                    match fs::mkdirat(&file, part, Mode::from_raw_mode(0o700)) {
                        Ok(()) | Err(rustix::io::Errno::EXIST) => (),
                        Err(error) => return Err(error.into()),
                    }
                    fs::openat(&file, part, flags, Mode::empty())?
                }
                Err(error) => return Err(error.into()),
            };
            file = File::from(next);
            let metadata = file.metadata()?;
            let final_part = parts.peek().is_none();
            // Root-owned sticky temporary roots are safe for private test folders.
            let sticky_root = metadata.uid() == 0 && metadata.mode() & 0o1000 != 0;
            if metadata.uid() != uid && metadata.uid() != 0
                || metadata.mode() & 0o022 != 0 && !sticky_root
                || final_part && (metadata.uid() != uid || metadata.mode() & 0o077 != 0)
            {
                return Err(denied());
            }
        }
        #[cfg(target_os = "macos")]
        if exacl::getfacl(path, None)?
            .iter()
            .any(|entry| entry.allow && !entry.perms.is_empty())
        {
            return Err(denied());
        }
        Ok(Self { file, uid })
    }
    fn inspect(&self, file: &File) -> io::Result<()> {
        use std::os::unix::fs::MetadataExt;
        let metadata = file.metadata()?;
        if !metadata.is_file()
            || metadata.uid() != self.uid
            || metadata.mode() & 0o077 != 0
            || metadata.nlink() != 1
        {
            return Err(denied());
        }
        Ok(())
    }
    pub(super) fn file(&self, name: &str, exclusive: bool) -> io::Result<File> {
        use rustix::fs::{self, Mode, OFlags};
        let flags = OFlags::RDWR
            | OFlags::CREATE
            | OFlags::NOFOLLOW
            | OFlags::NONBLOCK
            | OFlags::CLOEXEC
            | if exclusive {
                OFlags::EXCL
            } else {
                OFlags::empty()
            };
        let file = File::from(fs::openat(
            &self.file,
            name,
            flags,
            Mode::from_raw_mode(0o600),
        )?);
        self.inspect(&file)?;
        Ok(file)
    }
    pub(super) fn validate(&self, name: &str) -> io::Result<()> {
        use rustix::fs::{self, Mode, OFlags};
        absent((|| {
            let file = File::from(fs::openat(
                &self.file,
                name,
                OFlags::RDONLY | OFlags::NOFOLLOW | OFlags::NONBLOCK | OFlags::CLOEXEC,
                Mode::empty(),
            )?);
            self.inspect(&file)
        })())
    }
    pub(super) fn remove(&self, name: &str) -> io::Result<()> {
        absent(
            rustix::fs::unlinkat(&self.file, name, rustix::fs::AtFlags::empty())
                .map_err(Into::into),
        )
    }
    pub(super) fn rename(&self, from: &str, to: &str) -> io::Result<()> {
        absent(rustix::fs::renameat(&self.file, from, &self.file, to).map_err(Into::into))
    }
}

#[cfg(windows)]
pub(super) struct Directory {
    path: std::path::PathBuf,
    owner: String,
    _chain: Vec<File>,
}
#[cfg(windows)]
impl Directory {
    pub(super) fn open(path: &Path) -> io::Result<Self> {
        use std::{fs::OpenOptions, os::windows::fs::OpenOptionsExt, path::Component};
        use windows_permissions::{
            LocalBox, SecurityDescriptor,
            constants::{SeObjectType, SecurityInformation},
            wrappers,
        };
        if !path.is_absolute()
            || path
                .components()
                .any(|c| matches!(c, Component::ParentDir | Component::CurDir))
        {
            return Err(denied());
        }
        let owner = windows_permissions::utilities::current_process_sid()?.to_string();
        let mut walked = std::path::PathBuf::new();
        let mut chain = Vec::new();
        for part in path.components() {
            walked.push(part.as_os_str());
            if matches!(part, Component::Prefix(_)) {
                continue;
            }
            let created = match std::fs::create_dir(&walked) {
                Ok(()) => true,
                Err(error) if error.kind() == io::ErrorKind::AlreadyExists => false,
                Err(error) => return Err(error),
            };
            // READ_CONTROL | READ_ATTRIBUTES; WRITE_DAC only on our empty new directory.
            let mut file = OpenOptions::new()
                .access_mode(0x00020080 | if created { 0x00040000 } else { 0 })
                .share_mode(3)
                .custom_flags(0x00200000 | 0x02000000)
                .open(&walked)?;
            Self::regular(&file, true)?;
            if created {
                let desired: LocalBox<SecurityDescriptor> =
                    format!("O:{owner}D:P(A;OICI;FA;;;{owner})").parse()?;
                wrappers::SetSecurityInfo(
                    &mut file,
                    SeObjectType::SE_FILE_OBJECT,
                    SecurityInformation::Dacl | SecurityInformation::ProtectedDacl,
                    None,
                    None,
                    desired.dacl(),
                    None,
                )?;
            }
            if created || walked == path {
                Self::inspect(&file, &owner)?;
            }
            chain.push(file);
        }
        Ok(Self {
            path: path.to_owned(),
            owner,
            _chain: chain,
        })
    }
    fn regular(file: &File, directory: bool) -> io::Result<()> {
        use std::os::windows::fs::MetadataExt;
        let metadata = file.metadata()?;
        if metadata.file_attributes() & 0x400 != 0
            || metadata.is_dir() != directory
            || !directory && !metadata.is_file()
        {
            return Err(denied());
        }
        Ok(())
    }
    fn inspect(file: &File, owner: &str) -> io::Result<()> {
        use windows_permissions::{
            constants::{AceFlags, AceType, SeObjectType, SecurityInformation},
            wrappers,
        };
        let security = wrappers::GetSecurityInfo(
            file,
            SeObjectType::SE_FILE_OBJECT,
            SecurityInformation::Owner | SecurityInformation::Dacl,
        )?;
        if security.owner().is_none_or(|sid| sid.to_string() != owner) {
            return Err(denied());
        }
        let acl = security.dacl().ok_or_else(denied)?;
        if acl.len() == 0 || acl.len() > 64 {
            return Err(denied());
        }
        for index in 0..acl.len() {
            let ace = acl.get_ace(index).ok_or_else(denied)?;
            if ace.flags().contains(AceFlags::InheritOnly)
                || ace.ace_type() == AceType::ACCESS_DENIED_ACE_TYPE
            {
                continue;
            }
            if ace.ace_type() != AceType::ACCESS_ALLOWED_ACE_TYPE
                || ace.sid().is_none_or(|sid| sid.to_string() != owner)
            {
                return Err(denied());
            }
        }
        Ok(())
    }
    pub(super) fn file(&self, name: &str, exclusive: bool) -> io::Result<File> {
        use std::{fs::OpenOptions, os::windows::fs::OpenOptionsExt};
        let file = OpenOptions::new()
            .read(true)
            .write(true)
            .create(!exclusive)
            .create_new(exclusive)
            .share_mode(3)
            .custom_flags(0x00200000)
            .open(self.path.join(name))?;
        Self::regular(&file, false)?;
        Self::inspect(&file, &self.owner)?;
        Ok(file)
    }
    pub(super) fn validate(&self, name: &str) -> io::Result<()> {
        use std::{fs::OpenOptions, os::windows::fs::OpenOptionsExt};
        absent((|| {
            let file = OpenOptions::new()
                .read(true)
                .share_mode(3)
                .custom_flags(0x00200000)
                .open(self.path.join(name))?;
            Self::regular(&file, false)?;
            Self::inspect(&file, &self.owner)
        })())
    }
    pub(super) fn remove(&self, name: &str) -> io::Result<()> {
        absent(std::fs::remove_file(self.path.join(name)))
    }
    pub(super) fn rename(&self, from: &str, to: &str) -> io::Result<()> {
        absent(std::fs::rename(self.path.join(from), self.path.join(to)))
    }
}
