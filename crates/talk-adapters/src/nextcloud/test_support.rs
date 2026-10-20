//! Explicit, default-off synthetic fixture intake. This is not Login Flow or app storage.
use super::{AuthenticatedBinding, ServerOrigin};
use super::{NetworkRuntime, NextcloudClient, wire};
use std::path::Path;
use talk_core::ids::{AccountScope, HostId};
use talk_core::lifecycle::Cancellation;
use talk_core::ports::{ErrorKind, PortError, Retry};
use zeroize::Zeroizing;

/// Reads at most 16 KiB from an existing owner-private fixture file under a private
/// directory. Fields: origin, user, password. Only literal loopback origins are accepted.
/// The caller owns fixture lifecycle and must remove it before VM hibernation.
pub fn from_private_fixture(
    path: &Path,
    scope: AccountScope,
    host: HostId,
    runtime: NetworkRuntime,
) -> Result<NextcloudClient, PortError> {
    let failure = |kind| PortError {
        kind,
        account: Some(scope.account),
        retry: Retry::Never,
    };
    let bytes = private_file(path).map_err(failure)?;
    let mut data = wire::json(&bytes).map_err(failure)?;
    let origin = ServerOrigin::loopback(
        data["origin"]
            .as_str()
            .ok_or_else(|| failure(ErrorKind::Malformed))?,
    )
    .map_err(failure)?;
    let user = Zeroizing::new(
        data["user"]
            .as_str()
            .ok_or_else(|| failure(ErrorKind::Malformed))?
            .to_owned(),
    );
    // Move the only decoded secret String into a zeroizing owner rather than clone it.
    let password = match data.get_mut("password").map(serde_json::Value::take) {
        Some(serde_json::Value::String(value)) => Zeroizing::new(value),
        _ => return Err(failure(ErrorKind::Malformed)),
    };
    let mut client = NextcloudClient::from_identity(
        AuthenticatedBinding {
            scope,
            host,
            origin,
            login_name: user.clone(),
            user,
            password,
            cancellation: Cancellation::default(),
        },
        runtime,
    )?;
    if let Some(room) = data.get("room") {
        client.bind_room(room)?;
    }
    Ok(client)
}

#[cfg(unix)]
fn private_file(path: &Path) -> Result<Zeroizing<Vec<u8>>, ErrorKind> {
    use std::io::Read;
    use std::os::unix::fs::MetadataExt;
    let parent = path.parent().ok_or(ErrorKind::Malformed)?;
    let dir = std::fs::symlink_metadata(parent).map_err(|_| ErrorKind::PermissionDenied)?;
    let before = std::fs::symlink_metadata(path).map_err(|_| ErrorKind::PermissionDenied)?;
    let owner = std::fs::metadata("/proc/self")
        .map_err(|_| ErrorKind::Unsupported)?
        .uid();
    if !dir.is_dir()
        || dir.uid() != owner
        || dir.mode() & 0o777 != 0o700
        || !before.is_file()
        || before.uid() != owner
        || before.mode() & 0o777 != 0o600
        || before.len() > 16384
    {
        return Err(ErrorKind::PermissionDenied);
    }
    let file = std::fs::File::open(path).map_err(|_| ErrorKind::PermissionDenied)?;
    let opened = file.metadata().map_err(|_| ErrorKind::PermissionDenied)?;
    if before.ino() != opened.ino()
        || before.dev() != opened.dev()
        || opened.mode() & 0o777 != 0o600
        || opened.uid() != owner
    {
        return Err(ErrorKind::PermissionDenied);
    }
    let mut bytes = Zeroizing::new(Vec::new());
    file.take(16385)
        .read_to_end(&mut bytes)
        .map_err(|_| ErrorKind::Malformed)?;
    if bytes.len() > 16384 {
        return Err(ErrorKind::TooLarge);
    }
    Ok(bytes)
}
#[cfg(not(unix))]
fn private_file(_path: &Path) -> Result<Zeroizing<Vec<u8>>, ErrorKind> {
    Err(ErrorKind::Unsupported)
}

impl NextcloudClient {
    /// Inject the composition-owned service; normal account/room/policy admission still runs.
    pub fn fixture_media(&mut self, media: std::sync::Arc<crate::media::MediaService>) {
        self.set_media_service(Some(media));
    }
    /// Imports only the synthetic pre-known room from the private fixture intake.
    pub fn fixture_conversation(&self) -> Option<talk_core::ids::ConversationId> {
        self.rooms.first().map(|room| room.id)
    }
}
