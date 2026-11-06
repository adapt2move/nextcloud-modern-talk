//! A finite DAV lane using the same origin, cookie, cancellation and global I/O owner.
use super::*;
use tokio::io::{AsyncWrite, AsyncWriteExt};
const FILE_LIMIT: Duration = Duration::from_secs(60);
fn xml_escape(input: &str) -> String {
    let mut escaped = String::new();
    for ch in input.chars() {
        match ch {
            '&' => escaped.push_str("&amp;"),
            '<' => escaped.push_str("&lt;"),
            '>' => escaped.push_str("&gt;"),
            '"' => escaped.push_str("&quot;"),
            '\'' => escaped.push_str("&apos;"),
            ch => escaped.push(ch),
        }
    }
    escaped
}
pub(super) enum StreamRead<'a> {
    File(Option<&'a (dyn Fn(u64, Option<u64>) + Send + Sync)>),
    Dav(DavRead),
    Image,
    Pdf,
    Video(crate::content::VideoFormat),
}
pub(super) enum DavRead {
    Listing(Option<DavPageRequest>),
    Search(String),
    Favorites,
}
pub(crate) struct DavPageRequest {
    pub offset: u64,
    pub token: Option<String>,
}
pub(crate) struct DavPageResponse {
    pub total: Option<u64>,
    pub token: Option<String>,
}
impl Transport {
    pub(super) fn file_endpoint(
        &self,
        path: &crate::files::FilePath,
    ) -> Result<url::Url, PortError> {
        self.dav_endpoint(path.as_str())
    }
    fn dav_endpoint(&self, path: &str) -> Result<url::Url, PortError> {
        let mut url = self
            .binding
            .origin
            .endpoint("remote.php/dav/files/")
            .map_err(|kind| self.error(kind))?;
        {
            let mut parts = url
                .path_segments_mut()
                .map_err(|_| self.error(ErrorKind::Malformed))?;
            parts.pop_if_empty().push(self.binding.user.as_str());
            for segment in path
                .split('/')
                .skip(1)
                .filter(|segment| !segment.is_empty())
            {
                parts.push(segment);
            }
        }
        Ok(url)
    }
    pub(crate) async fn directory_listing(
        &mut self,
        c: &OperationContext,
        path: &crate::files::DirectoryPath,
    ) -> Result<(url::Url, Vec<u8>), PortError> {
        self.request_dispatched = false;
        let url = self.dav_endpoint(path.as_str())?;
        let mut sink = Vec::new();
        self.stream(
            c,
            url.clone(),
            None,
            &mut sink,
            StreamRead::Dav(DavRead::Listing(None)),
        )
        .await?;
        Ok((url, sink))
    }
    pub(crate) async fn directory_listing_page(
        &mut self,
        c: &OperationContext,
        path: &crate::files::DirectoryPath,
        request: DavPageRequest,
    ) -> Result<(url::Url, Vec<u8>, Option<DavPageResponse>), PortError> {
        self.request_dispatched = false;
        let url = self.dav_endpoint(path.as_str())?;
        let mut sink = Vec::new();
        let (_, page) = self
            .stream(
                c,
                url.clone(),
                None,
                &mut sink,
                StreamRead::Dav(DavRead::Listing(Some(request))),
            )
            .await?;
        Ok((url, sink, page))
    }
    pub(crate) async fn search_files(
        &mut self,
        c: &OperationContext,
        term: Option<&str>,
    ) -> Result<(url::Url, Vec<u8>), PortError> {
        self.request_dispatched = false;
        let root = self.dav_endpoint("/")?;
        let user = root
            .path_segments()
            .and_then(|mut parts| parts.next_back())
            .ok_or_else(|| self.error(ErrorKind::Malformed))?;
        let condition = if let Some(term) = term {
            if term.is_empty() || term.len() > 256 || term.chars().any(char::is_control) {
                return Err(self.error(ErrorKind::Malformed));
            }
            format!(
                "<d:like><d:prop><d:displayname/></d:prop><d:literal>%{}%</d:literal></d:like>",
                xml_escape(term)
            )
        } else {
            "<d:not><d:is-collection/></d:not>".to_owned()
        };
        let body = format!(
            "<?xml version=\"1.0\"?><d:searchrequest xmlns:d=\"DAV:\" xmlns:oc=\"http://owncloud.org/ns\"><d:basicsearch><d:select><d:prop><d:displayname/><d:resourcetype/><d:getcontentlength/><d:getlastmodified/><d:getcontenttype/><oc:fileid/></d:prop></d:select><d:from><d:scope><d:href>/files/{}</d:href><d:depth>infinity</d:depth></d:scope></d:from><d:where>{condition}</d:where><d:orderby><d:order><d:prop><d:getlastmodified/></d:prop><d:descending/></d:order></d:orderby><d:limit><d:nresults>200</d:nresults></d:limit></d:basicsearch></d:searchrequest>",
            xml_escape(user)
        );
        let endpoint = self
            .binding
            .origin
            .endpoint("remote.php/dav/")
            .map_err(|kind| self.error(kind))?;
        let mut sink = Vec::new();
        self.stream(
            c,
            endpoint,
            None,
            &mut sink,
            StreamRead::Dav(DavRead::Search(body)),
        )
        .await?;
        Ok((root, sink))
    }
    pub(crate) async fn favorite_files(
        &mut self,
        c: &OperationContext,
    ) -> Result<(url::Url, Vec<u8>), PortError> {
        self.request_dispatched = false;
        let root = self.dav_endpoint("/")?;
        let mut sink = Vec::new();
        self.stream(
            c,
            root.clone(),
            None,
            &mut sink,
            StreamRead::Dav(DavRead::Favorites),
        )
        .await?;
        Ok((root, sink))
    }
    pub(crate) async fn file<W: AsyncWrite + Unpin + Send>(
        &mut self,
        c: &OperationContext,
        path: crate::files::FilePath,
        upload: Option<(reqwest::Body, u64)>,
        sink: &mut W,
    ) -> Result<u64, PortError> {
        // Endpoint construction is local admission for this operation too.
        self.request_dispatched = false;
        let url = self.file_endpoint(&path)?;
        self.stream(c, url, upload, sink, StreamRead::File(None))
            .await
            .map(|(length, _)| length)
    }
    pub(crate) async fn file_progress<W: AsyncWrite + Unpin + Send>(
        &mut self,
        c: &OperationContext,
        path: crate::files::FilePath,
        sink: &mut W,
        progress: &(dyn Fn(u64, Option<u64>) + Send + Sync),
    ) -> Result<u64, PortError> {
        self.request_dispatched = false;
        let url = self.file_endpoint(&path)?;
        self.stream(c, url, None, sink, StreamRead::File(Some(progress)))
            .await
            .map(|(length, _)| length)
    }
    pub(super) async fn stream<W: AsyncWrite + Unpin + Send>(
        &mut self,
        c: &OperationContext,
        url: url::Url,
        upload: Option<(reqwest::Body, u64)>,
        sink: &mut W,
        read: StreamRead<'_>,
    ) -> Result<(u64, Option<DavPageResponse>), PortError> {
        self.request_dispatched = false;
        let listing = matches!(&read, StreamRead::Dav(_));
        let pdf = matches!(read, StreamRead::Pdf);
        let video = match &read {
            StreamRead::Video(format) => Some(*format),
            _ => None,
        };
        let (read_limit, progress, image) = match &read {
            StreamRead::File(progress) => (crate::files::MAX_FILE_BYTES, *progress, false),
            StreamRead::Dav(_) => (BODY_LIMIT as u64, None, false),
            StreamRead::Image => (crate::content::MAX_IMAGE_BYTES as u64, None, true),
            StreamRead::Pdf => (crate::content::MAX_IMAGE_BYTES as u64, None, false),
            StreamRead::Video(_) => (crate::content::MAX_VIDEO_BYTES as u64, None, false),
        };
        self.validate(c)?;
        let shared = self.begin_shared_cookies()?;
        let _permit = self
            .runtime
            .0
            .requests
            .clone()
            .try_acquire_owned()
            .map_err(|_| self.error(ErrorKind::Busy))?;
        let writing = upload.is_some();
        let mut request = self
            .client
            .request(
                if let StreamRead::Dav(dav) = &read {
                    Method::from_bytes(match dav {
                        DavRead::Listing(_) => b"PROPFIND",
                        DavRead::Search(_) => b"SEARCH",
                        DavRead::Favorites => b"REPORT",
                    })
                    .map_err(|_| self.error(ErrorKind::Malformed))?
                } else if writing {
                    Method::PUT
                } else {
                    Method::GET
                },
                url.clone(),
            )
            .timeout(FILE_LIMIT)
            .header(ACCEPT_ENCODING, "identity")
            .header(CACHE_CONTROL, "no-store")
            .basic_auth(
                self.binding.login_name.as_str(),
                Some(self.binding.password.as_str()),
            );
        if let StreamRead::Dav(dav) = &read {
            let body = match dav {
                DavRead::Listing(_) => {
                    "<?xml version=\"1.0\"?><d:propfind xmlns:d=\"DAV:\" xmlns:oc=\"http://owncloud.org/ns\"><d:prop><d:displayname/><d:resourcetype/><d:getcontentlength/><d:getlastmodified/><d:getcontenttype/><oc:fileid/></d:prop></d:propfind>"
                }
                DavRead::Search(body) => body.as_str(),
                DavRead::Favorites => {
                    "<?xml version=\"1.0\"?><oc:filter-files xmlns:d=\"DAV:\" xmlns:oc=\"http://owncloud.org/ns\"><d:prop><d:displayname/><d:resourcetype/><d:getcontentlength/><d:getlastmodified/><d:getcontenttype/><oc:fileid/></d:prop><oc:filter-rules><oc:favorite>1</oc:favorite></oc:filter-rules></oc:filter-files>"
                }
            };
            request = request
                .header(CONTENT_TYPE, "application/xml; charset=utf-8")
                .body(body.to_owned());
            if matches!(dav, DavRead::Listing(_)) {
                request = request.header("Depth", "1");
            }
            if let DavRead::Listing(Some(page)) = dav {
                request = request.header("X-NC-Paginate", "true").header(
                    "X-NC-Paginate-Count",
                    crate::files::DIRECTORY_PAGE_SIZE.to_string(),
                );
                if let Some(token) = &page.token {
                    request = request
                        .header("X-NC-Paginate-Token", token)
                        .header("X-NC-Paginate-Offset", page.offset.to_string());
                }
            }
        }
        self.cookies
            .retain(|cookie| cookie.expires.is_none_or(|time| time > Instant::now()));
        let mut cookies = Zeroizing::new(String::new());
        for cookie in &self.cookies {
            if (!cookie.secure || url.scheme() == "https")
                && cookie_path_matches(url.path(), &cookie.path)
            {
                if !cookies.is_empty() {
                    cookies.push_str("; ")
                }
                cookies.push_str(&cookie.name);
                cookies.push('=');
                cookies.push_str(&cookie.value);
            }
        }
        if !cookies.is_empty() {
            let mut value =
                HeaderValue::from_str(&cookies).map_err(|_| self.error(ErrorKind::Malformed))?;
            value.set_sensitive(true);
            request = request.header(COOKIE, value)
        }
        if let Some((body, size)) = upload {
            if size > crate::files::MAX_FILE_BYTES {
                return Err(self.error(ErrorKind::TooLarge));
            }
            request = request
                .header("If-None-Match", "*")
                .header(CONTENT_TYPE, "application/octet-stream")
                .header(reqwest::header::CONTENT_LENGTH, size)
                .body(body)
        }
        let request = request
            .build()
            .map_err(|_| self.error(ErrorKind::Malformed))?;
        let deadline = Duration::from_millis(
            c.deadline
                .millis
                .checked_sub(self.now().millis)
                .ok_or_else(|| self.error(ErrorKind::Deadline))?,
        )
        .min(FILE_LIMIT);
        let account_cancel = self.binding.cancellation.clone();
        let work = async {
            // Only polled work can dispatch; cancellation may win before this point.
            self.request_dispatched = true;
            let mut response = self
                .client
                .execute(request)
                .await
                .map_err(|_| self.error(ErrorKind::Transport))?;
            if response.url().origin() != self.binding.origin.0.origin() {
                return Err(self.error(ErrorKind::WrongScope));
            }
            let status = response.status().as_u16();
            if (300..400).contains(&status) {
                return Err(self.error(ErrorKind::RedirectRejected));
            }
            if response
                .headers()
                .get_all(CONTENT_ENCODING)
                .iter()
                .any(|h| h.as_bytes() != b"identity")
            {
                return Err(self.error(ErrorKind::Unsupported));
            }
            match status {
                201 if writing => {}
                207 if listing => {}
                200 if !writing && !listing => {}
                401 | 403 => return Err(self.error(ErrorKind::PermissionDenied)),
                404 => return Err(self.error(ErrorKind::NotFound)),
                409 | 412 => return Err(self.error(ErrorKind::Conflict)),
                405 | 501 if listing => return Err(self.error(ErrorKind::Unsupported)),
                413 => return Err(self.error(ErrorKind::TooLarge)),
                429 => return Err(self.error(ErrorKind::RateLimited)),
                _ => return Err(self.error(ErrorKind::ServerUnavailable)),
            }
            // The opaque token stays in the authenticated client, never in the UI.
            // Later pages need not repeat the total or token in their response.
            let page = if matches!(&read, StreamRead::Dav(DavRead::Listing(Some(_))))
                && response
                    .headers()
                    .get("X-NC-Paginate")
                    .and_then(|v| v.to_str().ok())
                    == Some("true")
            {
                let total = response
                    .headers()
                    .get("X-NC-Paginate-Total")
                    .map(|v| {
                        v.to_str()
                            .ok()
                            .and_then(|v| v.parse::<u64>().ok())
                            .filter(|n| *n > 0 && *n <= 1_000_000_000)
                            .ok_or_else(|| self.error(ErrorKind::Malformed))
                    })
                    .transpose()?;
                let token = response
                    .headers()
                    .get("X-NC-Paginate-Token")
                    .map(|v| {
                        v.to_str()
                            .ok()
                            .filter(|v| {
                                !v.is_empty()
                                    && v.len() <= 256
                                    && v.bytes().all(|b| (33..=126).contains(&b))
                            })
                            .map(str::to_owned)
                            .ok_or_else(|| self.error(ErrorKind::Malformed))
                    })
                    .transpose()?;
                Some(DavPageResponse { total, token })
            } else {
                None
            };
            let limit = if writing {
                BODY_LIMIT as u64
            } else {
                read_limit
            };
            if response.content_length().is_some_and(|n| n > limit) {
                return Err(self.error(ErrorKind::TooLarge));
            }
            if image || pdf || video.is_some() {
                let mime = response
                    .headers()
                    .get(CONTENT_TYPE)
                    .and_then(|v| v.to_str().ok())
                    .and_then(|v| v.split(';').next())
                    .unwrap_or("");
                let accepted = if let Some(format) = video {
                    mime == format.mime_type()
                } else if pdf {
                    mime == "application/pdf"
                } else {
                    matches!(
                        mime,
                        "image/png" | "image/jpeg" | "image/webp" | "image/gif" | "image/svg+xml"
                    )
                };
                if !accepted {
                    return Err(self.error(ErrorKind::Unsupported));
                }
            }
            let total = response.content_length();
            if let Some(progress) = progress {
                progress(0, total);
            }
            let mut length = 0u64;
            while let Some(chunk) = response
                .chunk()
                .await
                .map_err(|_| self.error(ErrorKind::Transport))?
            {
                if chunk.len() as u64 > limit - length {
                    return Err(self.error(ErrorKind::TooLarge));
                }
                length += chunk.len() as u64;
                if !writing {
                    for block in chunk.chunks(crate::files::FILE_CHUNK_BYTES) {
                        sink.write_all(block)
                            .await
                            .map_err(|_| self.error(ErrorKind::Transport))?
                    }
                }
                if let Some(progress) = progress {
                    progress(length, total);
                }
            }
            sink.flush()
                .await
                .map_err(|_| self.error(ErrorKind::Transport))?;
            let cookies = self.update_cookies(response.headers())?;
            Ok((length, cookies, page))
        };
        let cancel = async {
            loop {
                if account_cancel.is_cancelled() || c.cancellation.is_cancelled() {
                    break;
                }
                tokio::time::sleep(Duration::from_millis(25)).await;
            }
        };
        let result = tokio::select! {biased;_=cancel=>Err(PortError{kind:ErrorKind::Cancelled,account:Some(c.scope.account),retry:Retry::Never}),r=tokio::time::timeout(deadline,work)=>r.unwrap_or(Err(PortError{kind:ErrorKind::Deadline,account:Some(c.scope.account),retry:Retry::UserAction}))};
        self.validate(c)?;
        match result {
            Ok((length, cookies, page)) => {
                self.cookies = cookies;
                self.commit_shared_cookies(shared)?;
                Ok((length, page))
            }
            Err(error) => Err(error),
        }
    }
}

#[cfg(test)]
mod tests;
