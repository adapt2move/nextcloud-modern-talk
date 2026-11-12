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
        // NextCloud answers a link it could not read with the link itself as
        // its title. A card that only repeats the address says nothing.
        if thumbnail.is_none()
            && resolved
                .description
                .as_deref()
                .is_none_or(|description| description.trim().is_empty())
            && repeats_link(&title, &[url.as_str(), resolved.reference.as_str()])
        {
            return Ok(ProtocolReply::Reference(None));
        }
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

/// The title is one of the links, its host, or any other bare web address.
fn repeats_link(title: &str, links: &[&str]) -> bool {
    fn plain(value: &str) -> String {
        let value = value.trim();
        let value = value
            .strip_prefix("https://")
            .or_else(|| value.strip_prefix("http://"))
            .unwrap_or(value);
        value
            .strip_prefix("www.")
            .unwrap_or(value)
            .trim_end_matches('/')
            .to_lowercase()
    }
    let wanted = plain(title);
    links.iter().any(|link| {
        let link = plain(link);
        link == wanted || link.split(['/', '?', '#']).next() == Some(wanted.as_str())
    }) || talk_core::markup::web_destination(title.trim()).is_some()
}

#[cfg(test)]
mod tests {
    use super::repeats_link;

    #[test]
    fn a_title_that_only_repeats_the_address_is_no_preview() {
        let link = "https://www.example.test/path/page?id=1";
        for title in [
            link,
            "https://www.example.test/path/page?id=1/",
            "example.test/path/page?id=1",
            "Example.test",
            "www.example.test",
            "https://other.example/elsewhere",
        ] {
            assert!(repeats_link(title, &[link]), "{title}");
        }
        for title in ["A synthetic article", "example", "Page 1 of example.test"] {
            assert!(!repeats_link(title, &[link]), "{title}");
        }
    }
}
