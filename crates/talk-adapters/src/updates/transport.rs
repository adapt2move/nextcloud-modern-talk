//! Fixed public repository, exact package name and SHA-256 digest; no account client.
use super::{ReleaseInfo, UpdateChannel, UpdateError, install::InstallKind};
use reqwest::{Client, StatusCode};
use semver::Version;
use sha2::{Digest, Sha256};
use std::{
    fs::File,
    io::Write,
    time::{Duration, Instant},
};
use talk_core::bounded::Text;

const API: &str =
    "https://api.github.com/repos/adapt2move/nextcloud-modern-talk/releases?per_page=100";
const DOWNLOADS: &str = "https://github.com/adapt2move/nextcloud-modern-talk/releases/download/";
pub(super) const MAX_BODY: usize = 2 * 1024 * 1024;
pub(super) const MAX_DOWNLOAD: u64 = 512 * 1024 * 1024;
const DOWNLOAD_DEADLINE: Duration = Duration::from_secs(20 * 60);

#[derive(Clone, Debug, Eq, PartialEq)]
pub(super) struct Selection {
    pub info: ReleaseInfo,
    pub name: String,
    pub url: String,
    pub sha256: String,
}
pub(super) fn client() -> Result<Client, UpdateError> {
    Client::builder()
        .https_only(true)
        .referer(false)
        .redirect(reqwest::redirect::Policy::custom(|attempt| {
            let url = attempt.url();
            // Only the official public release object CDN receives the fixed public URL.
            if attempt.previous().len() < 4
                && url.scheme() == "https"
                && url.username().is_empty()
                && url.password().is_none()
                && url.port().is_none()
                && matches!(
                    url.host_str(),
                    Some(
                        "github.com"
                            | "release-assets.githubusercontent.com"
                            | "objects.githubusercontent.com"
                    )
                )
            {
                attempt.follow()
            } else {
                attempt.error("untrusted update redirect")
            }
        }))
        .connect_timeout(Duration::from_secs(8))
        .read_timeout(Duration::from_secs(30))
        .user_agent("NextCloud-Modern-Talk-updater")
        .build()
        .map_err(|_| UpdateError::Unavailable)
}
pub(super) async fn check(
    client: &Client,
    current: &Version,
    channel: UpdateChannel,
    kind: InstallKind,
) -> Result<Option<Selection>, UpdateError> {
    let mut response = client
        .get(API)
        .header("Accept", "application/vnd.github+json")
        .header("X-GitHub-Api-Version", "2022-11-28")
        .timeout(Duration::from_secs(20))
        .send()
        .await
        .map_err(|_| UpdateError::Network)?;
    status(response.status())?;
    if response
        .content_length()
        .is_some_and(|n| n > MAX_BODY as u64)
    {
        return Err(UpdateError::InvalidResponse);
    }
    let mut body = Vec::new();
    while let Some(chunk) = response.chunk().await.map_err(|_| UpdateError::Network)? {
        if chunk.len() > MAX_BODY - body.len() {
            return Err(UpdateError::InvalidResponse);
        }
        body.extend_from_slice(&chunk);
    }
    select(&body, current, channel, kind)
}
fn status(status: StatusCode) -> Result<(), UpdateError> {
    match status {
        StatusCode::OK => Ok(()),
        StatusCode::FORBIDDEN | StatusCode::TOO_MANY_REQUESTS => Err(UpdateError::RateLimited),
        StatusCode::NOT_FOUND => Err(UpdateError::Unavailable),
        _ => Err(UpdateError::Network),
    }
}
pub(super) fn select(
    bytes: &[u8],
    current: &Version,
    channel: UpdateChannel,
    kind: InstallKind,
) -> Result<Option<Selection>, UpdateError> {
    if bytes.len() > MAX_BODY {
        return Err(UpdateError::InvalidResponse);
    }
    let decoded: serde_json::Value =
        serde_json::from_slice(bytes).map_err(|_| UpdateError::InvalidResponse)?;
    let releases = decoded
        .as_array()
        .filter(|v| v.len() <= 100)
        .ok_or(UpdateError::InvalidResponse)?;
    let mut selected: Option<(Version, &serde_json::Value, bool)> = None;
    for release in releases {
        if release["draft"]
            .as_bool()
            .ok_or(UpdateError::InvalidResponse)?
        {
            continue;
        }
        let prerelease = release["prerelease"]
            .as_bool()
            .ok_or(UpdateError::InvalidResponse)?;
        let tag = release["tag_name"]
            .as_str()
            .ok_or(UpdateError::InvalidResponse)?;
        let Some(version) = parsed_tag(tag) else {
            continue;
        };
        let prerelease = prerelease || !version.pre.is_empty();
        if channel == UpdateChannel::Stable && prerelease {
            continue;
        }
        if version.cmp_precedence(current).is_gt()
            && selected
                .as_ref()
                .is_none_or(|(old, _, _)| version.cmp_precedence(old).is_gt())
        {
            selected = Some((version, release, prerelease));
        }
    }
    let Some((version, release, prerelease)) = selected else {
        return Ok(None);
    };
    // Never silently choose an older release when the newest selected release lacks a
    // matching, verifiable package. A changed package layout must be an explicit error.
    let version_text = version.to_string();
    let name = kind.asset_name(&version_text)?;
    let assets = release["assets"]
        .as_array()
        .filter(|v| v.len() <= 100)
        .ok_or(UpdateError::InvalidResponse)?;
    let mut matching = assets
        .iter()
        .filter(|asset| asset["name"].as_str() == Some(name.as_str()));
    let asset = matching.next().ok_or(UpdateError::Unavailable)?;
    if matching.next().is_some() {
        return Err(UpdateError::InvalidResponse);
    }
    let size = asset["size"]
        .as_u64()
        .filter(|n| *n > 0)
        .ok_or(UpdateError::InvalidResponse)?;
    if size > MAX_DOWNLOAD {
        return Err(UpdateError::TooLarge);
    }
    let sha256 = asset["digest"]
        .as_str()
        .and_then(|s| s.strip_prefix("sha256:"))
        .filter(|s| valid_digest(s))
        .ok_or(UpdateError::Integrity)?
        .to_owned();
    let tag = release["tag_name"]
        .as_str()
        .ok_or(UpdateError::InvalidResponse)?;
    let url = format!("{DOWNLOADS}{tag}/{name}");
    if asset["browser_download_url"].as_str() != Some(url.as_str()) {
        return Err(UpdateError::InvalidResponse);
    }
    Ok(Some(Selection {
        info: ReleaseInfo {
            version: Text::new(&version_text).map_err(|_| UpdateError::InvalidResponse)?,
            prerelease,
            bytes: size,
        },
        name,
        url,
        sha256,
    }))
}
pub(super) fn parsed_tag(tag: &str) -> Option<Version> {
    if tag.len() > 64
        || !tag
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'.' | b'-'))
    {
        return None;
    }
    let version = Version::parse(tag.strip_prefix('v').unwrap_or(tag)).ok()?;
    version.build.is_empty().then_some(version)
}
pub(super) fn valid_digest(value: &str) -> bool {
    value.len() == 64
        && value
            .bytes()
            .all(|c| c.is_ascii_digit() || (b'a'..=b'f').contains(&c))
}
pub(super) async fn download(
    client: &Client,
    selection: &Selection,
    file: &mut File,
    mut progress: impl FnMut(u64),
) -> Result<(), UpdateError> {
    let mut response = client
        .get(&selection.url)
        .header("Accept", "application/octet-stream")
        .timeout(DOWNLOAD_DEADLINE)
        .send()
        .await
        .map_err(|_| UpdateError::Network)?;
    status(response.status())?;
    if response
        .content_length()
        .is_some_and(|n| n != selection.info.bytes)
    {
        return Err(UpdateError::Integrity);
    }
    let mut received = 0u64;
    let mut digest = Sha256::new();
    let mut last = Instant::now();
    while let Some(chunk) = response.chunk().await.map_err(|_| UpdateError::Network)? {
        received = checked_download_size(received, chunk.len(), selection.info.bytes)?;
        file.write_all(&chunk).map_err(|_| UpdateError::Storage)?;
        digest.update(&chunk);
        if last.elapsed() >= Duration::from_millis(250) {
            progress(received);
            last = Instant::now();
        }
    }
    if received != selection.info.bytes || super::digest_hex(digest.finalize()) != selection.sha256
    {
        return Err(UpdateError::Integrity);
    }
    file.sync_all().map_err(|_| UpdateError::Storage)?;
    progress(received);
    Ok(())
}
pub(super) fn checked_download_size(
    received: u64,
    chunk: usize,
    expected: u64,
) -> Result<u64, UpdateError> {
    let total = received
        .checked_add(chunk as u64)
        .ok_or(UpdateError::TooLarge)?;
    if total > expected || total > MAX_DOWNLOAD {
        Err(UpdateError::TooLarge)
    } else {
        Ok(total)
    }
}
