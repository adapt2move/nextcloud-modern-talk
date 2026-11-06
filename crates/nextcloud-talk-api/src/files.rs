//! Account-owned WebDAV streams. JSON limits remain independent of binary transfers.
//! Each transfer is finite, cancelable and bounded by the caller and the 32 MiB policy.
use crate::{Client, Error, ErrorKind, scope::OperationContext};
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite};
pub const MAX_FILE_BYTES: u64 = 32 * 1024 * 1024;
pub const FILE_CHUNK_BYTES: usize = 64 * 1024;
const MAX_DIRECTORY_ENTRIES: usize = 1000;

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct DirectoryEntry {
    pub name: String,
    pub is_directory: bool,
    pub size: Option<u64>,
    pub modified: Option<String>,
    pub content_type: Option<String>,
    pub file_id: Option<String>,
}

fn parse_listing(bytes: &[u8], base: &url::Url) -> Result<Vec<DirectoryEntry>, ErrorKind> {
    let xml = std::str::from_utf8(bytes).map_err(|_| ErrorKind::Malformed)?;
    let doc = roxmltree::Document::parse(xml).map_err(|_| ErrorKind::Malformed)?;
    let root = doc.root_element();
    if !root.has_tag_name(("DAV:", "multistatus")) {
        return Err(ErrorKind::Malformed);
    }
    let mut entries = Vec::new();
    for response in root.children().filter(|n| n.has_tag_name(("DAV:", "response"))) {
        let href = response.children().find(|n| n.has_tag_name(("DAV:", "href")))
            .and_then(|n| n.text()).ok_or(ErrorKind::Malformed)?;
        let href = base.join(href).map_err(|_| ErrorKind::Malformed)?;
        if href.origin() != base.origin() || href.query().is_some() || href.fragment().is_some() {
            return Err(ErrorKind::WrongScope);
        }
        if href.path().trim_end_matches('/') == base.path().trim_end_matches('/') {
            continue;
        }
        let propstat = response.children().find(|n| {
            n.has_tag_name(("DAV:", "propstat")) && n.children().any(|s| {
                s.has_tag_name(("DAV:", "status")) && s.text().is_some_and(|v| v.split_whitespace().nth(1) == Some("200"))
            })
        }).ok_or(ErrorKind::Malformed)?;
        let prop = propstat.children().find(|n| n.has_tag_name(("DAV:", "prop"))).ok_or(ErrorKind::Malformed)?;
        let value = |ns, name| prop.children().find(|n| n.has_tag_name((ns, name))).and_then(|n| n.text());
        let name = value("DAV:", "displayname").ok_or(ErrorKind::Malformed)?;
        if name.is_empty() || name.len() > 255 || name.contains(['/', '\\', '\0']) || name.chars().any(char::is_control) || matches!(name, "." | "..") {
            return Err(ErrorKind::Malformed);
        }
        let mut expected = base.clone();
        expected.path_segments_mut().map_err(|_| ErrorKind::Malformed)?.pop_if_empty().push(name);
        if href.path().trim_end_matches('/') != expected.path().trim_end_matches('/') {
            return Err(ErrorKind::WrongScope);
        }
        if entries.len() == MAX_DIRECTORY_ENTRIES { return Err(ErrorKind::TooLarge); }
        let is_directory = prop.children().find(|n| n.has_tag_name(("DAV:", "resourcetype")))
            .is_some_and(|n| n.children().any(|c| c.has_tag_name(("DAV:", "collection"))));
        let bounded = |value: Option<&str>| value.filter(|s| s.len() <= 256).map(str::to_owned);
        entries.push(DirectoryEntry {
            name: name.to_owned(), is_directory,
            size: value("DAV:", "getcontentlength").and_then(|v| v.parse().ok()),
            modified: bounded(value("DAV:", "getlastmodified")),
            content_type: bounded(value("DAV:", "getcontenttype")),
            file_id: bounded(value("http://owncloud.org/ns", "fileid")),
        });
    }
    Ok(entries)
}
/// A DAV directory relative to the authenticated user's files root.
/// Root is represented by `/`; other paths have no trailing slash.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct DirectoryPath(String);
impl DirectoryPath {
    pub fn parse(path: &str) -> Result<Self, ErrorKind> {
        if path == "/" {
            return Ok(Self(path.to_owned()));
        }
        FilePath::parse(path).map(|path| Self(path.0))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }

