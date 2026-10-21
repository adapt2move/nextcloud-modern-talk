//! Typed installation-relative binary reads using the account's existing HTTP lane.
use crate::{Client, Error, models::RoomToken, scope::OperationContext};
pub const MAX_IMAGE_BYTES: usize = 4 * 1024 * 1024;
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
    pub fn file_link(&self, file: u64) -> Result<String, Error> {
        self.transport.file_link(file)
    }
}
