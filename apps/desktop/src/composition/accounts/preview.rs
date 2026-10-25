//! One scoped gallery player. Pixels retire immediately; native stop settles
//! before replacement. Epochs also distinguish closing and reopening one file.
use super::*;
use crate::composition::content_video::{self, Player};
use talk_adapters::identity::CachedAttachment;
use talk_core::protocol::content::VideoBytes;
use talk_core::ui::content::{
    AssetState, AssetTarget, AttachmentRef, ContentAction, PlaybackState, PlaybackView,
};

struct Active {
    scope: AccountScope,
    reference: AttachmentRef,
    attachment: CachedAttachment,
    generation: u64,
    key: u64,
    player: Player,
}
struct Pending {
    scope: AccountScope,
    reference: AttachmentRef,
    attachment: CachedAttachment,
    generation: u64,
    bytes: VideoBytes,
    deadline: Instant,
}
#[derive(Default)]
pub(super) struct Video {
    active: Option<Active>,
    pending: Option<Pending>,
}
pub(in crate::composition) struct FrameReceipt {
    pub account: AccountScope,
    pub reference: AttachmentRef,
    pub key: u64,
    generation: u64,
    bytes: usize,
}
pub(in crate::composition) struct FrameJob {
    pub receipt: FrameReceipt,
    pub frame: content_video::Frame,
}
impl Accounts {
    pub(super) fn stop_video(&mut self) {
        self.content_owner.video.pending = None;
        if let Some(active) = &self.content_owner.video.active {
            active.player.stop();
        }
    }
    pub(super) fn stage_video(
        &mut self,
        reference: AttachmentRef,
        bytes: VideoBytes,
        attachment: CachedAttachment,
        generation: u64,
    ) {
        self.stop_video();
        self.content_owner.video.pending = Some(Pending {
            scope: reference.context.account,
            reference: reference.clone(),
            attachment,
            generation,
            bytes,
            deadline: Instant::now() + Duration::from_secs(10),
        });
        self.snapshot.content_panel.playback = Some(PlaybackView {
            attachment: reference,
            generation,
            state: PlaybackState::Loading,
            position_ms: 0,
            duration_ms: None,
            stop_unconfirmed: false,
        });
        self.tick_video();
    }
    fn video_is_current(
        &self,
        scope: AccountScope,
        reference: &AttachmentRef,
        descriptor: &CachedAttachment,
        generation: u64,
    ) -> bool {
        generation == self.content_owner.preview_generation
            && self.snapshot.active_account == Some(scope)
            && self.current(scope).is_ok()
            && self.snapshot.selected == Some(reference.context)
            && self.snapshot.content == ContentState::Ready
            && self
                .snapshot
                .accounts
                .as_slice()
                .iter()
                .find(|a| a.scope == scope)
                .is_some_and(|a| {
                    a.connection_failure.is_none()
                        || a.connection_failure == Some(ConnectionFailure::RateLimited)
                })
            && self.snapshot.content_panel.preview.as_ref() == Some(reference)
            && self.current_attachment_descriptor(reference).as_ref() == Some(descriptor)
            && self.asset_is_current(&AssetTarget::Image(reference.clone()))
    }
    fn fail_video_view(
        &mut self,
        reference: &AttachmentRef,
        generation: u64,
        reason: UnavailableReason,
        unconfirmed: bool,
    ) {
        if self.content_owner.preview_generation != generation
            || self.snapshot.content_panel.preview.as_ref() != Some(reference)
        {
            return;
        }
        self.snapshot.content_panel.playback = Some(PlaybackView {
            attachment: reference.clone(),
            generation,
            state: PlaybackState::Failed(reason),
            position_ms: 0,
            duration_ms: None,
            stop_unconfirmed: unconfirmed,
        });
        self.set_asset(talk_core::ui::content::AssetView {
            target: AssetTarget::Image(reference.clone()),
            state: AssetState::Unsupported,
            emoji: None,
            color: None,
            document_pages: None,
        });
        self.touch();
    }
    pub(super) fn tick_video(&mut self) {
        let mut retire = None;
        if let Some(active) = &self.content_owner.video.active {
            let valid = self.video_is_current(
                active.scope,
                &active.reference,
                &active.attachment,
                active.generation,
            );
            if !valid {
                active.player.stop();
                retire = Some(active.key);
            }
            let snapshot = active.player.snapshot();
            if valid
                || self
                    .snapshot
                    .content_panel
                    .playback
                    .as_ref()
                    .is_some_and(|p| {
                        p.generation == active.generation && p.attachment == active.reference
                    })
            {
                let state = if !valid {
                    PlaybackState::Stopping
                } else {
                    match snapshot.phase {
                        content_video::Phase::Starting => PlaybackState::Loading,
                        content_video::Phase::Paused => PlaybackState::Paused,
                        content_video::Phase::Playing => PlaybackState::Playing,
                        content_video::Phase::Ended => PlaybackState::Ended,
                        content_video::Phase::Stopping | content_video::Phase::Stopped => {
                            PlaybackState::Stopping
                        }
                        content_video::Phase::Failed(error) => {
                            PlaybackState::Failed(video_error(error))
                        }
                    }
                };
                let value = PlaybackView {
                    attachment: active.reference.clone(),
                    generation: active.generation,
                    state,
                    position_ms: snapshot.position_ms,
                    duration_ms: snapshot.duration_ms,
                    stop_unconfirmed: snapshot.stop_unconfirmed,
                };
                if self.snapshot.content_panel.playback.as_ref() != Some(&value) {
                    self.snapshot.content_panel.playback = Some(value);
                    self.touch();
                }
            }
        }
        if let Some(key) = retire {
            self.snapshot.content_panel.assets = page(
                self.snapshot
                    .content_panel
                    .assets
                    .as_slice()
                    .iter()
                    .filter(|a| a.state != AssetState::Ready { key })
                    .cloned(),
            );
            self.content_owner.retained.retain(|(id, _)| *id != key);
            self.touch();
        }
        if self
            .content_owner
            .video
            .active
            .as_mut()
            .is_some_and(|a| a.player.reap())
            && let Some(active) = self.content_owner.video.active.take()
        {
            self.fail_video_view(
                &active.reference,
                active.generation,
                UnavailableReason::Unconfigured,
                false,
            );
        }
        if self
            .content_owner
            .video
            .pending
            .as_ref()
            .is_some_and(|p| Instant::now() >= p.deadline)
            && let Some(pending) = self.content_owner.video.pending.take()
        {
            let unconfirmed = self
                .content_owner
                .video
                .active
                .as_ref()
                .is_some_and(|a| a.player.snapshot().stop_unconfirmed);
            self.fail_video_view(
                &pending.reference,
                pending.generation,
                UnavailableReason::Busy,
                unconfirmed,
            );
        }
        if self.content_owner.video.active.is_some() {
            return;
        }
        let Some(pending) = self.content_owner.video.pending.take() else {
            return;
        };
        if !self.video_is_current(
            pending.scope,
            &pending.reference,
            &pending.attachment,
            pending.generation,
        ) {
            self.fail_video_view(
                &pending.reference,
                pending.generation,
                UnavailableReason::PolicyStale,
                false,
            );
            return;
        }
        match Player::start(pending.bytes.clone()) {
            Ok(player) => {
                let Some(key) = self.content_owner.next_key.checked_add(1) else {
                    player.stop();
                    self.fail_video_view(
                        &pending.reference,
                        pending.generation,
                        UnavailableReason::Busy,
                        false,
                    );
                    return;
                };
                self.content_owner.next_key = key;
                self.content_owner.video.active = Some(Active {
                    scope: pending.scope,
                    reference: pending.reference,
                    attachment: pending.attachment,
                    generation: pending.generation,
                    key,
                    player,
                });
            }
            Err(content_video::Error::Busy) if Instant::now() < pending.deadline => {
                self.content_owner.video.pending = Some(pending)
            }
            Err(error) => self.fail_video_view(
                &pending.reference,
                pending.generation,
                video_error(error),
                false,
            ),
        }
    }
    pub(super) fn video_control(&mut self, action: ContentAction) -> Result<(), SubmitError> {
        let (reference, generation) = match &action {
            ContentAction::PlayPreview {
                attachment,
                generation,
            }
            | ContentAction::PausePreview {
                attachment,
                generation,
            }
            | ContentAction::SeekPreview {
                attachment,
                generation,
                ..
            } => (attachment, *generation),
            _ => return Err(SubmitError::Unavailable(UnavailableReason::Unconfigured)),
        };
        let active = self
            .content_owner
            .video
            .active
            .as_ref()
            .ok_or(SubmitError::Unavailable(UnavailableReason::Unconfigured))?;
        if reference != &active.reference
            || generation != active.generation
            || !self.video_is_current(
                active.scope,
                &active.reference,
                &active.attachment,
                active.generation,
            )
        {
            return Err(SubmitError::StaleScope);
        }
        match action {
            ContentAction::PlayPreview { .. } => {
                if active.player.snapshot().phase == content_video::Phase::Ended {
                    active
                        .player
                        .seek_ms(0)
                        .map_err(|e| SubmitError::Unavailable(video_error(e)))?;
                }
                active.player.set_playing(true)
            }
            ContentAction::PausePreview { .. } => active.player.set_playing(false),
            ContentAction::SeekPreview { position_ms, .. } => active.player.seek_ms(position_ms),
            _ => return Err(SubmitError::Unavailable(UnavailableReason::Unconfigured)),
        }
        .map_err(|e| SubmitError::Unavailable(video_error(e)))
    }
    pub(crate) fn video_active(&self) -> bool {
        self.content_owner.video.active.as_ref().is_some_and(|a| {
            let state = a.player.snapshot();
            !state.stop_unconfirmed
                && matches!(
                    state.phase,
                    content_video::Phase::Starting
                        | content_video::Phase::Playing
                        | content_video::Phase::Stopping
                )
        })
    }
    pub(in crate::composition) fn take_video_frame(&mut self) -> Option<FrameJob> {
        let active = self.content_owner.video.active.as_ref()?;
        if !self.video_is_current(
            active.scope,
            &active.reference,
            &active.attachment,
            active.generation,
        ) || !matches!(
            active.player.snapshot().phase,
            content_video::Phase::Paused
                | content_video::Phase::Playing
                | content_video::Phase::Ended
        ) {
            return None;
        }
        let frame = active.player.take_frame()?;
        Some(FrameJob {
            receipt: FrameReceipt {
                account: active.scope,
                reference: active.reference.clone(),
                key: active.key,
                generation: active.generation,
                bytes: frame.bgra.len(),
            },
            frame,
        })
    }
    pub(in crate::composition) fn finish_video_frame(&mut self, receipt: &FrameReceipt) -> bool {
        let Some(active) = self.content_owner.video.active.as_ref() else {
            return false;
        };
        if active.key != receipt.key
            || active.generation != receipt.generation
            || active.reference != receipt.reference
            || !self.video_is_current(
                active.scope,
                &active.reference,
                &active.attachment,
                active.generation,
            )
            || !matches!(
                active.player.snapshot().phase,
                content_video::Phase::Paused
                    | content_video::Phase::Playing
                    | content_video::Phase::Ended
            )
        {
            return false;
        }
        self.content_owner
            .retained
            .retain(|(key, _)| *key != receipt.key);
        self.content_owner
            .retained
            .push((receipt.key, receipt.bytes));
        self.set_asset(talk_core::ui::content::AssetView {
            target: AssetTarget::Image(receipt.reference.clone()),
            state: AssetState::Ready { key: receipt.key },
            emoji: None,
            color: None,
            document_pages: None,
        });
        self.touch();
        true
    }
}
impl Video {
    pub(super) fn stopped(&self) -> bool {
        self.active.as_ref().is_none_or(|a| a.player.is_stopped()) && self.pending.is_none()
    }
}
fn video_error(error: content_video::Error) -> UnavailableReason {
    match error {
        content_video::Error::Busy => UnavailableReason::Busy,
        _ => UnavailableReason::Unconfigured,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn pending(model: &Accounts, generation: u64) -> Pending {
        let reference = model.snapshot.content_panel.preview.clone().unwrap();
        Pending {
            scope: reference.context.account,
            reference: reference.clone(),
            attachment: CachedAttachment {
                placeholder: reference.placeholder,
                name: text("synthetic.mp4"),
                mime_type: Some(text("video/mp4")),
                size: Some(12),
                id: None,
            },
            generation,
            bytes: VideoBytes::new(vec![0; 12]).unwrap(),
            deadline: Instant::now() + Duration::from_secs(10),
        }
    }
    fn model() -> Accounts {
        let mut model = super::super::tests::model();
        model.snapshot = super::super::super::fixtures::snapshot(Some("chat"));
        model.content_owner.preview_generation = 3;
        model.snapshot.content_panel.preview = Some(AttachmentRef {
            context: model.snapshot.selected.unwrap(),
            message: 7,
            placeholder: text("video"),
        });
        model
    }

    #[test]
    fn obsolete_video_failure_cannot_replace_a_reopened_file_or_its_pixels() {
        let mut model = model();
        let reference = model.snapshot.content_panel.preview.clone().unwrap();
        let target = AssetTarget::Image(reference.clone());
        model.set_asset(talk_core::ui::content::AssetView {
            target: target.clone(),
            state: AssetState::Ready { key: 9 },
            emoji: None,
            color: None,
            document_pages: None,
        });
        model.fail_video_view(&reference, 2, UnavailableReason::Busy, true);
        assert_eq!(model.snapshot.content_panel.playback, None);
        assert_eq!(
            model.snapshot.content_panel.assets.as_slice()[0].state,
            AssetState::Ready { key: 9 }
        );
        model.fail_video_view(&reference, 3, UnavailableReason::Busy, true);
        let value = model.snapshot.content_panel.playback.as_ref().unwrap();
        assert_eq!(value.generation, 3);
        assert_eq!(value.state, PlaybackState::Failed(UnavailableReason::Busy));
        assert!(value.stop_unconfirmed);
    }

    #[test]
    fn invalid_or_timed_out_pending_video_settles_loading_without_starting_a_worker() {
        for timed_out in [false, true] {
            let mut model = model();
            let mut work = pending(&model, 3);
            if timed_out {
                work.deadline = Instant::now();
            }
            model.content_owner.video.pending = Some(work);
            // The fixture has no account runtime. Its queued media cannot
            // acquire a player or remain falsely loading after validation.
            model.tick_video();
            assert!(model.content_owner.video.active.is_none());
            assert!(model.content_owner.video.pending.is_none());
            assert!(matches!(
                model
                    .snapshot
                    .content_panel
                    .playback
                    .as_ref()
                    .unwrap()
                    .state,
                PlaybackState::Failed(_)
            ));
            assert!(model.content_owner.video.stopped());
        }
    }
}
