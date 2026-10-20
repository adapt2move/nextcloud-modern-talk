//! Explicit public-release checks; no account credentials, background polling or installer.
//!
//! One worker owns one operation and one fixed-size snapshot. The response is capped
//! before JSON decoding; only this repository's version tag becomes an OS browser URL.
use reqwest::{Client, StatusCode};
use semver::Version;
use std::{
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
        mpsc::{self, SyncSender},
    },
    thread::{self, JoinHandle},
    time::Duration,
};
use talk_core::{
    bounded::Text,
    lifecycle::Cancellation,
    preferences::{
        UpdateChannel,
        updates::{ReleaseInfo, UpdateError, UpdateSnapshot, UpdateState},
    },
};

const API: &str =
    "https://api.github.com/repos/adapt2move/nextcloud-modern-talk/releases?per_page=100";
const RELEASES: &str = "https://github.com/adapt2move/nextcloud-modern-talk/releases/tag/";
const MAX_BODY: usize = 256 * 1024;
const DEADLINE: Duration = Duration::from_secs(12);

struct State {
    snapshot: UpdateSnapshot,
    tag: Option<String>,
}
struct Shared {
    state: Mutex<State>,
    busy: AtomicBool,
    stop: AtomicBool,
    stopped: AtomicBool,
    cancellation: Cancellation,
}
enum Command {
    Check(UpdateChannel),
    Open(String),
    Stop,
}
pub struct UpdateRuntime {
    shared: Arc<Shared>,
    sender: SyncSender<Command>,
    worker: Option<JoinHandle<()>>,
}
impl UpdateRuntime {
    /// Starting is offline. Only `check` may contact the public GitHub API.
    pub fn start(current_version: &str) -> Result<Self, UpdateError> {
        let current = Version::parse(current_version).map_err(|_| UpdateError::InvalidResponse)?;
        let shared = Arc::new(Shared {
            state: Mutex::new(State {
                snapshot: UpdateSnapshot {
                    revision: 1,
                    current_version: Text::new(current_version)
                        .map_err(|_| UpdateError::InvalidResponse)?,
                    channel: UpdateChannel::Stable,
                    state: UpdateState::Idle,
                },
                tag: None,
            }),
            busy: AtomicBool::new(false),
            stop: AtomicBool::new(false),
            stopped: AtomicBool::new(false),
            cancellation: Cancellation::default(),
        });
        let (sender, receiver) = mpsc::sync_channel(1);
        let owner = shared.clone();
        let worker = thread::Builder::new()
            .name("talk-updates".into())
            .spawn(move || {
                let runtime = tokio::runtime::Builder::new_current_thread()
                    .enable_all()
                    .build();
                if let Ok(runtime) = runtime {
                    // The worker sleeps on recv when idle, with no polling wakeups.
                    while !owner.stop.load(Ordering::Acquire) {
                        let Ok(command) = receiver.recv() else { break };
                        if owner.stop.load(Ordering::Acquire) || matches!(command, Command::Stop) {
                            break;
                        }
                        match command {
                            Command::Check(channel) => {
                                let result = runtime
                                    .block_on(interruptible(&owner, check(&current, channel)));
                                let mut state =
                                    owner.state.lock().unwrap_or_else(|e| e.into_inner());
                                state.tag = None;
                                state.snapshot.state = match result {
                                    Ok(Some((tag, info))) => {
                                        state.tag = Some(tag);
                                        UpdateState::Available(info)
                                    }
                                    Ok(None) => UpdateState::Current,
                                    Err(error) => UpdateState::Failed(error),
                                };
                                advance(&mut state.snapshot, &owner);
                            }
                            Command::Open(tag) => {
                                let result = runtime.block_on(async {
                                    let url = release_url(&tag)?;
                                    // The browser owner must finish its own child kill/reap;
                                    // dropping its future is not cancellation on macOS/Windows.
                                    super::identity::browser::open(&url, &owner.cancellation)
                                        .await
                                        .map_err(|_| UpdateError::BrowserUnavailable)
                                });
                                if let Err(error) = result {
                                    let mut state =
                                        owner.state.lock().unwrap_or_else(|e| e.into_inner());
                                    state.snapshot.state = UpdateState::Failed(error);
                                    advance(&mut state.snapshot, &owner);
                                }
                            }
                            Command::Stop => break,
                        }
                        owner.busy.store(false, Ordering::Release);
                    }
                    runtime.shutdown_timeout(Duration::from_secs(1));
                }
                let mut state = owner.state.lock().unwrap_or_else(|e| e.into_inner());
                state.snapshot.state = UpdateState::Stopped;
                advance(&mut state.snapshot, &owner);
                owner.busy.store(false, Ordering::Release);
                owner.stopped.store(true, Ordering::Release);
            })
            .map_err(|_| UpdateError::Unavailable)?;
        Ok(Self {
            shared,
            sender,
            worker: Some(worker),
        })
    }
    pub fn snapshot(&self) -> UpdateSnapshot {
        self.shared
            .state
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .snapshot
            .clone()
    }
    pub fn poll(&self, after: u64) -> Option<(u64, UpdateSnapshot)> {
        let snapshot = self.snapshot();
        (snapshot.revision != after).then_some((snapshot.revision, snapshot))
    }
    pub fn check(&self, channel: UpdateChannel) -> Result<(), UpdateError> {
        self.admit()?;
        {
            let mut state = self.shared.state.lock().unwrap_or_else(|e| e.into_inner());
            state.snapshot.channel = channel;
            state.snapshot.state = UpdateState::Checking;
            state.tag = None;
            advance(&mut state.snapshot, &self.shared);
        }
        self.send(Command::Check(channel))
    }
    pub fn open_release(&self) -> Result<(), UpdateError> {
        self.admit()?;
        let tag = self
            .shared
            .state
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .tag
            .clone();
        match tag {
            Some(tag) => self.send(Command::Open(tag)),
            None => {
                self.shared.busy.store(false, Ordering::Release);
                Err(UpdateError::Unavailable)
            }
        }
    }
    fn admit(&self) -> Result<(), UpdateError> {
        if self.shared.stop.load(Ordering::Acquire) || self.is_stopped() {
            return Err(UpdateError::Stopped);
        }
        self.shared
            .busy
            .compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire)
            .map(|_| ())
            .map_err(|_| UpdateError::Busy)
    }
    fn send(&self, command: Command) -> Result<(), UpdateError> {
        self.sender.try_send(command).map_err(|_| {
            self.shared.busy.store(false, Ordering::Release);
            UpdateError::Stopped
        })
    }
    pub fn request_shutdown(&self) {
        self.shared.stop.store(true, Ordering::Release);
        self.shared.cancellation.cancel();
        let _ = self.sender.try_send(Command::Stop);
    }
    pub fn is_stopped(&self) -> bool {
        self.shared.stopped.load(Ordering::Acquire)
    }
}
impl Drop for UpdateRuntime {
    fn drop(&mut self) {
        self.request_shutdown();
        // Composition requests and observes shutdown before disposing its services.
        // Joining here also owns cleanup if a constructor or startup path exits early.
        if let Some(worker) = self.worker.take() {
            let _ = worker.join();
        }
    }
}
fn advance(snapshot: &mut UpdateSnapshot, shared: &Shared) {
    if let Some(next) = snapshot.revision.checked_add(1) {
        snapshot.revision = next;
    } else {
        shared.stop.store(true, Ordering::Release);
    }
}
async fn interruptible<T>(
    shared: &Shared,
    work: impl std::future::Future<Output = Result<T, UpdateError>>,
) -> Result<T, UpdateError> {
    let cancelled = async {
        loop {
            if shared.stop.load(Ordering::Acquire) {
                break;
            }
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    };
    tokio::select! {
        biased;
        _ = cancelled => Err(UpdateError::Stopped),
        result = tokio::time::timeout(DEADLINE, work) => result.unwrap_or(Err(UpdateError::Network)),
    }
}
async fn check(
    current: &Version,
    channel: UpdateChannel,
) -> Result<Option<(String, ReleaseInfo)>, UpdateError> {
    let client = Client::builder()
        .https_only(true)
        .redirect(reqwest::redirect::Policy::none())
        .referer(false)
        .connect_timeout(Duration::from_secs(5))
        .timeout(DEADLINE)
        .user_agent("NextCloud-Modern-Talk-update-check")
        .build()
        .map_err(|_| UpdateError::Unavailable)?;
    let mut response = client
        .get(API)
        .header("Accept", "application/vnd.github+json")
        .header("X-GitHub-Api-Version", "2022-11-28")
        .send()
        .await
        .map_err(|_| UpdateError::Network)?;
    match response.status() {
        StatusCode::NOT_FOUND => return Err(UpdateError::Unavailable),
        StatusCode::FORBIDDEN | StatusCode::TOO_MANY_REQUESTS => {
            return Err(UpdateError::RateLimited);
        }
        StatusCode::OK => {}
        _ => return Err(UpdateError::Network),
    }
    if response
        .content_length()
        .is_some_and(|length| length > MAX_BODY as u64)
    {
        return Err(UpdateError::InvalidResponse);
    }
    let mut bytes = Vec::new();
    while let Some(chunk) = response.chunk().await.map_err(|_| UpdateError::Network)? {
        if chunk.len() > MAX_BODY - bytes.len() {
            return Err(UpdateError::InvalidResponse);
        }
        bytes.extend_from_slice(&chunk);
    }
    select_release(&bytes, current, channel)
}
fn select_release(
    bytes: &[u8],
    current: &Version,
    channel: UpdateChannel,
) -> Result<Option<(String, ReleaseInfo)>, UpdateError> {
    if bytes.len() > MAX_BODY {
        return Err(UpdateError::InvalidResponse);
    }
    let decoded: serde_json::Value =
        serde_json::from_slice(bytes).map_err(|_| UpdateError::InvalidResponse)?;
    let releases = decoded
        .as_array()
        .filter(|items| items.len() <= 100)
        .ok_or(UpdateError::InvalidResponse)?;
    let mut selected: Option<(Version, String, bool)> = None;
    for release in releases {
        let draft = release
            .get("draft")
            .and_then(|v| v.as_bool())
            .ok_or(UpdateError::InvalidResponse)?;
        if draft {
            continue;
        }
        let prerelease = release
            .get("prerelease")
            .and_then(|v| v.as_bool())
            .ok_or(UpdateError::InvalidResponse)?;
        let tag = release
            .get("tag_name")
            .and_then(|v| v.as_str())
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
            selected = Some((version, tag.to_owned(), prerelease));
        }
    }
    selected
        .map(|(version, tag, prerelease)| {
            Ok((
                tag,
                ReleaseInfo {
                    version: Text::new(&version.to_string())
                        .map_err(|_| UpdateError::InvalidResponse)?,
                    prerelease,
                },
            ))
        })
        .transpose()
}
fn parsed_tag(tag: &str) -> Option<Version> {
    if tag.len() > 64
        || !tag
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'.' | b'-' | b'+'))
    {
        return None;
    }
    Version::parse(tag.strip_prefix('v').unwrap_or(tag)).ok()
}
fn release_url(tag: &str) -> Result<String, UpdateError> {
    parsed_tag(tag).ok_or(UpdateError::InvalidResponse)?;
    let mut url = url::Url::parse(RELEASES).map_err(|_| UpdateError::InvalidResponse)?;
    url.path_segments_mut()
        .map_err(|_| UpdateError::InvalidResponse)?
        .pop_if_empty()
        .push(tag);
    Ok(url.into())
}

#[cfg(test)]
mod tests;
