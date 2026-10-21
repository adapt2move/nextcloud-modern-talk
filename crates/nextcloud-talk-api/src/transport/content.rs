use super::*;
use crate::Error;
use crate::content::ImageRequest;
impl crate::transport::Transport {
    fn content_endpoint(&self, route: &str) -> Result<url::Url, Error> {
        self.binding
            .origin
            .endpoint(route)
            .map_err(|k| self.error(k))
    }
    pub(crate) fn file_link(&self, file: u64) -> Result<String, Error> {
        if file == 0 {
            return Err(self.error(ErrorKind::Malformed));
        }
        Ok(self
            .content_endpoint(&format!("index.php/f/{file}"))?
            .into())
    }
    pub(crate) async fn preview_pdf(
        &mut self,
        c: &OperationContext,
        path: crate::files::FilePath,
    ) -> Result<Vec<u8>, Error> {
        let url = self.file_endpoint(&path)?;
        let mut bytes = Vec::new();
        self.stream(c, url, None, &mut bytes, super::files::StreamRead::Pdf)
            .await?;
        if !bytes.starts_with(b"%PDF-") {
            return Err(self.error(ErrorKind::Malformed));
        }
        Ok(bytes)
    }
    pub(crate) async fn image(
        &mut self,
        c: &OperationContext,
        request: ImageRequest,
    ) -> Result<Vec<u8>, Error> {
        self.validate(c)?;
        let url = match request {
            ImageRequest::UserAvatar { user } => {
                if user.is_empty()
                    || user.len() > 512
                    || matches!(user.as_str(), "." | "..")
                    || user.chars().any(char::is_control)
                {
                    return Err(self.error(ErrorKind::Malformed));
                }
                let mut url = self.content_endpoint("index.php/avatar/")?;
                {
                    let mut parts = url
                        .path_segments_mut()
                        .map_err(|_| self.error(ErrorKind::Malformed))?;
                    parts.pop_if_empty().push(&user).push("512");
                }
                url
            }
            ImageRequest::ConversationAvatar(room) => self.content_endpoint(&format!(
                "ocs/v2.php/apps/spreed/api/v1/room/{}/avatar",
                room.as_str()
            ))?,
            ImageRequest::FilePreview { file, large } => {
                if file == 0 {
                    return Err(self.error(ErrorKind::Malformed));
                }
                let mut url = self.content_endpoint("index.php/core/preview")?;
                url.query_pairs_mut()
                    .append_pair("fileId", &file.to_string())
                    .append_pair("x", if large { "1600" } else { "400" })
                    .append_pair("y", if large { "1200" } else { "300" })
                    .append_pair("a", "1")
                    .append_pair("mode", "fill")
                    .append_pair("forceIcon", "0")
                    .append_pair("mimeFallback", "0");
                url
            }
            ImageRequest::FileImage(path) => self.file_endpoint(&path)?,
        };
        let mut bytes = Vec::new();
        self.stream(c, url, None, &mut bytes, super::files::StreamRead::Image)
            .await?;
        Ok(bytes)
    }
}
