//! Bounded per-account reference cache and one low-priority resolver lane.
//! Source URLs remain here; the renderer receives only card text and an opaque asset key.
use super::*;
use talk_core::protocol::reference::{ReferenceUrl, Resolved};
use talk_core::ui::{
    content::{AssetState, AssetTarget, AssetView},
    presentation::DeliveryState,
    reference::Preview,
};

const CACHE_ENTRIES: usize = 64;
const CACHE_IMAGE_BYTES: usize = 4 * 1024 * 1024;
const CACHE_SINGLE_IMAGE: usize = 256 * 1024;
const LIFETIME: Duration = Duration::from_secs(15 * 60);
const NEGATIVE_LIFETIME: Duration = Duration::from_secs(60);

#[derive(Clone, Eq, PartialEq)]
struct Binding {
    context: ConversationRef,
    message: u128,
    ordinal: u16,
    source: Text<4096>,
    url: ReferenceUrl,
}
struct Attached {
    binding: Binding,
    preview: Preview,
    expires: Instant,
}
struct Cached {
    url: ReferenceUrl,
    value: Option<Resolved>,
    expires: Instant,
}
struct PendingReference {
    binding: Binding,
    generation: u64,
    attempts: u8,
    due: Option<Instant>,
    deadline: Instant,
}
pub(super) struct Owner {
    cache: Vec<Cached>,
    attached: Vec<Attached>,
    pending: Option<PendingReference>,
    next_scan: Instant,
}
impl Default for Owner {
    fn default() -> Self {
        Self {
            cache: Vec::new(),
            attached: Vec::new(),
            pending: None,
            next_scan: Instant::now(),
        }
    }
}

fn candidates(snapshot: &PresentationSnapshot) -> Vec<Binding> {
    let Some(context) = snapshot.selected else {
        return Vec::new();
    };
    if snapshot.active_account != Some(context.account)
        || snapshot.content == ContentState::Denied
        || !snapshot
            .conversations
            .as_slice()
            .iter()
            .any(|row| row.context == context && row.draft_clipboard == Decision::Allowed)
    {
        return Vec::new();
    }
    let now = history::now_unix();
    let mut out = Vec::new();
    for message in snapshot.messages.as_slice() {
        if message.metadata.deleted
            || message.metadata.system
            || message.metadata.truncated
            || message
                .metadata
                .expires_at
                .is_some_and(|at| at != 0 && at <= now)
            || !matches!(
                message.delivery,
                DeliveryState::Received | DeliveryState::ServerAccepted
            )
            || u64::try_from(message.local_id)
                .ok()
                .is_none_or(|id| id == 0)
        {
            continue;
        }
        let document = talk_core::ui::markup::parse(message.body.as_str());
        for (ordinal, link) in document.links.as_slice().iter().enumerate().take(2) {
            let Some(url) = ReferenceUrl::parse(link.destination.as_str()) else {
                continue;
            };
            if out.len() == 64 {
                return out;
            }
            out.push(Binding {
                context,
                message: message.local_id,
                ordinal: ordinal as u16,
                source: message.body.clone(),
                url,
            });
        }
    }
    out
}

fn transient(error: IdentityError) -> bool {
    matches!(
        error,
        IdentityError::Request(
            ErrorKind::Transport
                | ErrorKind::Deadline
                | ErrorKind::ServerUnavailable
                | ErrorKind::RateLimited
                | ErrorKind::Busy
        )
    )
}
impl Accounts {
    pub(super) fn retire_references(&mut self) {
        if let Some(pending) = self.reference_owner.pending.take()
            && let Ok(connection) = &mut self.connection
        {
            for request in &self.pending {
                if matches!(request.purpose, Purpose::Reference(_, _, _, generation) if generation == pending.generation)
                {
                    let _ = connection.cancel_request(request.id);
                }
            }
        }
        self.reference_owner.attached.clear();
        self.snapshot.reference_previews = Page::empty();
    }

    pub(super) fn prune_references(&mut self) {
        if self.reference_owner.attached.is_empty() && self.reference_owner.pending.is_none() {
            return;
        }
        let current = candidates(&self.snapshot);
        let before = self.reference_owner.attached.len();
        let now = Instant::now();
        self.reference_owner
            .attached
            .retain(|entry| entry.expires > now && current.contains(&entry.binding));
        if self
            .reference_owner
            .pending
            .as_ref()
            .is_some_and(|pending| !current.contains(&pending.binding))
        {
            if let Some(pending) = self.reference_owner.pending.take()
                && let Ok(connection) = &mut self.connection
            {
                for request in &self.pending {
                    if matches!(request.purpose, Purpose::Reference(_, _, _, generation) if generation == pending.generation)
                    {
                        let _ = connection.cancel_request(request.id);
                    }
                }
            }
        }
        if before != self.reference_owner.attached.len() {
            self.snapshot.reference_previews = page(
                self.reference_owner
                    .attached
                    .iter()
                    .map(|entry| entry.preview.clone()),
            );
            let previews = &self.snapshot.reference_previews;
            self.snapshot.content_panel.assets = page(
                self.snapshot
                    .content_panel
                    .assets
                    .as_slice()
                    .iter()
                    .filter(|asset| match &asset.target {
                        AssetTarget::Reference {
                            context,
                            message,
                            ordinal,
                        } => previews.as_slice().iter().any(|p| {
                            p.context == *context && p.message == *message && p.ordinal == *ordinal
                        }),
                        _ => true,
                    })
                    .cloned(),
            );
            self.retire_content_keys();
            self.touch();
        }
    }

