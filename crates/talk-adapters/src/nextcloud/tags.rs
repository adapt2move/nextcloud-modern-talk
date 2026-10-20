//! Private released Talk24/25 conversation-tag routes. Both pinned versions share
//! ConversationTagController and ConversationTagService semantics. Server IDs stay
//! in the bounded client map; renderer-facing handles cannot select arbitrary IDs.
use super::{NextcloudClient, wire::Capabilities};
use reqwest::Method;
use serde_json::Value;
use talk_core::{
    bounded::{Page, Text},
    ports::{ErrorKind, OperationContext, PortError, ProtocolReply, ProtocolRequest},
    protocol::{
        Conversation, ConversationTag, ConversationTagId, ConversationTags, FeatureSupport, TagKind,
    },
};

pub(super) struct TagBinding {
    id: ConversationTagId,
    wire: String,
    live: bool,
    kind: TagKind,
}

fn valid_name(name: &str) -> bool {
    !name.is_empty() && name == name.trim() && name.chars().count() <= 250
        && !name.chars().any(|c| c.is_control() || matches!(c,
            '\u{061c}' | '\u{200e}' | '\u{200f}' | '\u{202a}'..='\u{202e}' | '\u{2066}'..='\u{2069}'))
}

impl NextcloudClient {
    fn tags_supported(&self, caps: &Capabilities) -> Result<(), PortError> {
        match caps.feature("conversation-tags") {
            FeatureSupport::Supported => Ok(()),
            FeatureSupport::Unsupported => Err(self.error(ErrorKind::Unsupported)),
            FeatureSupport::Unknown => Err(self.error(ErrorKind::Unconfigured)),
        }
    }

    fn bind_tag(&mut self, wire: &str) -> Result<ConversationTagId, PortError> {
        if wire.is_empty()
            || wire.len() > 32
            || wire.starts_with('0')
            || !wire.bytes().all(|b| b.is_ascii_digit())
        {
            return Err(self.error(ErrorKind::Malformed));
        }
        if let Some(tag) = self.tags.iter().find(|tag| tag.wire == wire) {
            return Ok(tag.id);
        }
        if self.tags.len() >= 256 {
            return Err(self.error(ErrorKind::Busy));
        }
        let mut random = [0; 16];
        getrandom::fill(&mut random).map_err(|_| self.error(ErrorKind::Transport))?;
        let id = ConversationTagId {
            scope: self.transport.scope(),
            local: u128::from_le_bytes(random),
        };
        if self.tags.iter().any(|tag| tag.id == id) {
            return Err(self.error(ErrorKind::Conflict));
        }
        self.tags.push(TagBinding {
            id,
            wire: wire.to_owned(),
            live: false,
            kind: TagKind::Unknown,
        });
        Ok(id)
    }

    fn parse_tag(&mut self, data: &Value) -> Result<ConversationTag, PortError> {
        let wire = data
            .get("id")
            .and_then(Value::as_str)
            .ok_or_else(|| self.error(ErrorKind::Malformed))?;
        let name = data
            .get("name")
            .and_then(Value::as_str)
            .filter(|name| valid_name(name))
            .ok_or_else(|| self.error(ErrorKind::Malformed))?;
        let kind = match data.get("type").and_then(Value::as_str) {
            Some("custom") => TagKind::Custom,
            Some("favorites") => TagKind::Favorites,
            Some("other") => TagKind::Other,
            _ => TagKind::Unknown,
        };
        let order = data
            .get("sortOrder")
            .and_then(Value::as_u64)
            .and_then(|order| u32::try_from(order).ok())
            .ok_or_else(|| self.error(ErrorKind::Malformed))?;
        let collapsed = data
            .get("collapsed")
            .and_then(Value::as_bool)
            .ok_or_else(|| self.error(ErrorKind::Malformed))?;
        Ok(ConversationTag {
            id: self.bind_tag(wire)?,
            name: Text::new(name).map_err(|_| self.error(ErrorKind::TooLarge))?,
            order,
            collapsed,
            kind,
        })
    }

    fn checked_tag(&self, id: ConversationTagId) -> Result<&TagBinding, PortError> {
        if id.scope != self.transport.scope() {
            return Err(self.error(ErrorKind::WrongScope));
        }
        self.tags
            .iter()
            .find(|tag| tag.id == id && tag.live)
            .ok_or_else(|| self.error(ErrorKind::NotFound))
    }

