//! Offline preview admission binds persisted stable room/message identities to
//! the currently selected presentation. Decoding remains with composition's
//! existing bounded worker; cached pixels confer no protocol authority.
use super::*;
use talk_adapters::identity::{
    CACHE_THUMBNAIL_BYTES, CACHE_THUMBNAIL_LIFETIME, CacheThumbnail, CachedAttachment,
};
use talk_core::ui::presentation::{MessageView, PresentationSnapshot};

fn image_message<'a>(
    snapshot: &'a PresentationSnapshot,
    attachment: &view::AttachmentRef,
    now: u64,
) -> Option<&'a MessageView> {
    let valid = |message: &&MessageView| {
        message.local_id == attachment.message
            && !message.metadata.deleted
            && message
                .metadata
                .expires_at
                .is_none_or(|at| at == 0 || at > now)
            && message.metadata.rich.as_slice().iter().any(|rich| {
                rich.kind.as_str() == "file"
                    && rich.placeholder == attachment.placeholder
                    && rich.mime_type.as_ref().is_some_and(|mime| {
                        mime.as_str().starts_with("image/")
                            || mime.as_str().starts_with("video/")
                            || mime.as_str() == "application/pdf"
                    })
            })
    };
    if let Some(message) = snapshot
        .messages
        .as_slice()
        .iter()
        .find(|m| m.local_id == attachment.message)
    {
        return valid(&message).then_some(message);
    }
    {
        snapshot
            .message_panel
            .as_ref()
            .filter(|p| p.context == attachment.context)
            .and_then(|panel| {
                let mut rows = panel
                    .shared_files
                    .as_slice()
                    .iter()
                    .chain(panel.thread_messages.as_slice())
                    .chain(panel.pinned_messages.as_slice())
                    .filter(|m| m.local_id == attachment.message);
                let first = rows.next()?;
                let canonical = descriptor(first, &attachment.placeholder);
                if !valid(&first)
                    || rows.any(|m| {
                        !valid(&m)
                            || descriptor(m, &attachment.placeholder) != canonical
                            || m.metadata.expires_at != first.metadata.expires_at
                    })
                {
                    return None;
                }
                Some(first)
            })
    }
}
fn descriptor(message: &MessageView, placeholder: &Text<128>) -> Option<CachedAttachment> {
    let rich = message
        .metadata
        .rich
        .as_slice()
        .iter()
        .find(|r| r.kind.as_str() == "file" && r.placeholder == *placeholder)?;
    Some(CachedAttachment {
        placeholder: rich.placeholder.clone(),
        name: rich.name.clone(),
        mime_type: rich.mime_type.clone(),
        size: rich.size,
        id: rich.id.clone(),
    })
}

