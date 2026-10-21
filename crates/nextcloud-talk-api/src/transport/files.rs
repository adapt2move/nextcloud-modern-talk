//! A finite DAV lane using the same origin, cookie, cancellation and global I/O owner.
use super::*;
use tokio::io::{AsyncWrite, AsyncWriteExt};
const FILE_LIMIT: Duration = Duration::from_secs(60);
pub(super) enum StreamRead<'a> {
    File(Option<&'a (dyn Fn(u64, Option<u64>) + Send + Sync)>),
    Image,
    Pdf,
}
impl Transport {
    pub(super) fn file_endpoint(
        &self,
        path: &crate::files::FilePath,
    ) -> Result<url::Url, PortError> {
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
            for segment in path.as_str().split('/').skip(1) {
                parts.push(segment);
            }
        }
        Ok(url)
    }
    pub(crate) async fn file<W: AsyncWrite + Unpin + Send>(
        &mut self,
        c: &OperationContext,
        path: crate::files::FilePath,
        upload: Option<(reqwest::Body, u64)>,
        sink: &mut W,
    ) -> Result<u64, PortError> {
        let url = self.file_endpoint(&path)?;
        self.stream(c, url, upload, sink, StreamRead::File(None))
            .await
    }
    pub(crate) async fn file_progress<W: AsyncWrite + Unpin + Send>(
        &mut self,
        c: &OperationContext,
        path: crate::files::FilePath,
        sink: &mut W,
        progress: &(dyn Fn(u64, Option<u64>) + Send + Sync),
    ) -> Result<u64, PortError> {
        let url = self.file_endpoint(&path)?;
        self.stream(c, url, None, sink, StreamRead::File(Some(progress)))
            .await
    }
    pub(super) async fn stream<W: AsyncWrite + Unpin + Send>(
        &mut self,
        c: &OperationContext,
        url: url::Url,
        upload: Option<(reqwest::Body, u64)>,
        sink: &mut W,
        read: StreamRead<'_>,
    ) -> Result<u64, PortError> {
        let pdf = matches!(read, StreamRead::Pdf);
        let (read_limit, progress, image) = match read {
            StreamRead::File(progress) => (crate::files::MAX_FILE_BYTES, progress, false),
            StreamRead::Image => (crate::content::MAX_IMAGE_BYTES as u64, None, true),
            StreamRead::Pdf => (crate::content::MAX_IMAGE_BYTES as u64, None, false),
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
            .request(if writing { Method::PUT } else { Method::GET }, url.clone())
            .timeout(FILE_LIMIT)
            .header(ACCEPT_ENCODING, "identity")
            .header(CACHE_CONTROL, "no-store")
            .basic_auth(
                self.binding.login_name.as_str(),
                Some(self.binding.password.as_str()),
            );
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
                200 if !writing => {}
                401 | 403 => return Err(self.error(ErrorKind::PermissionDenied)),
                404 => return Err(self.error(ErrorKind::NotFound)),
                409 | 412 => return Err(self.error(ErrorKind::Conflict)),
                413 => return Err(self.error(ErrorKind::TooLarge)),
                429 => return Err(self.error(ErrorKind::RateLimited)),
                _ => return Err(self.error(ErrorKind::ServerUnavailable)),
            }
            let limit = if writing {
                BODY_LIMIT as u64
            } else {
                read_limit
            };
            if response.content_length().is_some_and(|n| n > limit) {
                return Err(self.error(ErrorKind::TooLarge));
            }
            if image || pdf {
                let mime = response
                    .headers()
                    .get(CONTENT_TYPE)
                    .and_then(|v| v.to_str().ok())
                    .and_then(|v| v.split(';').next())
                    .unwrap_or("");
                let accepted = if pdf {
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
            Ok((length, cookies))
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
            Ok((length, cookies)) => {
                self.cookies = cookies;
                self.commit_shared_cookies(shared)?;
                Ok(length)
            }
            Err(error) => Err(error),
        }
    }
}
