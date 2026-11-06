//! Account-owned, bounded session cache for the native Files browser.
use super::*;

const CACHE_LIMIT: usize = 8;
const CACHE_LIFETIME: Duration = Duration::from_secs(10 * 60);

#[derive(Clone, Debug, Eq, PartialEq)]
pub(super) enum Query {
    Folder(Text<16384>),
    Recent,
    Favorites,
    Search(Text<256>),
}
impl Query {
    fn section(&self) -> view::FileSection {
        match self {
            Self::Folder(_) => view::FileSection::All,
            Self::Recent => view::FileSection::Recent,
            Self::Favorites => view::FileSection::Favorites,
            Self::Search(_) => view::FileSection::Search,
        }
    }
    fn command(&self) -> wire::ContentCommand {
        match self {
            Self::Folder(path) => wire::ContentCommand::ListFiles { path: path.clone() },
            Self::Recent => wire::ContentCommand::SearchFiles { query: None },
            Self::Favorites => wire::ContentCommand::FavoriteFiles,
            Self::Search(term) => wire::ContentCommand::SearchFiles {
                query: Some(term.clone()),
            },
        }
    }
    fn path(&self) -> Text<16384> {
        match self {
            Self::Folder(path) => path.clone(),
            _ => text("/"),
        }
    }
}

struct Cached {
    account: AccountScope,
    query: Query,
    entries: Page<view::CloudFileView, 1000>,
    saved: Instant,
}

#[derive(Default)]
pub(super) struct Owner {
    cache: Vec<Cached>,
    jobs: Vec<(OperationId, Query)>,
    current: Option<(AccountScope, RequestId, Query)>,
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
    pub(super) fn take_job(&mut self, operation: OperationId) -> Option<Query> {
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
        self.query_cloud_files(request, account, Query::Folder(path))
    }
    pub(super) fn query_cloud_files(
        &mut self,
        request: RequestId,
        account: AccountScope,
        query: Query,
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
            .find(|item| item.query == query)
            .map(|item| item.entries.clone());
        self.content_owner.browser.current = Some((account, request, query.clone()));
        self.snapshot.content_panel.file_browser = Some(view::FileBrowserView {
            account,
            path: query.path(),
            section: query.section(),
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
        match self.start_content(job, query.command()) {
            Ok(()) => {
                if let Some(job) = self.content_owner.jobs.last() {
                    self.content_owner.browser.jobs.push((job.operation, query));
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
        query: Query,
        result: Result<RuntimeReply, IdentityError>,
    ) {
        if self.content_owner.browser.current.as_ref() != Some(&(account, request, query.clone()))
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
            .any(|item| item.account == account && item.query == query);
        let Some(browser) = self
            .snapshot
            .content_panel
            .file_browser
            .as_mut()
            .filter(|view| {
                view.account == account
                    && view.path == query.path()
                    && view.section == query.section()
            })
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
                        path: file.path.clone(),
                        directory: file.directory,
                        size: file.size,
                        modified: file.modified.clone(),
                        mime: file.mime.clone(),
                        file_id: file.file_id.clone(),
                    })
                    .collect();
                if query != Query::Recent {
                    entries.sort_by(|a, b| {
                        b.directory.cmp(&a.directory).then_with(|| {
                            a.name
                                .as_str()
                                .to_lowercase()
                                .cmp(&b.name.as_str().to_lowercase())
                        })
                    });
                }
                browser.entries = page(entries);
                browser.offline = false;
                browser.error = None;
                self.content_owner
                    .browser
                    .cache
                    .retain(|item| item.query != query || item.account != account);
                if self.content_owner.browser.cache.len() >= CACHE_LIMIT {
                    self.content_owner.browser.cache.remove(0);
                }
                self.content_owner.browser.cache.push(Cached {
                    account,
                    query,
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
                            .retain(|item| item.query != query || item.account != account);
                    }
                    browser.error = Some(error_reason(error));
                }
            }
        }
        self.touch();
    }
}