    fn attach_reference(&mut self, binding: Binding, value: Resolved, expires: Instant) {
        if self.reference_owner.attached.len() >= 64
            || self
                .reference_owner
                .attached
                .iter()
                .any(|entry| entry.binding == binding)
        {
            return;
        }
        let preview = Preview {
            context: binding.context,
            message: binding.message,
            ordinal: binding.ordinal,
            title: value.title.clone(),
            description: value.description.clone(),
        };
        self.reference_owner.attached.push(Attached {
            binding: binding.clone(),
            preview,
            expires,
        });
        self.snapshot.reference_previews = page(
            self.reference_owner
                .attached
                .iter()
                .map(|entry| entry.preview.clone()),
        );
        if let Some(bytes) = value.thumbnail {
            let target = AssetTarget::Reference {
                context: binding.context,
                message: binding.message,
                ordinal: binding.ordinal,
            };
            self.set_asset(AssetView {
                target: target.clone(),
                state: AssetState::Loading,
                emoji: None,
                color: None,
                document_pages: None,
            });
            self.receive_image(binding.context.account, target, bytes, None, None);
        }
        self.touch();
    }

    fn cache_reference(
        &mut self,
        url: ReferenceUrl,
        mut value: Option<Resolved>,
        lifetime: Duration,
    ) -> Instant {
        if let Some(reference) = &mut value
            && reference
                .thumbnail
                .as_ref()
                .is_some_and(|image| image.as_slice().len() > CACHE_SINGLE_IMAGE)
        {
            reference.thumbnail = None;
        }
        self.reference_owner
            .cache
            .retain(|entry| entry.url != url && entry.expires > Instant::now());
        let size = value
            .as_ref()
            .and_then(|value| value.thumbnail.as_ref())
            .map_or(0, |image| image.as_slice().len());
        while self.reference_owner.cache.len() >= CACHE_ENTRIES
            || self
                .reference_owner
                .cache
                .iter()
                .filter_map(|entry| entry.value.as_ref()?.thumbnail.as_ref())
                .map(|image| image.as_slice().len())
                .sum::<usize>()
                + size
                > CACHE_IMAGE_BYTES
        {
            if self.reference_owner.cache.is_empty() {
                break;
            }
            self.reference_owner.cache.remove(0);
        }
        let expires = Instant::now() + lifetime;
        self.reference_owner.cache.push(Cached {
            url,
            value,
            expires,
        });
        expires
    }

    pub(super) fn tick_references(&mut self) {
        let now = Instant::now();
        if now < self.reference_owner.next_scan {
            return;
        }
        self.reference_owner.next_scan = now + Duration::from_millis(500);
        self.prune_references();
        let current = candidates(&self.snapshot);
        if let Some(pending) = self.reference_owner.pending.as_ref() {
            if pending.due.is_none_or(|due| now < due) {
                return;
            }
            let binding = pending.binding.clone();
            let generation = pending.generation;
            let deadline = pending.deadline;
            let Ok(sequence) = u64::try_from(binding.message) else {
                self.reference_owner.pending = None;
                return;
            };
            let command = ProtocolRequest::ResolveReference {
                message: protocol::MessageId {
                    conversation: binding.context.conversation,
                    sequence,
                },
                url: binding.url.clone(),
            };
            match self.request(
                binding.context.account,
                command,
                Purpose::Reference(
                    binding.context,
                    binding.message,
                    binding.ordinal,
                    generation,
                ),
            ) {
                Ok(()) => {
                    if let Some(pending) = &mut self.reference_owner.pending {
                        pending.due = None;
                    }
                }
                Err(SubmitError::Busy) if now < deadline => {
                    if let Some(pending) = &mut self.reference_owner.pending {
                        pending.due = Some(now + Duration::from_secs(1));
                    }
                }
                Err(_) => {
                    self.reference_owner.pending = None;
                    let _ = self.cache_reference(binding.url, None, NEGATIVE_LIFETIME);
                }
            }
            return;
        }
        self.reference_owner
            .cache
            .retain(|entry| entry.expires > now);
        for binding in current {
            if self
                .reference_owner
                .attached
                .iter()
                .any(|entry| entry.binding == binding)
            {
                continue;
            }
            if let Some((value, expires)) = self
                .reference_owner
                .cache
                .iter()
                .find(|entry| entry.url == binding.url)
                .map(|entry| (entry.value.clone(), entry.expires))
            {
                if let Some(value) = value {
                    self.attach_reference(binding, value, expires);
                }
                continue;
            }
            if self.pending.len() >= 8
                || !self.snapshot.accounts.as_slice().iter().any(|account| {
                    account.scope == binding.context.account
                        && account.connection == AccountConnectionState::Connected
                })
            {
                return;
            }
            let Some(generation) = self.reference_generation.checked_add(1) else {
                return;
            };
            self.reference_generation = generation;
            self.reference_owner.pending = Some(PendingReference {
                binding,
                generation,
                attempts: 0,
                due: Some(now),
                deadline: now + Duration::from_secs(30),
            });
            break;
        }
    }

