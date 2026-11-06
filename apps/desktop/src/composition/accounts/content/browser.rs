//! Account-owned, bounded session cache for the native Files browser.
use super::*;

const CACHE_LIMIT: usize = 8;
const CACHE_LIFETIME: Duration = Duration::from_secs(10 * 60);

#[derive(Clone, Debug, Eq, PartialEq)]
pub(super) enum Query {
    Folder(Text<16384>, u64),
    Recent,
    Favorites,
    Search(Text<256>),
}
impl Query {
    fn section(&self) -> view::FileSection {
        match self {
            Self::Folder(_, _) => view::FileSection::All,
            Self::Recent => view::FileSection::Recent,
            Self::Favorites => view::FileSection::Favorites,
            Self::Search(_) => view::FileSection::Search,
        }
    }
    fn command(&self) -> wire::ContentCommand {
        match self {
            Self::Folder(path, offset) => wire::ContentCommand::ListFilesPage {
                path: path.clone(),
                offset: *offset,
            },
            Self::Recent => wire::ContentCommand::SearchFiles { query: None },
            Self::Favorites => wire::ContentCommand::FavoriteFiles,
            Self::Search(term) => wire::ContentCommand::SearchFiles {
                query: Some(term.clone()),
            },
        }
    }
    fn path(&self) -> Text<16384> {
        match self {
            Self::Folder(path, _) => path.clone(),
            _ => text("/"),
        }
    }
}
fn sort_entries(entries: &mut [view::CloudFileView], sort: view::FileSort) {
    entries.sort_by(|a, b| {
        b.directory
            .cmp(&a.directory)
            .then_with(|| match sort {
                view::FileSort::Name => std::cmp::Ordering::Equal,
                view::FileSort::Size => b.size.cmp(&a.size),
                view::FileSort::Modified => b.modified_unix.cmp(&a.modified_unix),
            })
            .then_with(|| {
                a.name
                    .as_str()
                    .to_lowercase()
                    .cmp(&b.name.as_str().to_lowercase())
            })
    });
}

struct Cached {
    account: AccountScope,
    query: Query,
    entries: Page<view::CloudFileView, 1000>,
    pagination: Option<(u64, u64)>,
    saved: Instant,
}
struct Retry {
    account: AccountScope,
    request: RequestId,
    query: Query,
    attempts: u8,
    next: Instant,
    expires: Instant,
}

#[derive(Default)]
pub(super) struct Owner {
    cache: Vec<Cached>,
    jobs: Vec<(OperationId, Query, u8)>,
    current: Option<(AccountScope, RequestId, Query)>,
    retry: Option<Retry>,
    download: Option<(OperationId, AccountScope, RequestId, wire::TransferProgress)>,
    generation: u64,
    search_unsupported: Option<AccountScope>,
    favorites_unsupported: Option<AccountScope>,
}
impl Owner {
    pub(super) fn close(&mut self) {
        self.current = None;
        self.retry = None;
    }
    pub(super) fn retire_jobs(&mut self) {
        self.jobs.clear();
        self.current = None;
        self.retry = None;
        self.download = None;
    }
    pub(super) fn retain_account(&mut self, account: Option<AccountScope>) {
        self.cache.retain(|item| Some(item.account) == account);
        if self.search_unsupported != account {
            self.search_unsupported = None;
        }
        if self.favorites_unsupported != account {
            self.favorites_unsupported = None;
        }
    }
    pub(super) fn take_job(&mut self, operation: OperationId) -> Option<(Query, u8)> {
        let index = self.jobs.iter().position(|(id, _, _)| *id == operation)?;
        let (_, query, attempts) = self.jobs.remove(index);
        Some((query, attempts))
    }
    pub(super) fn is_download(&self, operation: OperationId) -> bool {
        self.download
            .as_ref()
            .is_some_and(|(id, ..)| *id == operation)
    }
}

