//! Persistent directory identity. Unix device numbers identify a current mount;
//! on macOS they must not be persisted as a volume identity across reboots.
use super::super::error::{IdentityError, ProfileFailure, Result};

pub(in crate::identity) struct DirectoryBinding {
    pub current: String,
    pub legacy: Option<String>,
}

impl DirectoryBinding {
    pub fn needs_update(&self, stored: &str, repair: bool) -> Result<bool> {
        if stored == self.current {
            return Ok(false);
        }
        if let Some(legacy) = &self.legacy {
            if stored == legacy {
                return Ok(true);
            }
            if only_device_changed(stored, legacy) {
                return if repair {
                    Ok(true)
                } else {
                    Err(IdentityError::Profile(ProfileFailure::DeviceChanged))
                };
            }
        }
        Err(IdentityError::Profile(ProfileFailure::BindingMismatch))
    }
}

fn unix_parts(value: &str) -> Option<(u32, u64, u64)> {
    let mut parts = value.strip_prefix("unix:")?.split(':');
    let uid = parts.next()?.parse().ok()?;
    let device = parts.next()?.parse().ok()?;
    let inode = parts.next()?.parse().ok()?;
    if parts.next().is_some() || value != format!("unix:{uid}:{device}:{inode}") {
        return None;
    }
    Some((uid, device, inode))
}

fn only_device_changed(stored: &str, current: &str) -> bool {
    match (unix_parts(stored), unix_parts(current)) {
        (Some((old_uid, old_dev, old_ino)), Some((uid, dev, ino))) => {
            old_uid == uid && old_ino == ino && old_dev != dev
        }
        _ => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn mac() -> DirectoryBinding {
        DirectoryBinding {
            current: "macos:501:11111111-2222-3333-4444-555555555555:42".into(),
            legacy: Some("unix:501:123:42".into()),
        }
    }

    #[test]
    fn a_changed_mount_device_requires_explicit_recovery() {
        let binding = mac();
        assert_eq!(binding.needs_update(&binding.current, false), Ok(false));
        assert_eq!(binding.needs_update("unix:501:123:42", false), Ok(true));
        assert_eq!(
            binding.needs_update("unix:501:999:42", false),
            Err(IdentityError::Profile(ProfileFailure::DeviceChanged))
        );
        assert_eq!(binding.needs_update("unix:501:999:42", true), Ok(true));
    }

    #[test]
    fn recovery_cannot_rebind_another_user_directory_or_volume() {
        let binding = mac();
        for stored in [
            "unix:502:123:42",
            "unix:501:123:43",
            "unix:501:999:43",
            "unix:0501:123:42",
            "unix:501:123:42:extra",
            "unix:501:not-a-device:42",
            "macos:501:aaaaaaaa-bbbb-cccc-dddd-eeeeeeeeeeee:42",
        ] {
            assert_eq!(
                binding.needs_update(stored, true),
                Err(IdentityError::Profile(ProfileFailure::BindingMismatch)),
                "{stored}"
            );
        }
        let unix = DirectoryBinding {
            current: "unix:501:123:42".into(),
            legacy: None,
        };
        assert!(unix.needs_update("unix:501:999:42", true).is_err());
    }
}
