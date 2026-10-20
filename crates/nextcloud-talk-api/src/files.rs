//! Account-owned WebDAV streams. JSON limits remain independent of binary transfers.
//! Each transfer is finite, cancelable and bounded by the caller and the 32 MiB policy.
use crate::{Client, Error, ErrorKind, scope::OperationContext};
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite};
pub const MAX_FILE_BYTES: u64 = 32 * 1024 * 1024;
pub const FILE_CHUNK_BYTES: usize = 64 * 1024;
#[derive(Clone)]
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
    /// Create only. The stream is not rewindable and an uncertain PUT is never replayed.
    pub async fn upload_file<R: AsyncRead + Unpin + Send + 'static>(
        &mut self,
        c: &OperationContext,
        path: FilePath,
        reader: R,
        size: u64,
    ) -> Result<(), Error> {
        self.validate(c)?;
        if size > MAX_FILE_BYTES {
            return Err(self.error(ErrorKind::TooLarge));
        }
        let stream =
            futures_lite::stream::unfold((reader, size), |(mut reader, remaining)| async move {
                if remaining == 0 {
                    return None;
                }
                let mut block = vec![0; remaining.min(FILE_CHUNK_BYTES as u64) as usize];
                match reader.read(&mut block).await {
                    Ok(0) => Some((
                        Err(std::io::Error::from(std::io::ErrorKind::UnexpectedEof)),
                        (reader, 0),
                    )),
                    Ok(n) => {
                        block.truncate(n);
                        Some((Ok(block), (reader, remaining - n as u64)))
                    }
                    Err(e) => Some((Err(e), (reader, 0))),
                }
            });
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
