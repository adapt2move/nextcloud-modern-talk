//! Resolve links through NextCloud only; never make a client request to the linked host.
use super::*;
use nextcloud_talk_api as api;
use talk_core::protocol::{
    content::ImageBytes,
    reference::{ReferenceUrl, Resolved},
};

impl NextcloudClient {
    pub(super) async fn resolve_reference(
        &mut self,
        context: &OperationContext,
        message: MessageId,
        url: ReferenceUrl,
        caps: &Capabilities,
    ) -> Result<ProtocolReply, PortError> {
        if message.sequence == 0 {
            return Err(self.error(ErrorKind::Malformed));
        }
        let room = self.room(context, message.conversation, caps).await?;
        self.allowed(room.actions.read_messages)?;
        if room.classified || room.sensitive || room.unknown_restrictions {
            return Err(self.error(ErrorKind::PermissionDenied));
        }
        let api::Reply::Reference(resolved) = self
            .transport
            .api_request(
                context,
                api::Request::ResolveReference(url.as_str().to_owned()),
            )
            .await?
        else {
            return Err(self.error(ErrorKind::Malformed));
        };
        let Some(resolved) = resolved else {
            return Ok(ProtocolReply::Reference(None));
        };
        let Some(title) = resolved.title.filter(|title| !title.trim().is_empty()) else {
            return Ok(ProtocolReply::Reference(None));
        };
        let thumbnail = if let Some(url) = resolved.thumbnail {
            self.transport
                .reference_thumbnail(context, &url)
                .await
                .ok()
                .and_then(|bytes| ImageBytes::new(bytes).ok())
        } else {
            None
        };
        self.transport.validate(context)?;
        Ok(ProtocolReply::Reference(Some(Resolved {
            title: Text::new(&title).map_err(|_| self.error(ErrorKind::TooLarge))?,
            description: resolved
                .description
                .as_deref()
                .map(Text::new)
                .transpose()
                .map_err(|_| self.error(ErrorKind::TooLarge))?,
            thumbnail,
        })))
    }
}
