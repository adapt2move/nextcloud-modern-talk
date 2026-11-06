//! Account-owned, bounded session cache for the native Files browser.
use super::*;

const CACHE_LIMIT: usize = 8;
const CACHE_LIFETIME: Duration = Duration::from_secs(10 * 60);

struct Cached {
    account: AccountScope,
    path: Text<16384>,
    entries: Page<view::CloudFileView, 1000>,
    saved: Instant,
}

#[derive(Default)]
pub(super) struct Owner {
    cache: Vec<Cached>,
    jobs: Vec<(OperationId, Text<16384>)>,
    current: Option<(AccountScope, RequestId, Text<16384>)>,
}
impl Owner {
    pub(super) fn close(&mut self) {
        self.current = None;
    }
    pub(super) fn retire_jobs(&mut self) {
        self.jobs.clear();
        self.current = None;
    }
    pub(super) fn retain_account(&mut self, account: Option<AccountScope>) {
        self.cache.retain(|item| Some(item.account) == account);
    }
    pub(super) fn take_job(&mut self, operation: OperationId) -> Option<Text<16384>> {
        let index = self.jobs.iter().position(|(id, _)| *id == operation)?;
        Some(self.jobs.remove(index).1)
    }
}

impl Accounts {
    pub(super) fn list_cloud_files(
        &mut self,
        request: RequestId,
        account: AccountScope,
        path: Text<16384>,
    ) -> Result<(), SubmitError> {
        let now = Instant::now();
        self.content_owner.browser.cache.retain(|item| {
            item.account == account && now.saturating_duration_since(item.saved) < CACHE_LIFETIME
        });
        let cached = self
            .content_owner
            .browser
            .cache
            .iter()
            .find(|item| item.path == path)
            .map(|item| item.entries.clone());
        self.content_owner.browser.current = Some((account, request, path.clone()));
        self.snapshot.content_panel.file_browser = Some(view::FileBrowserView {
            account,
            path: path.clone(),
            entries: cached.clone().unwrap_or_else(Page::empty),
            loading: true,
            offline: false,
            error: None,
        });
        let job = Job {
            request,
            account,
            context: None,
            operation: OperationId::from_local(0),
            target: None,
            poll: None,
            mutation: false,
            cloud: false,
            attachment: None,
            preview_generation: None,
            avatar_actor: None,
        };
        match self.start_content(job, wire::ContentCommand::ListFiles { path: path.clone() }) {
            Ok(()) => {
                if let Some(job) = self.content_owner.jobs.last() {
                    self.content_owner.browser.jobs.push((job.operation, path));
                }
            }
            Err(error) => {
                if let Some(browser) = &mut self.snapshot.content_panel.file_browser {
                    browser.loading = false;
                    browser.offline = cached.is_some();
                    browser.error = cached.is_none().then_some(UnavailableReason::Busy);
                }
                self.content_owner.browser.current = None;
                self.touch();
                return if matches!(error, SubmitError::Busy) {
                    Ok(())
                } else {
                    Err(error)
                };
            }
        }
        self.touch();
        Ok(())
    }

    pub(super) fn complete_cloud_files(
        &mut self,
        account: AccountScope,
        request: RequestId,
        operation: OperationId,
        path: Text<16384>,
        result: Result<RuntimeReply, IdentityError>,
    ) {
        if self.content_owner.browser.current.as_ref() != Some(&(account, request, path.clone()))
            || self.snapshot.active_account != Some(account)
            || self.current(account).is_err()
        {
            return;
        }
        self.content_owner.browser.current = None;
        let has_cache = self
            .content_owner
            .browser
            .cache
            .iter()
            .any(|item| item.account == account && item.path == path);
        let Some(browser) = self
            .snapshot
            .content_panel
            .file_browser
            .as_mut()
            .filter(|view| view.account == account && view.path == path)
        else {
            return;
        };
        browser.loading = false;
        match result {
            Ok(RuntimeReply::Protocol(ProtocolReply::Content {
                operation: actual,
                result: wire::ContentResult::Files(files),
            })) if actual == operation => {
                let mut entries: Vec<_> = files
                    .as_slice()
                    .iter()
                    .map(|file| view::CloudFileView {
                        name: file.name.clone(),
                        directory: file.directory,
                        size: file.size,
                        modified: file.modified.clone(),
                        mime: file.mime.clone(),
                        file_id: file.file_id.clone(),
                    })
                    .collect();
                entries.sort_by(|a, b| {
                    b.directory.cmp(&a.directory).then_with(|| {
                        a.name
                            .as_str()
                            .to_lowercase()
                            .cmp(&b.name.as_str().to_lowercase())
                    })
                });
                browser.entries = page(entries);
                browser.offline = false;
                browser.error = None;
                self.content_owner
                    .browser
                    .cache
                    .retain(|item| item.path != path || item.account != account);
                if self.content_owner.browser.cache.len() >= CACHE_LIMIT {
                    self.content_owner.browser.cache.remove(0);
                }
                self.content_owner.browser.cache.push(Cached {
                    account,
                    path,
                    entries: browser.entries.clone(),
                    saved: Instant::now(),
                });
            }
            other => {
                let error = match other {
                    Err(error) => error,
                    Ok(_) => IdentityError::Request(ErrorKind::Malformed),
                };
                let transient = matches!(
                    error,
                    IdentityError::Request(
                        ErrorKind::Transport
                            | ErrorKind::Deadline
                            | ErrorKind::ServerUnavailable
                            | ErrorKind::RateLimited
                            | ErrorKind::Busy
                    )
                );
                if transient && has_cache {
                    browser.offline = true;
                } else {
                    if !transient {
                        browser.entries = Page::empty();
                        self.content_owner
                            .browser
                            .cache
                            .retain(|item| item.path != path || item.account != account);
                    }
                    browser.error = Some(error_reason(error));
                }
            }
        }
        self.touch();
    }
}