    /// Return the parent without allowing navigation above the user's root.
    pub fn parent(&self) -> Option<Self> {
        if self.0 == "/" {
            return None;
        }
        let end = self.0.rfind('/')?;
        Some(Self(if end == 0 {
            "/".to_owned()
        } else {
            self.0[..end].to_owned()
        }))
    }
}
#[derive(Clone, Debug)]
pub struct FilePath(String);
impl FilePath {
    pub fn parse(path: &str) -> Result<Self, ErrorKind> {
        if path.is_empty()
            || path.len() > 16384
            || !path.starts_with('/')
            || path.ends_with('/')
            || path.contains(['\\', '\0'])
            || path.chars().any(char::is_control)
            || path
                .split('/')
                .skip(1)
                .any(|s| s.is_empty() || matches!(s, "." | ".."))
        {
            return Err(ErrorKind::Malformed);
        }
        Ok(Self(path.to_owned()))
    }
    pub fn as_str(&self) -> &str {
        &self.0
    }
}
impl Client {
    pub async fn list_directory(&mut self, c: &OperationContext, path: &DirectoryPath) -> Result<Vec<DirectoryEntry>, Error> {
        let (base, body) = self.transport.directory_listing(c, path).await?;
        parse_listing(&body, &base).map_err(|kind| self.error(kind))
    }
    /// Share an existing file into a room. Policy and capability checks belong to
    /// the caller; an interrupted mutation must be reconciled, never replayed.
    pub async fn share_existing_file(
        &mut self,
        c: &OperationContext,
        room: &crate::models::RoomToken,
        path: &FilePath,
        caption: &str,
    ) -> Result<String, Error> {
        self.validate(c)?;
        if caption.len() > 4096 || caption.contains('\0') {
            return Err(self.error(ErrorKind::Malformed));
        }
        let mut form = vec![
            ("shareType", "10".to_owned()),
            ("shareWith", room.as_str().to_owned()),
            ("path", path.as_str().to_owned()),
        ];
        if !caption.is_empty() {
            form.push((
                "talkMetaData",
                serde_json::json!({"caption": caption}).to_string(),
            ));
        }
        let response = self
            .transport
            .ocs(
                c,
                reqwest::Method::POST,
                "ocs/v2.php/apps/files_sharing/api/v1/shares",
                &[],
                &form,
                false,
            )
            .await
            .map_err(|error| self.mutation_error(error))?;
        let id = &response.data["id"];
        let id = id
            .as_str()
            .map(str::to_owned)
            .or_else(|| id.as_u64().map(|v| v.to_string()));
        id.filter(|v| !v.is_empty() && v.len() <= 128 && v.bytes().all(|b| b.is_ascii_digit()))
            .ok_or_else(|| self.mutation_error(self.error(ErrorKind::Malformed)))
    }
    /// Create only. The stream is not rewindable and an uncertain PUT is never replayed.
    pub async fn upload_file<R: AsyncRead + Unpin + Send + 'static>(
        &mut self,
        c: &OperationContext,
        path: FilePath,
        reader: R,
        size: u64,
    ) -> Result<(), Error> {
        self.upload_file_progress(c, path, reader, size, std::sync::Arc::new(|_, _| {}))
            .await
    }
    /// Counts request-body bytes, not a confirmed server commit.
    pub async fn upload_file_progress<R: AsyncRead + Unpin + Send + 'static>(
        &mut self,
        c: &OperationContext,
        path: FilePath,
        reader: R,
        size: u64,
        progress: std::sync::Arc<dyn Fn(u64, Option<u64>) + Send + Sync>,
    ) -> Result<(), Error> {
        self.validate(c)?;
        if size > MAX_FILE_BYTES {
            return Err(self.error(ErrorKind::TooLarge));
        }
        progress(0, Some(size));
        let stream = futures_lite::stream::unfold(
            (reader, size, progress),
            move |(mut reader, remaining, progress)| async move {
                if remaining == 0 {
                    return None;
                }
                let mut block = vec![0; remaining.min(FILE_CHUNK_BYTES as u64) as usize];
                match reader.read(&mut block).await {
                    Ok(0) => Some((
                        Err(std::io::Error::from(std::io::ErrorKind::UnexpectedEof)),
                        (reader, 0, progress),
                    )),
                    Ok(n) => {
                        block.truncate(n);
                        let remaining = remaining - n as u64;
                        progress(size - remaining, Some(size));
                        Some((Ok(block), (reader, remaining, progress)))
                    }
                    Err(e) => Some((Err(e), (reader, 0, progress))),
                }
            },
        );
        self.transport
            .file(
                c,
                path,
                Some((reqwest::Body::wrap_stream(stream), size)),
                &mut tokio::io::sink(),
            )
            .await
            .map(|_| ())
            .map_err(|e| self.mutation_error(e))
    }
    pub async fn download_file_progress<W: AsyncWrite + Unpin + Send>(
        &mut self,
        c: &OperationContext,
        path: FilePath,
        sink: &mut W,
        progress: &(dyn Fn(u64, Option<u64>) + Send + Sync),
    ) -> Result<u64, Error> {
        self.transport.file_progress(c, path, sink, progress).await
    }
    /// Writes bounded chunks into an application-owned staging sink. The caller publishes
    /// the destination only after success and removes partial output on cancellation/drop.
    pub async fn download_file<W: AsyncWrite + Unpin + Send>(
        &mut self,
        c: &OperationContext,
        path: FilePath,
        sink: &mut W,
    ) -> Result<u64, Error> {
        self.transport.file(c, path, None, sink).await
    }
}

#[cfg(test)]
mod directory_tests {
    use super::DirectoryPath;

    #[test]
    fn directory_navigation_stays_within_root() {
        let nested = DirectoryPath::parse("/Documents/Plans").unwrap();
        let parent = nested.parent().unwrap();
        assert_eq!(parent.as_str(), "/Documents");
        let root = parent.parent().unwrap();
        assert_eq!(root.as_str(), "/");
        assert!(root.parent().is_none());
    }

    #[test]
    fn directory_paths_reject_ambiguous_segments() {
        for path in [
            "", "relative", "//", "/a/", "/a//b", "/a/../b", "/a/./b", "/a\\b", "/a\0b",
        ] {
            assert!(DirectoryPath::parse(path).is_err(), "{path:?}");
        }
        assert!(DirectoryPath::parse("/日本語/plan #1").is_ok());
    }
}
