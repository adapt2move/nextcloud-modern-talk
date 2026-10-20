//! One private directory and process lock; fixed names, no arbitrary cleanup targets.
use super::{UpdateError, install::InstallKind, schedule::Schedule};
use std::{
    fs::{self, File},
    io::{Read, Write},
    path::{Path, PathBuf},
};

pub(super) struct Store {
    pub root: PathBuf,
    _lock: File,
    directory: File,
}
impl Store {
    pub fn open() -> Result<Self, UpdateError> {
        let home = dirs::home_dir().ok_or(UpdateError::Storage)?;
        let base = dirs::data_local_dir()
            .filter(|p| p.starts_with(&home))
            .ok_or(UpdateError::Storage)?;
        Self::at(&base.join("org.adapt2move.nextcloud-modern-talk-updates"))
    }
    #[cfg(unix)]
    pub fn at(root: &Path) -> Result<Self, UpdateError> {
        use rustix::fs::{self as rfs, Mode, OFlags};
        use std::os::unix::fs::MetadataExt;
        let flags = OFlags::RDONLY | OFlags::DIRECTORY | OFlags::NOFOLLOW | OFlags::CLOEXEC;
        let mut directory =
            File::from(rfs::open("/", flags, Mode::empty()).map_err(|_| UpdateError::Storage)?);
        let parts: Vec<_> = root.components().collect();
        if !root.is_absolute() || parts.len() > 64 {
            return Err(UpdateError::Storage);
        }
        let uid = rustix::process::geteuid().as_raw();
        for (index, component) in parts.iter().enumerate() {
            let std::path::Component::Normal(part) = component else {
                if index == 0 {
                    continue;
                }
                return Err(UpdateError::Storage);
            };
            let file = match rfs::openat(&directory, *part, flags, Mode::empty()) {
                Ok(file) => file,
                Err(rustix::io::Errno::NOENT) => {
                    rfs::mkdirat(&directory, *part, Mode::from_raw_mode(0o700))
                        .map_err(|_| UpdateError::Storage)?;
                    rfs::openat(&directory, *part, flags, Mode::empty())
                        .map_err(|_| UpdateError::Storage)?
                }
                Err(_) => return Err(UpdateError::Storage),
            };
            directory = File::from(file);
            let meta = directory.metadata().map_err(|_| UpdateError::Storage)?;
            let last = index + 1 == parts.len();
            // A sticky system temp parent is used by isolated tests only; the final
            // directory must still be owner-only and opened without following links.
            let sticky_tmp = cfg!(test) && meta.uid() == 0 && meta.mode() & 0o1000 != 0;
            if !meta.is_dir()
                || (meta.uid() != 0 && meta.uid() != uid)
                || (!sticky_tmp && meta.mode() & 0o022 != 0)
                || (last && (meta.uid() != uid || meta.mode() & 0o7777 != 0o700))
            {
                return Err(UpdateError::Storage);
            }
            #[cfg(target_os = "linux")]
            if last {
                for name in ["system.posix_acl_access", "system.posix_acl_default"] {
                    let mut bytes = [0u8; 4096];
                    if !matches!(
                        rfs::fgetxattr(&directory, name, &mut bytes[..]),
                        Err(rustix::io::Errno::NODATA)
                    ) {
                        return Err(UpdateError::Storage);
                    }
                }
            }
        }
        #[cfg(target_os = "macos")]
        if exacl::getfacl(root, None)
            .map_err(|_| UpdateError::Storage)?
            .iter()
            .any(|e| e.allow && !e.perms.is_empty())
        {
            return Err(UpdateError::Storage);
        }
        let lock = File::from(
            rfs::openat(
                &directory,
                "owner.lock",
                OFlags::RDWR | OFlags::CREATE | OFlags::NOFOLLOW | OFlags::CLOEXEC,
                Mode::from_raw_mode(0o600),
            )
            .map_err(|_| UpdateError::Storage)?,
        );
        private_regular(&lock)?;
        lock.try_lock().map_err(|_| UpdateError::Busy)?;
        let store = Self {
            root: root.to_owned(),
            _lock: lock,
            directory,
        };
        // Staging has a single bounded slot. After a process crash a partial stage
        // never becomes Ready without a new API selection and verification.
        store.remove_payload()?;
        Ok(store)
    }
    #[cfg(not(unix))]
    pub fn at(_: &Path) -> Result<Self, UpdateError> {
        Err(UpdateError::UnsupportedInstall)
    }
    pub fn check(&self) -> Result<(), UpdateError> {
        let live = fs::symlink_metadata(&self.root).map_err(|_| UpdateError::Storage)?;
        let opened = self
            .directory
            .metadata()
            .map_err(|_| UpdateError::Storage)?;
        if !live.is_dir() || live.file_type().is_symlink() {
            return Err(UpdateError::Storage);
        }
        #[cfg(unix)]
        {
            use std::os::unix::fs::MetadataExt;
            if live.ino() != opened.ino()
                || live.dev() != opened.dev()
                || live.mode() & 0o7777 != 0o700
            {
                return Err(UpdateError::Storage);
            }
        }
        #[cfg(not(unix))]
        let _ = opened;
        Ok(())
    }
    pub fn payload(&self, kind: InstallKind) -> PathBuf {
        self.root.join(payload_name(kind))
    }
    pub fn remove_payload(&self) -> Result<(), UpdateError> {
        self.check()?;
        for kind in [
            InstallKind::Portable,
            InstallKind::MacBundle,
            InstallKind::Debian,
        ] {
            match fs::remove_file(self.payload(kind)) {
                Ok(()) => {}
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
                Err(_) => return Err(UpdateError::Storage),
            }
        }
        Ok(())
    }
    pub fn create_payload(&self, kind: InstallKind) -> Result<File, UpdateError> {
        self.remove_payload()?;
        #[cfg(unix)]
        {
            use rustix::fs::{self as rfs, Mode, OFlags};
            let file = File::from(
                rfs::openat(
                    &self.directory,
                    payload_name(kind),
                    OFlags::RDWR
                        | OFlags::CREATE
                        | OFlags::EXCL
                        | OFlags::NOFOLLOW
                        | OFlags::CLOEXEC,
                    Mode::from_raw_mode(0o600),
                )
                .map_err(|_| UpdateError::Storage)?,
            );
            private_regular(&file)?;
            Ok(file)
        }
        #[cfg(not(unix))]
        {
            let _ = kind;
            Err(UpdateError::UnsupportedInstall)
        }
    }
    pub fn schedule(&self) -> Result<Option<Schedule>, UpdateError> {
        self.check()?;
        let path = self.root.join("schedule.json");
        let file = match open_regular(&path) {
            Ok(file) => file,
            Err(UpdateError::Unavailable) => return Ok(None),
            Err(error) => return Err(error),
        };
        let mut bytes = Vec::new();
        file.take(257)
            .read_to_end(&mut bytes)
            .map_err(|_| UpdateError::Storage)?;
        Schedule::decode(&bytes).map(Some)
    }
    pub fn save_schedule(&self, schedule: Schedule) -> Result<(), UpdateError> {
        self.check()?;
        let mut temp =
            tempfile::NamedTempFile::new_in(&self.root).map_err(|_| UpdateError::Storage)?;
        temp.write_all(&schedule.encode()?)
            .map_err(|_| UpdateError::Storage)?;
        temp.as_file()
            .sync_all()
            .map_err(|_| UpdateError::Storage)?;
        temp.persist(self.root.join("schedule.json"))
            .map_err(|_| UpdateError::Storage)?;
        self.directory.sync_all().map_err(|_| UpdateError::Storage)
    }
}
pub(super) fn open_regular(path: &Path) -> Result<File, UpdateError> {
    #[cfg(unix)]
    {
        use rustix::fs::{self as rfs, Mode, OFlags};
        let file = File::from(
            rfs::open(
                path,
                OFlags::RDONLY | OFlags::NOFOLLOW | OFlags::CLOEXEC,
                Mode::empty(),
            )
            .map_err(|e| {
                if e == rustix::io::Errno::NOENT {
                    UpdateError::Unavailable
                } else {
                    UpdateError::Storage
                }
            })?,
        );
        private_regular(&file)?;
        Ok(file)
    }
    #[cfg(not(unix))]
    {
        let _ = path;
        Err(UpdateError::UnsupportedInstall)
    }
}
#[cfg(unix)]
fn private_regular(file: &File) -> Result<(), UpdateError> {
    let meta = file.metadata().map_err(|_| UpdateError::Storage)?;
    if !meta.is_file() {
        return Err(UpdateError::Storage);
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        if meta.uid() != rustix::process::geteuid().as_raw()
            || meta.nlink() != 1
            || meta.mode() & 0o7777 != 0o600
        {
            return Err(UpdateError::Storage);
        }
    }
    Ok(())
}

fn payload_name(kind: InstallKind) -> &'static str {
    match kind {
        InstallKind::Portable => "release.tar.gz",
        InstallKind::MacBundle => "release.zip",
        InstallKind::Debian => "release.deb",
    }
}
