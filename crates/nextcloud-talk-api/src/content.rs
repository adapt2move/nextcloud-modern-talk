//! Typed installation-relative binary reads using the account's existing HTTP lane.
use crate::{Client, Error, models::RoomToken, scope::OperationContext};
pub const MAX_IMAGE_BYTES: usize = 4 * 1024 * 1024;
#[derive(Clone, Debug)]
pub enum ImageRequest {
    UserAvatar { user: String },
    ConversationAvatar(RoomToken),
    FilePreview { file: u64, large: bool },
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
    pub fn file_link(&self, file: u64) -> Result<String, Error> {
        self.transport.file_link(file)
    }
}
