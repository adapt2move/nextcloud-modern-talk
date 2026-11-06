//! Explicitly selected local images use the existing bounded decoder lane.
use super::projection::preview_target;
use super::*;
use talk_core::ui::content::{AssetState, AssetTarget, AssetView};

fn image_name(name: &str) -> bool {
    name.rsplit_once('.').is_some_and(|(_, extension)| {
        ["png", "jpg", "jpeg", "gif", "webp"]
            .iter()
            .any(|e| extension.eq_ignore_ascii_case(e))
    })
}
impl Accounts {
    pub(in crate::composition::accounts) fn queue_upload_previews(&mut self) {
        if self.content_owner.images.len() >= 2 {
            return;
        }
        let Some(batch) = self.attachment_owner.batch.as_mut().filter(|b| {
            Some(b.context) == self.snapshot.selected
                && self.snapshot.active_account == Some(b.context.account)
        }) else {
            return;
        };
        let Some(file) = batch
            .files
            .iter_mut()
            .find(|f| !f.preview_queued && image_name(f.view.name.as_str()))
        else {
            return;
        };
        let Some(key) = self.content_owner.next_key.checked_add(1) else {
            return;
        };
        self.content_owner.next_key = key;
        file.preview_queued = true;
        let target = preview_target(batch.context, file.view.id);
        let job = content::ImageJob {
            cached: false,
            local: file
                .image
                .is_none()
                .then(|| (file.local.clone(), file.progress.clone())),
            account: batch.context.account,
            target: target.clone(),
            key,
            bytes: file.image.clone().unwrap_or_else(|| {
                protocol::content::ImageBytes::new(Vec::new()).expect("empty bounded input")
            }),
            attachment: None,
            preview_generation: None,
        };
        self.content_owner.images.push(job);
        self.set_asset(AssetView {
            target,
            state: AssetState::Loading,
            emoji: None,
            color: None,
            document_pages: None,
        });
    }

    pub(in crate::composition::accounts) fn upload_preview_current(
        &self,
        target: &AssetTarget,
        local: Option<&(Text<16384>, protocol::content::TransferProgress)>,
    ) -> bool {
        let AssetTarget::Thumbnail(reference) = target else {
            return false;
        };
        if reference.placeholder.as_str() != "upload"
            || !UploadView::is_message_id(reference.message)
            || self.snapshot.selected != Some(reference.context)
            || self.snapshot.active_account != Some(reference.context.account)
            || self.snapshot.content == ContentState::Denied
        {
            return false;
        }
        self.attachment_owner
            .batch
            .as_ref()
            .filter(|b| {
                b.context == reference.context && b.created.elapsed() < Duration::from_secs(30 * 60)
            })
            .is_some_and(|b| {
                b.files.iter().any(|f| {
                    f.view.message_id() == reference.message
                        && f.preview_queued
                        && !f.progress.is_cancelled()
                        && local.is_none_or(|(path, progress)| {
                            *path == f.local && *progress == f.progress
                        })
                })
            })
    }
}
