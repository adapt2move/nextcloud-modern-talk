//! Typed installation-relative binary reads using the account's existing HTTP lane.
use crate::{Client, Error, models::RoomToken, scope::OperationContext};
pub const MAX_IMAGE_BYTES: usize = 4 * 1024 * 1024;
pub const MAX_VIDEO_BYTES: usize = 32 * 1024 * 1024;
/// Only local-player container inputs, never playlists or external media URLs.
/// Actual codec support is determined by the caller's bounded native decoder.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum VideoFormat {
    Mp4,
    WebM,
}
impl VideoFormat {
    pub(crate) fn mime_type(self) -> &'static str {
        match self {
            Self::Mp4 => "video/mp4",
            Self::WebM => "video/webm",
        }
    }
}
#[derive(Clone, Debug)]
pub enum ImageRequest {
    UserAvatar {
        user: String,
    },
    ConversationAvatar(RoomToken),
    FilePreview {
        file: u64,
        large: bool,
    },
    /// An account-relative DAV image, never an arbitrary rich-object URL.
    FileImage(crate::files::FilePath),
}
impl Client {
    /// Only known server routes are accepted; no arbitrary authenticated URL entry point.
    pub async fn image(
        &mut self,
        c: &OperationContext,
        request: ImageRequest,
    ) -> Result<Vec<u8>, Error> {
        self.transport.image(c, request).await
    }
    /// Bounded authenticated PDF input for the caller's native preview renderer.
    pub async fn preview_pdf(
        &mut self,
        c: &OperationContext,
        path: crate::files::FilePath,
    ) -> Result<Vec<u8>, Error> {
        self.transport.preview_pdf(c, path).await
    }
    /// Bounded original bytes from an account-relative DAV path. A format label
    /// requires an exact response MIME match; it is not a claim of decodability.
    pub async fn video(
        &mut self,
        c: &OperationContext,
        path: crate::files::FilePath,
        format: VideoFormat,
    ) -> Result<Vec<u8>, Error> {
        self.transport.video(c, path, format).await
    }
    pub fn file_link(&self, file: u64) -> Result<String, Error> {
        self.transport.file_link(file)
    }
}