    pub(super) fn project_room_tags(
        &mut self,
        room: &mut Conversation,
        data: &Value,
        caps: &Capabilities,
    ) -> Result<(), PortError> {
        if caps.feature("conversation-tags") != FeatureSupport::Supported {
            return Ok(());
        }
        let Some(values) = data.get("tagIds").and_then(Value::as_array) else {
            room.organization.tag_change = talk_core::policy::Decision::Unknown;
            return Ok(());
        };
        if values.len() > 20 {
            return Err(self.error(ErrorKind::TooLarge));
        }
        let mut ids = Page::empty();
        for value in values {
            let wire = match value {
                Value::String(value) => value.clone(),
                Value::Number(value) if value.as_u64().is_some() => value.to_string(),
                _ => return Err(self.error(ErrorKind::Malformed)),
            };
            let id = self.bind_tag(&wire)?;
            if ids.as_slice().contains(&id) {
                return Err(self.error(ErrorKind::Malformed));
            }
            ids.try_push(id)
                .map_err(|_| self.error(ErrorKind::TooLarge))?;
        }
        room.tag_ids = Some(ids);
        Ok(())
    }

    async fn read_tags(
        &mut self,
        context: &OperationContext,
    ) -> Result<ConversationTags, PortError> {
        let response = self
            .transport
            .ocs(
                context,
                Method::GET,
                "ocs/v2.php/apps/spreed/api/v4/tags",
                &[],
                &[],
                false,
            )
            .await?;
        let values = response
            .data
            .as_array()
            .ok_or_else(|| self.error(ErrorKind::Malformed))?;
        if values.len() > 102 {
            return Err(self.error(ErrorKind::TooLarge));
        }
        let mut tags = Page::<ConversationTag, 102>::empty();
        for data in values {
            let tag = self.parse_tag(data)?;
            if tags.as_slice().iter().any(|old| old.id == tag.id) {
                return Err(self.error(ErrorKind::Malformed));
            }
            tags.try_push(tag)
                .map_err(|_| self.error(ErrorKind::TooLarge))?;
        }
        // Only a complete valid catalog updates liveness; never evict/rebind handles.
        for binding in &mut self.tags {
            let current = tags.as_slice().iter().find(|tag| tag.id == binding.id);
            binding.live = current.is_some();
            binding.kind = current.map_or(TagKind::Unknown, |tag| tag.kind);
        }
        self.tag_creation_uncertain = false;
        Ok(ConversationTags {
            scope: context.scope,
            tags,
        })
    }