    pub(super) fn complete_reference(
        &mut self,
        context: ConversationRef,
        message: u128,
        ordinal: u16,
        generation: u64,
        result: Result<RuntimeReply, IdentityError>,
    ) {
        let Some(pending) = self.reference_owner.pending.as_ref().filter(|pending| {
            pending.generation == generation
                && pending.binding.context == context
                && pending.binding.message == message
                && pending.binding.ordinal == ordinal
        }) else {
            return;
        };
        if !candidates(&self.snapshot).contains(&pending.binding) {
            self.reference_owner.pending = None;
            return;
        }
        if let Err(error) = result.as_ref()
            && transient(*error)
            && pending.attempts < 2
            && Instant::now() < pending.deadline
        {
            if let Some(pending) = &mut self.reference_owner.pending {
                pending.attempts += 1;
                pending.due = Some(Instant::now() + Duration::from_secs(1 << pending.attempts));
            }
            return;
        }
        let Some(pending) = self.reference_owner.pending.take() else {
            return;
        };
        let value = match result {
            Ok(RuntimeReply::Protocol(ProtocolReply::Reference(value))) => value,
            _ => None,
        };
        let expires = self.cache_reference(
            pending.binding.url.clone(),
            value.clone(),
            if value.is_some() {
                LIFETIME
            } else {
                NEGATIVE_LIFETIME
            },
        );
        if let Some(value) = value {
            self.attach_reference(pending.binding, value, expires);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fixture() -> (Accounts, Binding) {
        let mut model = super::super::tests::model();
        model.snapshot = crate::composition::fixtures::snapshot(Some("chat"));
        let context = model.snapshot.selected.unwrap();
        model.snapshot.content = ContentState::Ready;
        model.snapshot.conversations = page(
            model
                .snapshot
                .conversations
                .as_slice()
                .iter()
                .cloned()
                .map(|mut room| {
                    if room.context == context {
                        room.draft_clipboard = Decision::Allowed;
                    }
                    room
                }),
        );
        let mut message = model.snapshot.messages.as_slice()[0].clone();
        message.body = text("Read https://example.test/guide");
        message.metadata.deleted = false;
        message.metadata.system = false;
        message.delivery = DeliveryState::Received;
        model.snapshot.messages = page([message]);
        let binding = candidates(&model.snapshot).into_iter().next().unwrap();
        (model, binding)
    }
    fn value() -> Resolved {
        Resolved {
            title: text("Synthetic guide"),
            description: Some(text("Short description")),
            thumbnail: None,
        }
    }

    #[test]
    fn edited_or_revoked_messages_cannot_restore_a_stale_reference() {
        let (mut model, binding) = fixture();
        model.reference_owner.pending = Some(PendingReference {
            binding: binding.clone(),
            generation: 7,
            attempts: 0,
            due: None,
            deadline: Instant::now() + Duration::from_secs(30),
        });
        let mut changed = model.snapshot.messages.as_slice()[0].clone();
        changed.body = text("The link was removed");
        model.snapshot.messages = page([changed]);
        model.complete_reference(
            binding.context,
            binding.message,
            binding.ordinal,
            7,
            Ok(RuntimeReply::Protocol(ProtocolReply::Reference(Some(
                value(),
            )))),
        );
        assert!(model.snapshot.reference_previews.as_slice().is_empty());
        assert!(model.reference_owner.cache.is_empty());
        let (mut model, binding) = fixture();
        model.attach_reference(binding, value(), Instant::now() + LIFETIME);
        model.snapshot.content = ContentState::Denied;
        model.prune_references();
        assert!(model.snapshot.reference_previews.as_slice().is_empty());
    }

    #[test]
    fn cache_is_bounded_and_does_not_retain_large_thumbnails() {
        let (mut model, _) = fixture();
        let image = protocol::content::ImageBytes::new(vec![1; CACHE_SINGLE_IMAGE + 1]).unwrap();
        let mut result = value();
        result.thumbnail = Some(image);
        let url = ReferenceUrl::parse("https://example.test/large").unwrap();
        model.cache_reference(url, Some(result), LIFETIME);
        assert!(
            model.reference_owner.cache[0]
                .value
                .as_ref()
                .unwrap()
                .thumbnail
                .is_none()
        );
        for index in 0..(CACHE_ENTRIES + 4) {
            model.cache_reference(
                ReferenceUrl::parse(&format!("https://example.test/{index}")).unwrap(),
                None,
                NEGATIVE_LIFETIME,
            );
        }
        assert_eq!(model.reference_owner.cache.len(), CACHE_ENTRIES);
    }
}
