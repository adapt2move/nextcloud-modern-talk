//! One room-scoped voice player using the existing bounded native media owner.
use super::*;
use crate::composition::content_video::{self, Player};
use talk_adapters::identity::CachedAttachment;
use view::{AttachmentRef, PlaybackState, VoicePlaybackView};

struct Active {
    reference: AttachmentRef,
    descriptor: CachedAttachment,
    generation: u64,
    player: Player,
    speed: u8,
}
struct Pending {
    reference: AttachmentRef,
    descriptor: CachedAttachment,
    generation: u64,
    bytes: wire::VideoBytes,
    deadline: Instant,
}
#[derive(Default)]
pub(super) struct Owner {
    generation: u64,
    active: Option<Active>,
    pending: Option<Pending>,
}
#[derive(Clone, Copy)]
pub(super) enum Control {
    Pause,
    Seek(u64),
    Speed(u8),
}

/// An ordinary attachment the server describes as audio.
fn playable_audio(object: &talk_core::ui::messaging::RichObject) -> bool {
    object.mime_type.as_ref().is_some_and(|mime| {
        mime.as_str().starts_with("audio/") || mime.as_str() == "application/ogg"
    })
}

impl Accounts {
    fn voice_descriptor(&self, reference: &AttachmentRef) -> Option<CachedAttachment> {
        if self.snapshot.active_account != Some(reference.context.account)
            || self.snapshot.selected != Some(reference.context)
        {
            return None;
        }
        let message = self
            .snapshot
            .messages
            .as_slice()
            .iter()
            .chain(
                self.snapshot
                    .message_panel
                    .as_ref()
                    .filter(|p| p.context == reference.context)
                    .into_iter()
                    .flat_map(|p| p.thread_messages.as_slice()),
            )
            .find(|m| {
                m.local_id == reference.message
                    && !m.metadata.deleted
                    && m.metadata
                        .expires_at
                        .is_none_or(|at| at == 0 || at > history::now_unix())
            })?;
        let object = message
            .metadata
            .rich
            .as_slice()
            .iter()
            .find(|o| o.kind.as_str() == "file" && o.placeholder == reference.placeholder)?;
        // A voice message, or an ordinary attachment the server calls audio.
        if !message.metadata.voice_message && !playable_audio(object) {
            return None;
        }
        Some(CachedAttachment {
            placeholder: object.placeholder.clone(),
            name: object.name.clone(),
            mime_type: object.mime_type.clone(),
            size: object.size,
            id: object.id.clone(),
        })
    }
    fn voice_message_current(
        &self,
        reference: &AttachmentRef,
        descriptor: &CachedAttachment,
    ) -> bool {
        self.snapshot.selected == Some(reference.context)
            && self.media.capture().is_none()
            && self.snapshot.active_account == Some(reference.context.account)
            && self.snapshot.content == ContentState::Ready
            && self.current(reference.context.account).is_ok()
            && self.voice_descriptor(reference).as_ref() == Some(descriptor)
            && self
                .snapshot
                .messages
                .as_slice()
                .iter()
                .chain(
                    self.snapshot
                        .message_panel
                        .as_ref()
                        .into_iter()
                        .flat_map(|p| p.thread_messages.as_slice()),
                )
                .any(|message| {
                    message.local_id == reference.message
                        && !message.metadata.deleted
                        && message.metadata.rich.as_slice().iter().any(|object| {
                            object.placeholder == reference.placeholder
                                && object.kind.as_str() == "file"
                                && (message.metadata.voice_message || playable_audio(object))
                        })
                })
    }
    pub(super) fn voice_job_current(
        &self,
        reference: &AttachmentRef,
        generation: u64,
        descriptor: Option<&CachedAttachment>,
    ) -> bool {
        generation == self.content_owner.voice.generation
            && descriptor.is_some_and(|d| self.voice_message_current(reference, d))
            && self
                .snapshot
                .content_panel
                .voice_playback
                .as_ref()
                .is_some_and(|v| v.attachment == *reference)
    }
    pub(in crate::composition) fn stop_voice_player(&mut self) {
        let owner = &mut self.content_owner.voice;
        owner.generation = owner.generation.saturating_add(1);
        owner.pending = None;
        if let Some(active) = &owner.active {
            active.player.stop();
        }
        self.snapshot.content_panel.voice_playback = None;
    }
    pub(super) fn play_voice(
        &mut self,
        request: RequestId,
        reference: AttachmentRef,
    ) -> Result<(), SubmitError> {
        self.require_attachment(&reference)?;
        if let Some(active) = self.content_owner.voice.active.as_ref()
            && active.reference == reference
            && self.voice_message_current(&reference, &active.descriptor)
        {
            return active
                .player
                .set_playing(true)
                .map_err(|_| SubmitError::Unavailable(UnavailableReason::Unsupported));
        }
        let descriptor = self
            .voice_descriptor(&reference)
            .ok_or(SubmitError::StaleScope)?;
        if !self.voice_message_current(&reference, &descriptor) {
            return Err(SubmitError::StaleScope);
        }
        self.stop_video();
        self.stop_voice_player();
        let generation = self.content_owner.voice.generation;
        self.start_content(
            Job {
                request,
                account: reference.context.account,
                context: Some(reference.context),
                operation: OperationId::from_local(0),
                target: None,
                poll: None,
                mutation: false,
                cloud: false,
                attachment: Some(descriptor),
                preview_generation: None,
                avatar_actor: None,
                voice: Some((reference.clone(), generation)),
            },
            wire::ContentCommand::VoiceAudio {
                message: mid(&reference)?,
                placeholder: reference.placeholder.clone(),
            },
        )?;
        self.snapshot.content_panel.voice_playback = Some(VoicePlaybackView {
            attachment: reference,
            state: PlaybackState::Loading,
            position_ms: 0,
            duration_ms: None,
            speed_half_steps: 2,
            stop_unconfirmed: false,
        });
        self.touch();
        Ok(())
    }
    pub(super) fn voice_control(
        &mut self,
        reference: AttachmentRef,
        control: Control,
    ) -> Result<(), SubmitError> {
        let active = self
            .content_owner
            .voice
            .active
            .as_ref()
            .filter(|a| a.reference == reference)
            .ok_or(SubmitError::StaleScope)?;
        if !self.voice_message_current(&reference, &active.descriptor) {
            return Err(SubmitError::StaleScope);
        }
        match control {
            Control::Pause => active.player.set_playing(false),
            Control::Seek(ms) => active.player.seek_ms(ms),
            Control::Speed(speed) => active.player.set_rate(speed),
        }
        .map_err(|_| SubmitError::Unavailable(UnavailableReason::Unsupported))?;
        if let Control::Speed(speed) = control
            && let Some(active) = self.content_owner.voice.active.as_mut()
        {
            active.speed = speed;
        }
        Ok(())
    }
    pub(super) fn finish_voice_download(
        &mut self,
        reference: AttachmentRef,
        generation: u64,
        descriptor: Option<CachedAttachment>,
        result: Result<wire::VideoBytes, IdentityError>,
    ) {
        match (descriptor, result) {
            (Some(descriptor), Ok(bytes)) => {
                self.content_owner.voice.pending = Some(Pending {
                    reference,
                    descriptor,
                    generation,
                    bytes,
                    deadline: Instant::now() + Duration::from_secs(10),
                })
            }
            (_, Err(error)) => self.fail_voice(&reference, error_reason(error)),
            _ => self.fail_voice(&reference, UnavailableReason::Unsupported),
        }
        self.touch();
    }
    fn fail_voice(&mut self, reference: &AttachmentRef, reason: UnavailableReason) {
        if let Some(view) = self
            .snapshot
            .content_panel
            .voice_playback
            .as_mut()
            .filter(|v| v.attachment == *reference)
        {
            view.state = PlaybackState::Failed(reason);
        }
        self.touch();
    }
    pub(super) fn tick_voice_player(&mut self) {
        if let Some(active) = &self.content_owner.voice.active {
            if !self.voice_job_current(
                &active.reference,
                active.generation,
                Some(&active.descriptor),
            ) {
                active.player.stop();
            }
            let snapshot = active.player.snapshot();
            if self.voice_job_current(
                &active.reference,
                active.generation,
                Some(&active.descriptor),
            ) {
                let state = match snapshot.phase {
                    content_video::Phase::Starting => PlaybackState::Loading,
                    content_video::Phase::Playing => PlaybackState::Playing,
                    content_video::Phase::Paused => PlaybackState::Paused,
                    content_video::Phase::Ended => PlaybackState::Ended,
                    content_video::Phase::Stopping | content_video::Phase::Stopped => {
                        PlaybackState::Stopping
                    }
                    content_video::Phase::Failed(_) => {
                        PlaybackState::Failed(UnavailableReason::Unsupported)
                    }
                };
                let value = VoicePlaybackView {
                    attachment: active.reference.clone(),
                    state,
                    position_ms: snapshot.position_ms,
                    duration_ms: snapshot.duration_ms,
                    speed_half_steps: active.speed,
                    stop_unconfirmed: snapshot.stop_unconfirmed,
                };
                if self.snapshot.content_panel.voice_playback.as_ref() != Some(&value) {
                    self.snapshot.content_panel.voice_playback = Some(value);
                    self.touch();
                }
            }
        }
        if self
            .content_owner
            .voice
            .active
            .as_mut()
            .is_some_and(|a| a.player.reap())
        {
            self.content_owner.voice.active = None;
        }
        if self.content_owner.voice.active.is_some() {
            return;
        }
        let Some(pending) = self.content_owner.voice.pending.take() else {
            return;
        };
        if !self.voice_job_current(
            &pending.reference,
            pending.generation,
            Some(&pending.descriptor),
        ) {
            return;
        }
        if Instant::now() >= pending.deadline {
            self.fail_voice(&pending.reference, UnavailableReason::Busy);
            return;
        }
        match Player::start_audio(pending.bytes.clone()) {
            Ok(player) => {
                if player.set_playing(true).is_err() {
                    self.fail_voice(&pending.reference, UnavailableReason::Unsupported);
                    return;
                }
                self.content_owner.voice.active = Some(Active {
                    reference: pending.reference,
                    descriptor: pending.descriptor,
                    generation: pending.generation,
                    player,
                    speed: 2,
                });
            }
            Err(content_video::Error::Busy) => self.content_owner.voice.pending = Some(pending),
            Err(_) => self.fail_voice(&pending.reference, UnavailableReason::Unsupported),
        }
    }
}