impl Accounts {
    pub(crate) fn current_attachment_descriptor(
        &self,
        attachment: &view::AttachmentRef,
    ) -> Option<CachedAttachment> {
        if self.snapshot.active_account != Some(attachment.context.account)
            || self.snapshot.selected != Some(attachment.context)
        {
            return None;
        }
        let message = image_message(&self.snapshot, attachment, history::now_unix())?;
        descriptor(message, &attachment.placeholder)
    }
    pub(crate) fn cached_thumbnail_current(&self, target: &view::AssetTarget) -> bool {
        let view::AssetTarget::Thumbnail(attachment) = target else {
            return false;
        };
        let Some(message) = image_message(&self.snapshot, attachment, history::now_unix()) else {
            return false;
        };
        let Some(current) = descriptor(message, &attachment.placeholder) else {
            return false;
        };
        let Some(room) = self.cache.stable_room(attachment.context) else {
            return false;
        };
        let now = history::now_unix();
        self.snapshot.active_account == Some(attachment.context.account)
            && self.snapshot.selected == Some(attachment.context)
            && self.snapshot.content != ContentState::Denied
            && self.cache.persistable(attachment.context)
            && self
                .cache
                .loaded(attachment.context.account.account)
                .is_some_and(|snapshot| {
                    snapshot.thumbnails.as_slice().iter().any(|t| {
                        t.room == room
                            && t.saved_at <= now
                            && now - t.saved_at < CACHE_THUMBNAIL_LIFETIME
                            && u128::from(t.message) == attachment.message
                            && t.placeholder == attachment.placeholder
                            && t.attachment == current
                            && t.expires_at.is_none_or(|at| at == 0 || at > now)
                    })
                })
    }
    pub(crate) fn cached_thumbnail_job_current(&self, job: &ImageJob) -> bool {
        if !job.cached || !self.cached_thumbnail_current(&job.target) {
            return false;
        }
        let Some(context) = job.target.context() else {
            return false;
        };
        let Some(room) = self.cache.stable_room(context) else {
            return false;
        };
        let view::AssetTarget::Thumbnail(a) = &job.target else {
            return false;
        };
        if job.attachment.as_ref() != self.current_attachment_descriptor(a).as_ref() {
            return false;
        }
        self.cache
            .loaded(context.account.account)
            .is_some_and(|snapshot| {
                snapshot.thumbnails.as_slice().iter().any(|t| {
                    t.room == room
                        && u128::from(t.message) == a.message
                        && t.placeholder == a.placeholder
                        && t.bytes.as_ref() == job.bytes.as_slice()
                })
            })
    }
    pub(crate) fn forget_thumbnail(&mut self, attachment: &view::AttachmentRef) {
        self.cache.forget_thumbnail(attachment);
    }
    pub(crate) fn hydrate_thumbnails(&mut self) {
        let Some(context) = self.snapshot.selected else {
            return;
        };
        if self.snapshot.active_account != Some(context.account)
            || self.snapshot.content == ContentState::Denied
            || !self.cache.persistable(context)
        {
            return;
        }
        let Some(room) = self.cache.stable_room(context) else {
            return;
        };
        let now = history::now_unix();
        let Some(snapshot) = self.cache.loaded(context.account.account) else {
            return;
        };
        let mut candidates = Vec::with_capacity(2);
        for thumbnail in snapshot.thumbnails.as_slice() {
            if self.content_owner.images.len() + candidates.len() >= 2 {
                break;
            }
            if thumbnail.room != room
                || thumbnail.bytes.is_empty()
                || thumbnail.bytes.len() > CACHE_THUMBNAIL_BYTES
                || thumbnail.saved_at > now
                || now - thumbnail.saved_at >= CACHE_THUMBNAIL_LIFETIME
                || thumbnail.expires_at.is_some_and(|at| at > 0 && at <= now)
            {
                continue;
            }
            let attachment = view::AttachmentRef {
                context,
                message: u128::from(thumbnail.message),
                placeholder: thumbnail.placeholder.clone(),
            };
            if image_message(&self.snapshot, &attachment, now)
                .and_then(|message| descriptor(message, &attachment.placeholder))
                .as_ref()
                != Some(&thumbnail.attachment)
            {
                continue;
            }
            let target = view::AssetTarget::Thumbnail(attachment);
            // Loading/ready and every terminal target observation retain their
            // own authority. Disk pixels never override an explicit refusal.
            if self.snapshot.content_panel.account == Some(context.account)
                && self
                    .snapshot
                    .content_panel
                    .assets
                    .as_slice()
                    .iter()
                    .any(|a| a.target == target)
                || self
                    .content_owner
                    .jobs
                    .iter()
                    .any(|job| job.target.as_ref() == Some(&target))
                || self
                    .content_owner
                    .images
                    .iter()
                    .any(|job| job.target == target)
            {
                continue;
            }
            candidates.push((
                target,
                thumbnail.bytes.clone(),
                thumbnail.attachment.clone(),
            ));
        }
        if candidates.is_empty() {
            return;
        }
        if self.snapshot.content_panel.account != Some(context.account) {
            self.snapshot.content_panel = view::ContentPanel {
                account: Some(context.account),
                context: Some(context),
                ..Default::default()
            };
            self.retire_content_keys();
        } else {
            self.snapshot.content_panel.context = Some(context);
        }
        for (target, bytes, attachment) in candidates {
            let Some(key) = self.content_owner.next_key.checked_add(1) else {
                break;
            };
            let Ok(bytes) = wire::ImageBytes::new(bytes.to_vec()) else {
                continue;
            };
            self.content_owner.next_key = key;
            self.set_asset(view::AssetView {
                target: target.clone(),
                state: view::AssetState::Loading,
                emoji: None,
                color: None,
                document_pages: None,
            });
            self.content_owner.images.push(ImageJob {
                preview_generation: None,
                cached: true,
                account: context.account,
                target,
                key,
                bytes,
                attachment: Some(attachment),
            });
        }
        self.touch();
    }

    pub(crate) fn persist_thumbnail(&mut self, job: &ImageJob, encoded: Vec<u8>) {
        let view::AssetTarget::Thumbnail(attachment) = &job.target else {
            return;
        };
        if job.cached
            || encoded.is_empty()
            || encoded.len() > CACHE_THUMBNAIL_BYTES
            || self.snapshot.active_account != Some(job.account)
            || self.snapshot.selected != Some(attachment.context)
            || self.snapshot.content == ContentState::Denied
            || !self.cache.persistable(attachment.context)
            || self.cache.is_local(job.account)
            || self.cache.is_presentation(attachment.context)
            || !self
                .snapshot
                .content_panel
                .assets
                .as_slice()
                .iter()
                .any(|a| {
                    a.target == job.target && a.state == (view::AssetState::Ready { key: job.key })
                })
        {
            return;
        }
        let now = history::now_unix();
        let Some(message) = image_message(&self.snapshot, attachment, now) else {
            return;
        };
        if job.attachment.as_ref() != descriptor(message, &attachment.placeholder).as_ref() {
            return;
        }
        let Some(room) = self.cache.stable_room(attachment.context) else {
            return;
        };
        let Ok(message_id) = u64::try_from(message.local_id) else {
            return;
        };
        let thumbnail = CacheThumbnail {
            room,
            message: message_id,
            placeholder: attachment.placeholder.clone(),
            bytes: encoded.into(),
            saved_at: now,
            expires_at: message.metadata.expires_at,
            attachment: match descriptor(message, &attachment.placeholder) {
                Some(value) => value,
                None => return,
            },
        };
        self.cache.remember_thumbnail(attachment.context, thumbnail);
    }
}