impl Accounts {
    pub(super) fn queue_browser_download(
        &mut self,
        request: RequestId,
        account: AccountScope,
        path: Text<16384>,
    ) -> Result<(), SubmitError> {
        let context = self
            .snapshot
            .selected
            .filter(|context| context.account == account)
            .ok_or(SubmitError::StaleScope)?;
        let entry = self
            .snapshot
            .content_panel
            .file_browser
            .as_ref()
            .filter(|browser| browser.account == account && !browser.offline)
            .and_then(|browser| {
                browser
                    .entries
                    .as_slice()
                    .iter()
                    .find(|entry| entry.path == path && !entry.directory)
            })
            .ok_or(SubmitError::StaleScope)?;
        let name = entry.name.clone();
        self.queue_picker(
            request,
            context,
            super::super::files::Kind::CloudDownload { path, name },
        )
    }
    pub(in crate::composition::accounts) fn start_browser_download(
        &mut self,
        request: RequestId,
        account: AccountScope,
        path: Text<16384>,
        name: Text<255>,
        destination: Text<16384>,
    ) -> Result<(), SubmitError> {
        if !std::path::Path::new(destination.as_str()).is_absolute()
            || self.content_owner.transfer.is_some()
            || self.content_owner.browser.download.is_some()
            || !self
                .snapshot
                .content_panel
                .file_browser
                .as_ref()
                .is_some_and(|browser| {
                    browser.account == account
                        && !browser.offline
                        && browser.entries.as_slice().iter().any(|entry| {
                            entry.path == path && entry.name == name && !entry.directory
                        })
                })
        {
            return Err(SubmitError::StaleScope);
        }
        let progress = wire::TransferProgress::default();
        self.start_content(
            Job {
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
            },
            wire::ContentCommand::DownloadCloudFile {
                path,
                destination,
                progress: progress.clone(),
            },
        )?;
        let Some(job) = self.content_owner.jobs.last() else {
            return Err(SubmitError::Busy);
        };
        self.content_owner.browser.download =
            Some((job.operation, account, request, progress.clone()));
        self.content_owner.transfer = Some(progress);
        self.snapshot.content_panel.browser_download = Some(view::BrowserDownloadView {
            request,
            name,
            received: 0,
            total: None,
            state: view::DownloadState::Transferring,
        });
        self.touch();
        Ok(())
    }
    pub(super) fn cancel_browser_download(&mut self) {
        let Some((operation, _, request, progress)) = &self.content_owner.browser.download else {
            return;
        };
        progress.cancel();
        if let Ok(connection) = &mut self.connection {
            for pending in &self.pending {
                if matches!(pending.purpose, Purpose::Content(id) if id == *operation) {
                    let _ = connection.cancel_request(pending.id);
                }
            }
        }
        if let Some(view) = self
            .snapshot
            .content_panel
            .browser_download
            .as_mut()
            .filter(|view| view.request == *request)
        {
            view.state = if progress.is_published() {
                view::DownloadState::Saved
            } else {
                view::DownloadState::Cancelled
            };
        }
        self.touch();
    }
    pub(super) fn complete_browser_download(
        &mut self,
        account: AccountScope,
        request: RequestId,
        operation: OperationId,
        result: Result<RuntimeReply, IdentityError>,
    ) {
        let Some((actual, scope, expected, progress)) = self.content_owner.browser.download.take()
        else {
            return;
        };
        if actual != operation || scope != account || expected != request {
            return;
        }
        self.content_owner.transfer = None;
        if self.snapshot.active_account != Some(account) {
            return;
        }
        if let Some(view) = self
            .snapshot
            .content_panel
            .browser_download
            .as_mut()
            .filter(|view| view.request == request)
        {
            let (received, total, _) = progress.snapshot();
            view.received = received;
            view.total = total;
            view.state = if progress.is_published() {
                view::DownloadState::Saved
            } else if matches!(view.state, view::DownloadState::Cancelled) {
                view::DownloadState::Cancelled
            } else {
                match result {
                    Ok(RuntimeReply::Protocol(ProtocolReply::Content {
                        operation: returned,
                        result: wire::ContentResult::CloudFileSaved(_),
                    })) if returned == operation => view::DownloadState::Saved,
                    Err(error) => view::DownloadState::Failed(error_reason(error)),
                    _ => view::DownloadState::Failed(UnavailableReason::PolicyStale),
                }
            };
        }
        self.touch();
    }
    pub(super) fn open_browser_file(
        &mut self,
        request: RequestId,
        account: AccountScope,
        path: Text<16384>,
    ) -> Result<(), SubmitError> {
        let (file, generation) = self
            .snapshot
            .content_panel
            .file_browser
            .as_ref()
            .filter(|browser| browser.account == account)
            .and_then(|browser| {
                browser
                    .entries
                    .as_slice()
                    .iter()
                    .find(|entry| entry.path == path)
            })
            .and_then(|entry| entry.file_id.as_ref())
            .and_then(|id| id.as_str().parse::<u64>().ok())
            .filter(|id| *id > 0)
            .zip(
                self.snapshot
                    .content_panel
                    .file_browser
                    .as_ref()
                    .map(|browser| browser.generation),
            )
            .ok_or(SubmitError::StaleScope)?;
        self.start_content(
            Job {
                request,
                account,
                context: None,
                operation: OperationId::from_local(0),
                target: Some(view::AssetTarget::CloudFile {
                    account,
                    path,
                    file,
                    generation,
                }),
                poll: None,
                mutation: false,
                cloud: true,
                attachment: None,
                preview_generation: None,
                avatar_actor: None,
            },
            wire::ContentCommand::OpenCloudFile(file),
        )
    }
    pub(super) fn list_cloud_files(
        &mut self,
        request: RequestId,
        account: AccountScope,
        path: Text<16384>,
    ) -> Result<(), SubmitError> {
        self.query_cloud_files(request, account, Query::Folder(path, 0))
    }
    pub(super) fn page_cloud_files(
        &mut self,
        request: RequestId,
        account: AccountScope,
        path: Text<16384>,
        offset: u64,
    ) -> Result<(), SubmitError> {
        let current = self
            .snapshot
            .content_panel
            .file_browser
            .as_ref()
            .filter(|browser| {
                browser.account == account && browser.path == path && !browser.loading
            })
            .and_then(|browser| browser.pagination)
            .ok_or(SubmitError::StaleScope)?;
        if offset != current.0.saturating_sub(40) && offset != current.0.saturating_add(40)
            || offset >= current.1
        {
            return Err(SubmitError::StaleScope);
        }
        self.query_cloud_files(request, account, Query::Folder(path, offset))
    }
    pub(super) fn query_cloud_files(
        &mut self,
        request: RequestId,
        account: AccountScope,
        query: Query,
    ) -> Result<(), SubmitError> {
        self.content_owner.browser.retry = None;
        self.start_cloud_query(request, account, query, 0)
    }
    fn start_cloud_query(
        &mut self,
        request: RequestId,
        account: AccountScope,
        query: Query,
        attempts: u8,
    ) -> Result<(), SubmitError> {
        if matches!(query, Query::Search(_) | Query::Recent)
            && self.content_owner.browser.search_unsupported == Some(account)
            || matches!(query, Query::Favorites)
                && self.content_owner.browser.favorites_unsupported == Some(account)
            || matches!(query, Query::Folder(_, _))
                && self.snapshot.content_panel.file_browser_supported == Some(false)
        {
            return Err(SubmitError::Unavailable(UnavailableReason::Unsupported));
        }
        let now = Instant::now();
        self.content_owner.browser.cache.retain(|item| {
            item.account == account && now.saturating_duration_since(item.saved) < CACHE_LIFETIME
        });
        let sort = if query == Query::Recent {
            view::FileSort::Modified
        } else {
            view::FileSort::Name
        };
        let cached = self
            .content_owner
            .browser
            .cache
            .iter()
            .find(|item| item.query == query)
            .map(|item| {
                let mut entries = item.entries.as_slice().to_vec();
                sort_entries(&mut entries, sort);
                (page(entries), item.pagination)
            });
        self.content_owner.browser.current = Some((account, request, query.clone()));
        self.content_owner.browser.generation =
            self.content_owner.browser.generation.saturating_add(1);
        self.snapshot.content_panel.file_browser = Some(view::FileBrowserView {
            account,
            path: query.path(),
            section: query.section(),
            sort,
            entries: cached
                .as_ref()
                .map_or_else(Page::empty, |(entries, _)| entries.clone()),
            generation: self.content_owner.browser.generation,
            pagination: cached.as_ref().and_then(|(_, pagination)| *pagination),
            search_available: self.content_owner.browser.search_unsupported != Some(account),
            favorites_available: self.content_owner.browser.favorites_unsupported != Some(account),
            loading: true,
            offline: false,
            error: None,
        });
        self.retire_cloud_previews();
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
                    self.content_owner
                        .browser
                        .jobs
                        .push((job.operation, query, attempts));
                }
            }
            Err(error) => {
                if matches!(error, SubmitError::Busy) && attempts < 3 {
                    self.content_owner.browser.current = None;
                    self.content_owner.browser.retry = Some(Retry {
                        account,
                        request,
                        query,
                        attempts: attempts + 1,
                        next: Instant::now() + Duration::from_secs(1),
                        expires: Instant::now() + Duration::from_secs(60),
                    });
                    self.touch();
                    return Ok(());
                }
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
        attempts: u8,
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
        let result = match result {
            Ok(RuntimeReply::Protocol(ProtocolReply::Content {
                operation: actual,
                result: wire::ContentResult::FilePage { files, pagination },
            })) if actual == operation && matches!(query, Query::Folder(_, _)) => {
                Ok((files, pagination))
            }
            Ok(RuntimeReply::Protocol(ProtocolReply::Content {
                operation: actual,
                result: wire::ContentResult::Files(files),
            })) if actual == operation && !matches!(query, Query::Folder(_, _)) => {
                Ok((files, None))
            }
            Err(error) => Err(error),
            _ => Err(IdentityError::Request(ErrorKind::Malformed)),
        };
        let mut retire_previews = false;
        match result {
            Ok((files, pagination)) => {
                if matches!(query, Query::Folder(_, _)) {
                    self.snapshot.content_panel.file_browser_supported = Some(true);
                }
                let mut entries: Vec<_> = files
                    .as_slice()
                    .iter()
                    .map(|file| view::CloudFileView {
                        name: file.name.clone(),
                        path: file.path.clone(),
                        directory: file.directory,
                        size: file.size,
                        modified: file.modified.clone(),
                        modified_unix: file.modified_unix,
                        mime: file.mime.clone(),
                        file_id: file.file_id.clone(),
                    })
                    .collect();
                sort_entries(&mut entries, browser.sort);
                self.content_owner.browser.generation =
                    self.content_owner.browser.generation.saturating_add(1);
                browser.generation = self.content_owner.browser.generation;
                browser.entries = page(entries);
                browser.pagination = pagination;
                browser.offline = false;
                browser.error = None;
                retire_previews = true;
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
                    pagination,
                    saved: Instant::now(),
                });
            }
            Err(error) => {
                if matches!(error, IdentityError::Request(ErrorKind::Unsupported)) {
                    match query {
                        Query::Folder(_, _) => {
                            self.snapshot.content_panel.file_browser_supported = Some(false)
                        }
                        Query::Recent | Query::Search(_) => {
                            self.content_owner.browser.search_unsupported = Some(account);
                            browser.search_available = false;
                        }
                        Query::Favorites => {
                            self.content_owner.browser.favorites_unsupported = Some(account);
                            browser.favorites_available = false;
                        }
                    }
                }
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
                if transient && attempts < 3 {
                    browser.loading = true;
                    browser.offline = has_cache;
                    self.content_owner.browser.retry = Some(Retry {
                        account,
                        request,
                        query,
                        attempts: attempts + 1,
                        next: Instant::now() + Duration::from_secs(1u64 << attempts),
                        expires: Instant::now() + Duration::from_secs(60),
                    });
                    self.touch();
                    return;
                }
                if transient && has_cache {
                    browser.offline = true;
                } else {
                    if !transient {
                        browser.entries = Page::empty();
                        retire_previews = true;
                        self.content_owner
                            .browser
                            .cache
                            .retain(|item| item.query != query || item.account != account);
                    }
                    browser.error = Some(error_reason(error));
                }
            }
        }
        if retire_previews {
            self.retire_cloud_previews();
        }
        self.touch();
    }
    pub(super) fn retry_cloud_files(&mut self) {
        let now = Instant::now();
        self.content_owner
            .browser
            .cache
            .retain(|item| now.saturating_duration_since(item.saved) < CACHE_LIFETIME);
        if !self
            .content_owner
            .browser
            .retry
            .as_ref()
            .is_some_and(|retry| retry.next <= now)
        {
            return;
        }
        let Some(retry) = self.content_owner.browser.retry.take() else {
            return;
        };
        if now >= retry.expires
            || self.snapshot.active_account != Some(retry.account)
            || self
                .snapshot
                .content_panel
                .file_browser
                .as_ref()
                .is_none_or(|browser| {
                    browser.account != retry.account
                        || browser.section != retry.query.section()
                        || browser.path != retry.query.path()
                })
        {
            return;
        }
        let _ = self.start_cloud_query(retry.request, retry.account, retry.query, retry.attempts);
    }
    pub(super) fn sort_cloud_files(&mut self, account: AccountScope, sort: view::FileSort) {
        let Some(browser) = self
            .snapshot
            .content_panel
            .file_browser
            .as_mut()
            .filter(|browser| browser.account == account)
        else {
            return;
        };
        if browser.sort == sort {
            return;
        }
        let mut entries = browser.entries.as_slice().to_vec();
        sort_entries(&mut entries, sort);
        browser.entries = page(entries);
        browser.sort = sort;
        self.touch();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn entry(
        name: &str,
        directory: bool,
        size: Option<u64>,
        modified_unix: Option<i64>,
    ) -> view::CloudFileView {
        view::CloudFileView {
            name: text(name),
            path: text(&format!("/{name}")),
            directory,
            size,
            modified: None,
            modified_unix,
            mime: None,
            file_id: None,
        }
    }
    #[test]
    fn sorting_keeps_directories_first_and_unknown_values_last() {
        let mut entries = vec![
            entry("small", false, Some(2), Some(3)),
            entry("folder", true, None, None),
            entry("unknown", false, None, None),
            entry("large", false, Some(9), Some(7)),
        ];
        sort_entries(&mut entries, view::FileSort::Size);
        assert_eq!(
            entries.iter().map(|e| e.name.as_str()).collect::<Vec<_>>(),
            ["folder", "large", "small", "unknown"]
        );
        sort_entries(&mut entries, view::FileSort::Modified);
        assert_eq!(
            entries.iter().map(|e| e.name.as_str()).collect::<Vec<_>>(),
            ["folder", "large", "small", "unknown"]
        );
        sort_entries(&mut entries, view::FileSort::Name);
        assert_eq!(
            entries.iter().map(|e| e.name.as_str()).collect::<Vec<_>>(),
            ["folder", "large", "small", "unknown"]
        );
    }
    #[test]
    fn closing_retires_query_authority_without_reusing_an_old_completion() {
        let mut owner = Owner::default();
        let operation = OperationId::from_local(7);
        owner
            .jobs
            .push((operation, Query::Folder(text("/old"), 0), 0));
        owner.current = Some((
            crate::composition::fixtures::scope(),
            RequestId(2),
            Query::Folder(text("/old"), 0),
        ));
        owner.close();
        assert!(owner.current.is_none());
        assert_eq!(
            owner.take_job(operation),
            Some((Query::Folder(text("/old"), 0), 0))
        );
        assert!(owner.take_job(operation).is_none());
    }
}