    pub(super) async fn tag_request(
        &mut self,
        context: &OperationContext,
        request: ProtocolRequest,
        caps: &Capabilities,
    ) -> Result<ProtocolReply, PortError> {
        self.tags_supported(caps)?;
        if request == ProtocolRequest::ReadTags {
            return Ok(ProtocolReply::Tags(self.read_tags(context).await?));
        }
        if let ProtocolRequest::CreateTag { name } = request {
            if self.tag_creation_uncertain {
                return Err(self.error(ErrorKind::Uncertain));
            }
            let name = name.as_str().trim();
            if !valid_name(name) {
                return Err(self.error(ErrorKind::Malformed));
            }
            self.tag_creation_uncertain = true;
            let response = match self
                .write(
                    context,
                    Method::POST,
                    "ocs/v2.php/apps/spreed/api/v4/tags",
                    &[("name", name.into())],
                )
                .await
            {
                Ok(response) => response,
                Err(error) => {
                    self.tag_creation_uncertain = error.kind == ErrorKind::Uncertain;
                    return Err(error);
                }
            };
            let tag = self
                .parse_tag(&response.data)
                .map_err(|_| self.error(ErrorKind::Uncertain))?;
            if tag.kind != TagKind::Custom || tag.name.as_str() != name {
                return Err(self.error(ErrorKind::Uncertain));
            }
            let binding = self
                .tags
                .iter_mut()
                .find(|binding| binding.id == tag.id)
                .expect("bound tag");
            binding.live = true;
            binding.kind = tag.kind;
            self.tag_creation_uncertain = false;
            return Ok(ProtocolReply::TagCreated(tag));
        }
        // Re-read authoritative ownership/kind before accepting a mutation handle.
        let before = self.read_tags(context).await?;
        let (method, route, form) = match &request {
            ProtocolRequest::RenameTag { tag, name } => {
                let binding = self.checked_tag(*tag)?;
                if binding.kind != TagKind::Custom {
                    return Err(self.error(ErrorKind::PermissionDenied));
                }
                let name = name.as_str().trim();
                if !valid_name(name) {
                    return Err(self.error(ErrorKind::Malformed));
                }
                (
                    Method::PUT,
                    format!("ocs/v2.php/apps/spreed/api/v4/tags/{}", binding.wire),
                    vec![("name", name.into())],
                )
            }
            ProtocolRequest::DeleteTag(tag) => {
                let binding = self.checked_tag(*tag)?;
                if binding.kind != TagKind::Custom {
                    return Err(self.error(ErrorKind::PermissionDenied));
                }
                (
                    Method::DELETE,
                    format!("ocs/v2.php/apps/spreed/api/v4/tags/{}", binding.wire),
                    vec![],
                )
            }
            ProtocolRequest::SetTagCollapsed { tag, collapsed } => {
                let binding = self.checked_tag(*tag)?;
                if binding.kind == TagKind::Unknown {
                    return Err(self.error(ErrorKind::Unconfigured));
                }
                (
                    Method::PUT,
                    format!(
                        "ocs/v2.php/apps/spreed/api/v4/tags/{}/collapsed",
                        binding.wire
                    ),
                    vec![(
                        "collapsed",
                        if *collapsed {
                            "true".into()
                        } else {
                            "false".into()
                        },
                    )],
                )
            }
            ProtocolRequest::ReorderTags { tags } => {
                if tags.as_slice().len() != before.tags.as_slice().len() {
                    return Err(self.error(ErrorKind::Conflict));
                }
                let mut seen = Vec::new();
                let mut form = Vec::new();
                for id in tags.as_slice() {
                    let binding = self.checked_tag(*id)?;
                    if binding.kind == TagKind::Unknown || seen.contains(id) {
                        return Err(self.error(ErrorKind::Conflict));
                    }
                    seen.push(*id);
                    form.push(("orderedIds[]", binding.wire.clone()));
                }
                (
                    Method::PUT,
                    "ocs/v2.php/apps/spreed/api/v4/tags/reorder".into(),
                    form,
                )
            }
            ProtocolRequest::SetConversationTags { conversation, tags } => {
                let mut seen = Vec::new();
                let mut form = Vec::new();
                for id in tags.as_slice() {
                    let binding = self.checked_tag(*id)?;
                    if binding.kind != TagKind::Custom || seen.contains(id) {
                        return Err(self.error(ErrorKind::Conflict));
                    }
                    seen.push(*id);
                    form.push(("tagIds[]", binding.wire.clone()));
                }
                let room = self.room(context, *conversation, caps).await?;
                if room.organization.tag_change == talk_core::policy::Decision::Unknown {
                    return Err(self.error(ErrorKind::Unconfigured));
                }
                self.allowed(room.organization.tag_change)?;
                (
                    Method::POST,
                    self.route(*conversation, "v4/room", "/tags")?,
                    form,
                )
            }
            _ => return Err(self.error(ErrorKind::Malformed)),
        };
        self.write(context, method, &route, &form).await?;
        if let ProtocolRequest::SetConversationTags { conversation, tags } = &request {
            let room = self
                .room(context, *conversation, caps)
                .await
                .map_err(|_| self.error(ErrorKind::Uncertain))?;
            if room.tag_ids.as_ref().is_none_or(|actual| {
                actual.as_slice().len() != tags.as_slice().len()
                    || tags
                        .as_slice()
                        .iter()
                        .any(|id| !actual.as_slice().contains(id))
            }) {
                return Err(self.error(ErrorKind::Uncertain));
            }
            return Ok(ProtocolReply::Conversation(room));
        }
        let after = self
            .read_tags(context)
            .await
            .map_err(|_| self.error(ErrorKind::Uncertain))?;
        let confirmed = match request {
            ProtocolRequest::RenameTag { tag, name } => after
                .tags
                .as_slice()
                .iter()
                .any(|actual| actual.id == tag && actual.name.as_str() == name.as_str().trim()),
            ProtocolRequest::DeleteTag(tag) => {
                !after.tags.as_slice().iter().any(|actual| actual.id == tag)
            }
            ProtocolRequest::SetTagCollapsed { tag, collapsed } => after
                .tags
                .as_slice()
                .iter()
                .any(|actual| actual.id == tag && actual.collapsed == collapsed),
            ProtocolRequest::ReorderTags { tags } => {
                after.tags.as_slice().len() == tags.as_slice().len()
                    && tags.as_slice().iter().enumerate().all(|(order, id)| {
                        after
                            .tags
                            .as_slice()
                            .iter()
                            .any(|actual| actual.id == *id && actual.order as usize == order)
                    })
            }
            _ => false,
        };
        if !confirmed {
            return Err(self.error(ErrorKind::Uncertain));
        }
        Ok(ProtocolReply::Tags(after))
    }
}
