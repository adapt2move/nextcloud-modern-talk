use super::super::error::{self, Result};
use rustix::fd::OwnedFd;
use rustix::fs::{self, AtFlags, Mode, OFlags};
use std::fs::File;
use std::io::{Read, Write};
use std::os::unix::fs::MetadataExt;
use std::path::{Component, Path, PathBuf};
use zeroize::Zeroizing;

pub(in crate::identity) struct PrivateDir {
    directory: File,
    path: PathBuf,
    uid: u32,
}
fn os(e: rustix::io::Errno) -> super::super::error::IdentityError {
    error::io(e.into())
}
const DIRECTORY: OFlags = OFlags::RDONLY
    .union(OFlags::DIRECTORY)
    .union(OFlags::NOFOLLOW)
    .union(OFlags::CLOEXEC);
impl PrivateDir {
    pub(in crate::identity) fn open(path: &Path, fixture: bool) -> Result<Self> {
        if !path.is_absolute()
            || path
                .components()
                .any(|c| matches!(c, Component::ParentDir | Component::CurDir))
        {
            return Err(error::denied());
        }
        let uid = rustix::process::geteuid().as_raw();
        let mut current = File::from(fs::open("/", DIRECTORY, Mode::empty()).map_err(os)?);
        let parts: Vec<_> = path
            .components()
            .filter_map(|c| {
                if let Component::Normal(n) = c {
                    Some(n)
                } else {
                    None
                }
            })
            .collect();
        if parts.len() < 2 || parts.len() > 64 {
            return Err(error::denied());
        }
        let mut walked = PathBuf::from("/");
        for (index, part) in parts.iter().enumerate() {
            let final_part = index + 1 == parts.len();
            let mut created = false;
            let child = match fs::openat(&current, *part, DIRECTORY, Mode::empty()) {
                Ok(fd) => fd,
                Err(rustix::io::Errno::NOENT) => {
                    fs::mkdirat(&current, *part, Mode::from_raw_mode(0o700)).map_err(os)?;
                    fs::fsync(&current).map_err(os)?;
                    created = true;
                    fs::openat(&current, *part, DIRECTORY, Mode::empty()).map_err(os)?
                }
                Err(e) => return Err(os(e)),
            };
            let child = File::from(child);
            let meta = child.metadata().map_err(error::io)?;
            walked.push(part);
            let fixture_tmp = fixture && matches!(walked.to_str(), Some("/tmp" | "/run"));
            if !meta.is_dir()
                || (meta.uid() != uid && meta.uid() != 0)
                || (!fixture_tmp && meta.mode() & 0o022 != 0)
            {
                return Err(error::denied());
            }
            if (final_part || created) && (meta.uid() != uid || meta.mode() & 0o7777 != 0o700) {
                return Err(error::denied());
            }
            // Existing private nodes must have no inherited/default extra ACL grants.
            Self::acl(&child, &walked, final_part || created)?;
            current = child;
        }
        Ok(Self {
            directory: current,
            path: path.to_owned(),
            uid,
        })
    }
    fn acl(file: &File, path: &Path, private: bool) -> Result<()> {
        #[cfg(target_os = "linux")]
        {
            let _ = path;
            for name in ["system.posix_acl_access", "system.posix_acl_default"] {
                let mut bytes = [0u8; 4096];
                match fs::fgetxattr(file, name, &mut bytes[..]) {
                    Ok(_) if private => return Err(error::denied()),
                    Ok(_) => {}
                    Err(rustix::io::Errno::NODATA) => {}
                    Err(e) => return Err(os(e)),
                }
            }
        }
        #[cfg(target_os = "macos")]
        {
            let _ = file;
            // Parent chain is held open and rejects other-user replacement; same-user
            // hostile code is outside this provider's protection boundary.
            let entries = exacl::getfacl(path, None).map_err(error::io)?;
            let mutation = exacl::Perm::WRITE
                | exacl::Perm::APPEND
                | exacl::Perm::DELETE
                | exacl::Perm::DELETE_CHILD
                | exacl::Perm::WRITEATTR
                | exacl::Perm::WRITEEXTATTR
                | exacl::Perm::WRITESECURITY
                | exacl::Perm::CHOWN;
            if entries.iter().any(|e| {
                e.allow
                    && if private {
                        !e.perms.is_empty()
                    } else {
                        e.perms.intersects(mutation)
                    }
            }) {
                return Err(error::denied());
            }
        }
        Ok(())
    }
    pub(in crate::identity) fn path(&self) -> &Path {
        &self.path
    }
    pub(in crate::identity) fn binding(&self) -> Result<String> {
        let m = self.directory.metadata().map_err(error::io)?;
        Ok(format!("unix:{}:{}:{}", self.uid, m.dev(), m.ino()))
    }
    pub(in crate::identity) fn check(&self) -> Result<()> {
        let m = self.directory.metadata().map_err(error::io)?;
        let live = std::fs::symlink_metadata(&self.path).map_err(error::io)?;
        if m.uid() != self.uid
            || m.mode() & 0o7777 != 0o700
            || !live.is_dir()
            || live.dev() != m.dev()
            || live.ino() != m.ino()
        {
            return Err(error::denied());
        }
        Self::acl(&self.directory, &self.path, true)
    }
    fn name(name: &str) -> Result<()> {
        if name.is_empty()
            || name.len() > 160
            || !name
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || b"-_.".contains(&b))
            || matches!(name, "." | "..")
        {
            return Err(error::denied());
        }
        Ok(())
    }
    fn validate(&self, file: &File, name: &str) -> Result<()> {
        let m = file.metadata().map_err(error::io)?;
        if !m.is_file() || m.uid() != self.uid || m.mode() & 0o7777 != 0o600 || m.nlink() != 1 {
            return Err(error::denied());
        }
        Self::acl(file, &self.path.join(name), true)
    }
    fn open_file(&self, name: &str, flags: OFlags) -> Result<File> {
        Self::name(name)?;
        self.check()?;
        let fd: OwnedFd = fs::openat(
            &self.directory,
            name,
            flags | OFlags::NOFOLLOW | OFlags::CLOEXEC | OFlags::NONBLOCK,
            Mode::from_raw_mode(0o600),
        )
        .map_err(os)?;
        let file = File::from(fd);
        self.validate(&file, name)?;
        Ok(file)
    }
    pub(in crate::identity) fn lock(&self) -> Result<File> {
        let file = self.open_file("profile.lock", OFlags::RDWR | OFlags::CREATE)?;
        file.try_lock().map_err(|_| {
            error::storage(
                talk_core::identity::StorageBackend::PrivateFile,
                talk_core::identity::StorageFailure::Conflict,
            )
        })?;
        Ok(file)
    }
    pub(in crate::identity) fn read(&self, name: &str, limit: usize) -> Result<Zeroizing<Vec<u8>>> {
        let mut f = self.open_file(name, OFlags::RDONLY)?;
        if f.metadata().map_err(error::io)?.len() > limit as u64 {
            return Err(error::corrupt());
        }
        let mut out = Zeroizing::new(Vec::with_capacity(limit.min(16384)));
        (&mut f)
            .take(limit as u64 + 1)
            .read_to_end(&mut out)
            .map_err(error::io)?;
        self.validate(&f, name)?;
        if out.len() > limit {
            return Err(error::corrupt());
        }
        Ok(out)
    }
    pub(in crate::identity) fn prepare_database(&self, name: &str) -> Result<()> {
        self.open_file(name, OFlags::RDWR | OFlags::CREATE)?;
        for suffix in ["-journal", "-wal", "-shm"] {
            let n = format!("{name}{suffix}");
            if self.path.join(&n).symlink_metadata().is_ok() {
                self.open_file(&n, OFlags::RDWR)?;
            }
        }
        Ok(())
    }
    pub(in crate::identity) fn create(&self, name: &str, bytes: &[u8]) -> Result<()> {
        if bytes.len() > 16384 {
            return Err(error::corrupt());
        }
        let stage = format!("{name}.stage");
        let mut f = self.open_file(&stage, OFlags::WRONLY | OFlags::CREATE | OFlags::EXCL)?;
        f.write_all(bytes).map_err(error::io)?;
        Self::flush(&f)?;
        self.validate(&f, &stage)?;
        // NOREPLACE is available on both selected Unix targets. No link/copy fallback.
        fs::renameat_with(
            &self.directory,
            &stage,
            &self.directory,
            name,
            fs::RenameFlags::NOREPLACE,
        )
        .map_err(os)?;
        fs::fsync(&self.directory).map_err(os)?;
        self.open_file(name, OFlags::RDONLY)?;
        Ok(())
    }
    fn flush(f: &File) -> Result<()> {
        fs::fsync(f).map_err(os)?;
        #[cfg(target_os = "macos")]
        fs::fcntl_fullfsync(f).map_err(os)?;
        Ok(())
    }
    pub(in crate::identity) fn remove(&self, name: &str) -> Result<()> {
        match self.open_file(name, OFlags::RDONLY) {
            Ok(_) => {}
            Err(super::super::error::IdentityError::Storage {
                failure: talk_core::identity::StorageFailure::Missing,
                ..
            }) => return Ok(()),
            Err(e) => return Err(e),
        }
        fs::unlinkat(&self.directory, name, AtFlags::empty()).map_err(os)?;
        fs::fsync(&self.directory).map_err(os)?;
        match fs::statat(&self.directory, name, AtFlags::SYMLINK_NOFOLLOW) {
            Err(rustix::io::Errno::NOENT) => Ok(()),
            _ => Err(error::denied()),
        }
    }
}
