//! Worker lifetime and the memory-only presentation adapter.
use crate::devices::DeviceRuntime;
use crate::presentation::preferences::ComputerSettingsPort;
use crate::settings::{SettingsRuntime, notifications::NotificationRuntime};
use std::rc::Rc;
use talk_core::preferences::devices::{DeviceError, DeviceIntent, DeviceSnapshot};
use talk_core::preferences::notifications::{
    NotificationAccessAction, NotificationAccessSnapshot, NotificationCategory, NotificationError,
    NotificationEvent, NotificationState,
};
use talk_core::preferences::{
    Locale, PreferenceCommand, PreferenceError, PreferenceSnapshot, RequestId,
};
use talk_core::ui::Text;

pub struct Services {
    shutdown_deadline: std::cell::Cell<Option<std::time::Instant>>,
    shutdown_unconfirmed_reported: std::cell::Cell<bool>,
    pub(super) install: std::cell::RefCell<super::updates::Install>,
    notification_sequence: std::cell::Cell<u64>,
    voice_sequence: std::cell::Cell<u64>,
    voice_player: std::cell::RefCell<
        Option<(
            talk_core::ui::presentation::ConversationRef,
            super::content_video::Player,
            u8,
        )>,
    >,
    notification_sources:
        std::cell::RefCell<Vec<talk_core::preferences::notifications::NotificationSources>>,
    pub updates:
        Result<talk_adapters::updates::UpdateRuntime, talk_core::preferences::updates::UpdateError>,
    pub media: std::sync::Arc<talk_adapters::media::MediaService>,
    pub preferences: SettingsRuntime,
    pub accounts: Option<Rc<std::cell::RefCell<super::accounts::Accounts>>>,
    pub permissions: Result<
        crate::settings::permissions::PermissionRuntime,
        talk_core::preferences::permissions::PermissionError,
    >,
    pub devices: Result<DeviceRuntime, DeviceError>,
    pub notifications: Result<NotificationRuntime, NotificationError>,
}
impl Services {
    pub fn start(connect_accounts: bool) -> Result<Rc<Self>, PreferenceError> {
        let preferences = SettingsRuntime::start()?;
        let media = talk_adapters::media::MediaService::new();
        let accounts = connect_accounts.then(|| super::accounts::Accounts::start(media.clone()));
        let notifications = if let Some(accounts) = &accounts {
            NotificationRuntime::start_with_dispatcher(
                preferences.snapshot().active.notifications,
                Some(accounts.borrow().call_dispatcher()),
            )
        } else {
            NotificationRuntime::start(preferences.snapshot().active.notifications)
        };
        let devices = DeviceRuntime::start();
        if let Ok(devices) = &devices {
            devices.attach_media(media.clone());
        }
        Ok(Rc::new(Self {
            shutdown_deadline: std::cell::Cell::new(None),
            shutdown_unconfirmed_reported: std::cell::Cell::new(false),
            install: std::cell::RefCell::default(),
            notification_sequence: std::cell::Cell::new(0),
            voice_sequence: std::cell::Cell::new(0),
            voice_player: std::cell::RefCell::new(None),
            notification_sources: std::cell::RefCell::new(Vec::new()),
            updates: if connect_accounts {
                talk_adapters::updates::UpdateRuntime::start(env!("CARGO_PKG_VERSION"))
            } else {
                Err(talk_core::preferences::updates::UpdateError::UnsupportedInstall)
            },
            media: media.clone(),
            preferences,
            accounts,
            permissions: crate::settings::permissions::PermissionRuntime::start(),
            notifications,
            devices,
        }))
    }
    /// All live events and settings tests share the worker's single request sequence.
    pub fn submit_notification(
        &self,
        mut event: NotificationEvent,
    ) -> Result<RequestId, NotificationError> {
        let runtime = self.notifications.as_ref().map_err(|error| *error)?;
        let next = self
            .notification_sequence
            .get()
            .checked_add(1)
            .ok_or(NotificationError::Busy)?;
        event.request = RequestId(next);
        runtime.submit(event)?;
        self.notification_sequence.set(next);
        Ok(RequestId(next))
    }
    pub fn poll_notifications(
        &self,
        background: bool,
    ) -> Option<talk_core::preferences::notifications::NotificationActivation> {
        let (Some(accounts), Ok(runtime)) = (&self.accounts, &self.notifications) else {
            return None;
        };
        runtime.update_visibility(background);
        let mut accounts = accounts.borrow_mut();
        let observations = accounts.notification_observations();
        let mut previous = self.notification_sources.borrow_mut();
        // Retire removed sources before admitting replacements. This also frees a
        // bounded slot after logout; the first eight lifetime accounts are not permanent.
        previous.retain(|old| {
            if observations
                .iter()
                .any(|s| s.scope.account == old.scope.account)
            {
                true
            } else {
                !matches!(
                    runtime.retire_source(old.scope),
                    Ok(()) | Err(NotificationError::Stale)
                )
            }
        });
        for mut observation in observations.into_iter().take(8) {
            let old = previous
                .iter_mut()
                .find(|s| s.scope.account == observation.scope.account);
            let expected = old.as_ref().map(|s| s.scope);
            observation.revision = old.as_ref().map_or(0, |s| s.revision);
            if old.as_ref().is_some_and(|s| **s == observation) {
                continue;
            }
            let Some(revision) = observation.revision.checked_add(1) else {
                continue;
            };
            observation.revision = revision;
            if runtime.update_sources(expected, observation).is_ok() {
                if let Some(old) = old {
                    *old = observation;
                } else if previous.len() < 8 {
                    previous.push(observation);
                }
            }
        }
        let preferences = self.preferences.snapshot();
        let locale = match preferences.active.locale {
            talk_core::preferences::LocalePreference::System => preferences.observed_system_locale,
            selected => selected.resolve(&[]),
        };
        // Actions already dispatch through the current-call owner. Drain their
        // bounded receipts independently of whether the settings page is visible.
        for _ in 0..16 {
            if runtime.take_action_completion().is_none() {
                break;
            }
        }
        if let Some(mut event) = accounts.incoming_notification() {
            event.title = Text::new(crate::presentation::localization::text(
                locale,
                "Eingehender Anruf",
            ))
            .expect("bounded translated title");
            let kind = event.kind;
            // The single retained invitation expires with its current source.
            if self.submit_notification(event) != Err(NotificationError::Busy) {
                accounts.settle_incoming_notification(kind);
            }
        }
        if let Some(event) = accounts.message_notification(background, locale) {
            let kind = event.kind;
            // Busy retains this alert for at most five seconds; it is never a durable retry.
            if self.submit_notification(event) != Err(NotificationError::Busy) {
                accounts.settle_message_notification(kind);
            }
        }
        runtime.take_message_activation()
    }
    pub(super) fn shutdown_requested(&self) -> bool {
        self.shutdown_deadline.get().is_some()
    }
    pub fn request_shutdown(&self) {
        if self.shutdown_deadline.get().is_none() {
            self.shutdown_deadline.set(Some(
                std::time::Instant::now() + std::time::Duration::from_secs(5),
            ));
        }
        if let Ok(updates) = &self.updates {
            updates.request_shutdown();
        }
        self.request_application_shutdown();
    }
    pub(super) fn request_application_shutdown(&self) {
        self.preferences.request_shutdown();
        if let Some((_, player, _)) = self.voice_player.borrow().as_ref() {
            player.stop();
        }
        if let Some(accounts) = &self.accounts {
            accounts.borrow_mut().shutdown();
        }
        if let Ok(permissions) = &self.permissions {
            permissions.request_shutdown();
        }
        if let Ok(devices) = &self.devices {
            devices.request_shutdown();
        }
        if let Ok(notifications) = &self.notifications {
            notifications.request_shutdown();
        }
    }
    /// An explicit process quit has a finite wait. Expiry is unconfirmed
    /// shutdown, never a fabricated successful native/resource cleanup.
    pub fn waiting_for_shutdown(&self) -> bool {
        let stopped = self.stopped();
        let waiting = shutdown_pending(
            stopped,
            self.shutdown_deadline.get(),
            std::time::Instant::now(),
        );
        if !stopped && !waiting && !self.shutdown_unconfirmed_reported.replace(true) {
            crate::diagnostics::record(crate::diagnostics::Event::new(
                crate::diagnostics::Operation::Application,
                crate::diagnostics::Outcome::Uncertain,
            ));
        }
        waiting
    }
    pub fn stopped(&self) -> bool {
        self.updates
            .as_ref()
            .map_or(true, |updates| updates.is_stopped())
            && self.application_stopped()
    }
    pub(super) fn application_stopped(&self) -> bool {
        self.accounts
            .as_ref()
            .is_none_or(|accounts| accounts.borrow().stopped())
            && self.preferences.snapshot().runtime == talk_core::preferences::RuntimeState::Stopped
            && self.permissions.as_ref().map_or(true, |p| {
                p.snapshot().runtime == talk_core::preferences::RuntimeState::Stopped
            })
            && self.devices.as_ref().map_or(true, |d| {
                d.snapshot().runtime == talk_core::preferences::RuntimeState::Stopped
            })
            && self
                .notifications
                .as_ref()
                .map_or(true, |n| n.snapshot() == NotificationState::Stopped)
    }
}
pub struct Bridge {
    pub notification_worker_request: Option<RequestId>,
    pub update_last: Option<talk_core::preferences::updates::UpdateSnapshot>,
    pub services: Rc<Services>,
    pub cursor: u64,
    pub device_cursor: u64,
    pub fixture_voice: std::cell::RefCell<Option<talk_core::ui::content::VoiceSnapshot>>,
    pub fixture_voice_playback:
        std::cell::RefCell<Option<talk_core::ui::content::VoiceReviewPlayback>>,
    pub fixture_call_devices: bool,
    pub permission_cursor: u64,
    pub notification_request: Option<RequestId>,
    pub notification_result: NotificationState,
}
impl ComputerSettingsPort for Bridge {
    fn stop_voice_review_playback(&self) {
        *self.fixture_voice_playback.borrow_mut() = None;
        if let Some((_, player, _)) = self.services.voice_player.borrow().as_ref() {
            player.stop();
        }
    }
    fn voice_review_playback(&self) -> Option<talk_core::ui::content::VoiceReviewPlayback> {
        use super::content_video::Phase;
        use talk_core::ui::content::{PlaybackState, VoiceReviewPlayback};
        if self.fixture_voice.borrow().is_some() {
            return *self.fixture_voice_playback.borrow();
        }
        let slot = self.services.voice_player.borrow();
        let (context, player, speed) = slot.as_ref()?;
        if self.services.media.capture().is_some() {
            player.stop();
            return None;
        }
        if !self
            .services
            .devices
            .as_ref()
            .ok()
            .and_then(|d| d.voice_snapshot())
            .is_some_and(|v| {
                v.context == *context && v.phase == talk_core::ui::content::VoicePhase::Review
            })
        {
            player.stop();
            return None;
        }
        let snapshot = player.snapshot();
        Some(VoiceReviewPlayback {
            context: *context,
            speed_half_steps: *speed,
            state: match snapshot.phase {
                Phase::Starting => PlaybackState::Loading,
                Phase::Playing => PlaybackState::Playing,
                Phase::Paused => PlaybackState::Paused,
                Phase::Ended => PlaybackState::Ended,
                Phase::Stopping | Phase::Stopped => PlaybackState::Stopping,
                Phase::Failed(_) => {
                    PlaybackState::Failed(talk_core::ui::UnavailableReason::Unsupported)
                }
            },
            position_ms: snapshot.position_ms,
            duration_ms: snapshot.duration_ms,
        })
    }
    fn voice_review_play(
        &self,
        context: talk_core::ui::presentation::ConversationRef,
        play: bool,
    ) -> Result<(), DeviceError> {
        if let Some(voice) = *self.fixture_voice.borrow() {
            if voice.context != context || voice.phase != talk_core::ui::content::VoicePhase::Review
            {
                return Err(DeviceError::Stale);
            }
            let mut playback = self.fixture_voice_playback.borrow_mut();
            let current = playback.get_or_insert(talk_core::ui::content::VoiceReviewPlayback {
                context,
                state: talk_core::ui::content::PlaybackState::Paused,
                position_ms: 0,
                duration_ms: Some(voice.elapsed_ms),
                speed_half_steps: 2,
            });
            current.state = if play {
                talk_core::ui::content::PlaybackState::Playing
            } else {
                talk_core::ui::content::PlaybackState::Paused
            };
            return Ok(());
        }
        if self.services.media.capture().is_some() {
            return Err(DeviceError::Busy);
        }
        let devices = self.services.devices.as_ref().map_err(|e| *e)?;
        let bytes = devices.voice_audio(context).ok_or(DeviceError::Stale)?;
        let mut slot = self.services.voice_player.borrow_mut();
        if let Some((old, player, _)) = slot.as_mut() {
            if *old == context && !player.reap() {
                return player
                    .set_playing(play)
                    .map_err(|_| DeviceError::Unavailable);
            }
            player.stop();
            if !player.reap() {
                return Err(DeviceError::Busy);
            }
            *slot = None;
        }
        if !play {
            return Ok(());
        }
        if let Some(accounts) = &self.services.accounts {
            let mut accounts = accounts.borrow_mut();
            accounts.stop_video();
            accounts.stop_voice_player();
        }
        let bytes = talk_core::protocol::content::VideoBytes::from_shared(bytes.shared())
            .map_err(|_| DeviceError::CaptureFailed)?;
        let player = super::content_video::Player::start_audio(bytes)
            .map_err(|_| DeviceError::Unavailable)?;
        player
            .set_playing(true)
            .map_err(|_| DeviceError::Unavailable)?;
        *slot = Some((context, player, 2));
        Ok(())
    }
    fn voice_review_seek(
        &self,
        context: talk_core::ui::presentation::ConversationRef,
        position_ms: u64,
    ) -> Result<(), DeviceError> {
        if let Some(voice) = *self.fixture_voice.borrow() {
            if voice.context != context || voice.phase != talk_core::ui::content::VoicePhase::Review
            {
                return Err(DeviceError::Stale);
            }
            let mut playback = self.fixture_voice_playback.borrow_mut();
            let current = playback
                .as_mut()
                .filter(|p| p.context == context)
                .ok_or(DeviceError::Stale)?;
            current.position_ms = position_ms.min(current.duration_ms.unwrap_or(0));
            println!(
                "UI_VOICE_REVIEW_FIXTURE seek_ms={} no_backend=true",
                current.position_ms
            );
            return Ok(());
        }
        if self.services.media.capture().is_some() {
            return Err(DeviceError::Busy);
        }
        let devices = self.services.devices.as_ref().map_err(|e| *e)?;
        if !devices.voice_snapshot().is_some_and(|voice| {
            voice.context == context && voice.phase == talk_core::ui::content::VoicePhase::Review
        }) {
            return Err(DeviceError::Stale);
        }
        let slot = self.services.voice_player.borrow();
        let (_, player, _) = slot
            .as_ref()
            .filter(|(scope, _, _)| *scope == context)
            .ok_or(DeviceError::Stale)?;
        player
            .seek_ms(position_ms)
            .map_err(|_| DeviceError::Unavailable)
    }
    fn voice_review_speed(
        &self,
        context: talk_core::ui::presentation::ConversationRef,
        speed: u8,
    ) -> Result<(), DeviceError> {
        if self.fixture_voice.borrow().is_some() {
            let mut playback = self.fixture_voice_playback.borrow_mut();
            let current = playback
                .as_mut()
                .filter(|p| p.context == context)
                .ok_or(DeviceError::Stale)?;
            current.speed_half_steps = speed;
            return Ok(());
        }
        let mut slot = self.services.voice_player.borrow_mut();
        let (_, player, current) = slot
            .as_mut()
            .filter(|(scope, _, _)| *scope == context)
            .ok_or(DeviceError::Stale)?;
        player
            .set_rate(speed)
            .map_err(|_| DeviceError::Unavailable)?;
        *current = speed;
        Ok(())
    }
    fn voice_snapshot(&self) -> Option<talk_core::ui::content::VoiceSnapshot> {
        if self.fixture_voice.borrow().is_some() {
            return *self.fixture_voice.borrow();
        }
        self.services.devices.as_ref().ok()?.voice_snapshot()
    }
    fn start_voice(
        &self,
        context: talk_core::ui::presentation::ConversationRef,
    ) -> Result<(), DeviceError> {
        let accounts = self
            .services
            .accounts
            .as_ref()
            .ok_or(DeviceError::Unavailable)?;
        if !accounts.borrow().voice_allowed(context) || !self.device_preferences_applied() {
            return Err(DeviceError::Unavailable);
        }
        self.services
            .devices
            .as_ref()
            .map_err(|e| *e)?
            .start_voice(context)
    }
    fn stop_voice(
        &self,
        context: talk_core::ui::presentation::ConversationRef,
        cancel: bool,
    ) -> Result<(), DeviceError> {
        {
            let mut fixture = self.fixture_voice.borrow_mut();
            if let Some(voice) = fixture.as_mut() {
                if voice.context != context {
                    return Err(DeviceError::Stale);
                }
                if cancel {
                    *fixture = None;
                    *self.fixture_voice_playback.borrow_mut() = None;
                } else {
                    voice.phase = talk_core::ui::content::VoicePhase::Review;
                }
                return Ok(());
            }
        }
        if cancel
            && let Some((_, player, _)) = self
                .services
                .voice_player
                .borrow()
                .as_ref()
                .filter(|(scope, _, _)| *scope == context)
        {
            player.stop();
        }
        self.services
            .devices
            .as_ref()
            .map_err(|e| *e)?
            .stop_voice(context, cancel)
    }
    fn send_voice(
        &self,
        context: talk_core::ui::presentation::ConversationRef,
    ) -> Result<(), DeviceError> {
        let fixture_voice = *self.fixture_voice.borrow();
        if let Some(voice) = fixture_voice {
            if voice.context != context || voice.phase != talk_core::ui::content::VoicePhase::Review
            {
                return Err(DeviceError::Stale);
            }
            *self.fixture_voice.borrow_mut() = None;
            *self.fixture_voice_playback.borrow_mut() = None;
            println!("UI_VOICE_FIXTURE send no_backend=true");
            return Ok(());
        }
        let accounts = self
            .services
            .accounts
            .as_ref()
            .ok_or(DeviceError::Unavailable)?;
        if !accounts.borrow().voice_allowed(context) {
            return Err(DeviceError::Unavailable);
        }
        let devices = self.services.devices.as_ref().map_err(|e| *e)?;
        let bytes = devices.voice_audio(context).ok_or(DeviceError::Stale)?;
        let sequence = self
            .services
            .voice_sequence
            .get()
            .checked_add(1)
            .filter(|n| *n < (1u64 << 63))
            .ok_or(DeviceError::Unavailable)?;
        let request = talk_core::ui::presentation::RequestId((1u64 << 63) | sequence);
        accounts
            .borrow_mut()
            .send_voice_bytes(request, context, bytes)
            .map_err(|_| DeviceError::Unavailable)?;
        if let Some((_, player, _)) = self
            .services
            .voice_player
            .borrow()
            .as_ref()
            .filter(|(scope, _, _)| *scope == context)
        {
            player.stop();
        }
        self.services.voice_sequence.set(sequence);
        // The upload owner has already accepted this intent. A concurrent
        // device teardown may have retired the local review; it must not turn
        // an accepted send into a retry that could duplicate the message.
        let _ = devices.stop_voice(context, true);
        Ok(())
    }
    fn resolve_deadline(
        &self,
        choice: talk_core::preferences::time::DeadlineChoice,
    ) -> Result<
        talk_core::preferences::time::DeadlineMoment,
        talk_core::preferences::time::LocalTimeError,
    > {
        talk_adapters::system_time::deadline(choice)
    }
    fn search_date_boundary(
        &self,
        local: talk_core::ui::Text<32>,
        end: bool,
    ) -> Result<talk_core::ui::Text<32>, talk_core::preferences::time::LocalTimeError> {
        talk_adapters::system_time::search_date_boundary(local, end)
    }
    fn local_datetime(&self, timestamp: u64) -> Option<talk_core::ui::Text<32>> {
        if self.services.accounts.is_none() {
            // Disconnected fictional scenarios have no identity worker to
            // observe a system zone; use fixed authored dates for those fixtures.
            let label = match timestamp {
                1_800_003_600 => Some("2027-01-15 09:00"),
                1_800_090_000 => Some("2027-01-16 09:00"),
                _ => None,
            };
            if let Some(label) = label {
                return talk_core::ui::Text::new(label).ok();
            }
        }
        talk_adapters::system_time::local_datetime(timestamp)
    }
    fn same_local_day(&self, first: u64, second: u64) -> bool {
        talk_adapters::system_time::same_local_day(first, second)
    }
    fn reduced_motion(&self) -> bool {
        self.services.preferences.reduced_motion()
    }
    fn emoji_picker_available(&self) -> bool {
        self.services.preferences.emoji_picker_available()
    }
    fn show_emoji_picker(&self) -> Result<(), crate::presentation::preferences::EmojiPickerError> {
        if !self.services.preferences.emoji_picker_available() {
            return Err(crate::presentation::preferences::EmojiPickerError::Unavailable);
        }
        self.services
            .preferences
            .show_emoji_picker()
            .map_err(|_| crate::presentation::preferences::EmojiPickerError::Failed)
    }
    fn cancel_emoji_picker(&self) {
        self.services.preferences.cancel_emoji_picker();
    }
    fn take_emoji_picker_error(
        &mut self,
    ) -> Option<crate::presentation::preferences::EmojiPickerError> {
        self.services
            .preferences
            .take_emoji_picker_error()
            .map(|_| crate::presentation::preferences::EmojiPickerError::Failed)
    }
    fn update_snapshot(&self) -> talk_core::preferences::updates::UpdateSnapshot {
        let mut snapshot = self
            .services
            .updates
            .as_ref()
            .map(|u| u.snapshot())
            .unwrap_or_else(|error| talk_core::preferences::updates::UpdateSnapshot {
                revision: 0,
                generation: 0,
                current_version: Text::new(env!("CARGO_PKG_VERSION")).expect("package version"),
                channel: self.services.preferences.snapshot().active.update_channel,
                next_check_unix_seconds: None,
                last_check: None,
                state: talk_core::preferences::updates::UpdateState::Failed(*error),
            });
        self.services.install.borrow().project(&mut snapshot);
        snapshot
    }
    fn take_update_snapshot(&mut self) -> Option<talk_core::preferences::updates::UpdateSnapshot> {
        let snapshot = self.update_snapshot();
        if self.update_last.as_ref() == Some(&snapshot) {
            return None;
        }
        self.update_last = Some(snapshot.clone());
        Some(snapshot)
    }
    fn check_updates(&self) -> Result<(), talk_core::preferences::updates::UpdateError> {
        self.services.updates.as_ref().map_err(|e| *e)?.check(
            self.services
                .preferences
                .snapshot()
                .confirmed
                .update_channel,
        )
    }
    fn install_update(
        &self,
        expected_generation: u64,
    ) -> Result<(), talk_core::preferences::updates::UpdateError> {
        self.services
            .install
            .borrow_mut()
            .begin(&self.services, expected_generation)
    }
    fn snapshot(&self) -> PreferenceSnapshot {
        self.services.preferences.snapshot()
    }
    fn take_snapshot(&mut self) -> Option<PreferenceSnapshot> {
        let (cursor, snapshot) = self.services.preferences.poll(self.cursor)?;
        self.cursor = cursor;
        if let Ok(notifications) = &self.services.notifications {
            notifications.update_policy(snapshot.active.notifications);
            notifications.update_locale(match snapshot.active.locale {
                talk_core::preferences::LocalePreference::System => snapshot.observed_system_locale,
                selected => selected.resolve(&[]),
            });
        }
        Some(snapshot)
    }
    fn submit(&self, command: PreferenceCommand) -> Result<(), PreferenceError> {
        self.services.preferences.submit(command).map(|_| ())
    }
    fn permission_snapshot(
        &self,
    ) -> Result<
        talk_core::preferences::permissions::PermissionSnapshot,
        talk_core::preferences::permissions::PermissionError,
    > {
        self.services
            .permissions
            .as_ref()
            .map(|p| p.snapshot())
            .map_err(|e| *e)
    }
    fn take_permission_snapshot(
        &mut self,
    ) -> Option<
        Result<
            talk_core::preferences::permissions::PermissionSnapshot,
            talk_core::preferences::permissions::PermissionError,
        >,
    > {
        match &self.services.permissions {
            Ok(runtime) => {
                let (cursor, snapshot) = runtime.poll(self.permission_cursor)?;
                self.permission_cursor = cursor;
                Some(Ok(snapshot))
            }
            Err(error) if self.permission_cursor == 0 => {
                self.permission_cursor = 1;
                Some(Err(*error))
            }
            Err(_) => None,
        }
    }
    fn submit_permission(
        &self,
        intent: talk_core::preferences::permissions::PermissionIntent,
    ) -> Result<(), talk_core::preferences::permissions::PermissionError> {
        self.services
            .permissions
            .as_ref()
            .map_err(|e| *e)?
            .submit(intent)
    }
    fn device_preferences_applied(&self) -> bool {
        let Ok(devices) = &self.services.devices else {
            return false;
        };
        let plan = devices.selected_plan();
        let preference = self.services.preferences.snapshot();
        plan.revision == preference.revision
            && plan.saved == preference.active.devices
            && ![plan.microphone, plan.speaker, plan.camera]
                .iter()
                .any(|s| matches!(s, talk_core::preferences::devices::SelectionState::Unknown))
    }
    fn device_snapshot(&self) -> Result<DeviceSnapshot, DeviceError> {
        if self.fixture_call_devices {
            return Ok(super::fixtures::call_devices());
        }
        self.services
            .devices
            .as_ref()
            .map(|d| d.snapshot())
            .map_err(|e| *e)
    }
    fn take_device_snapshot(&mut self) -> Option<Result<DeviceSnapshot, DeviceError>> {
        if self.fixture_call_devices {
            return None;
        }
        match &self.services.devices {
            Ok(devices) => {
                let (cursor, snapshot) = devices.poll(self.device_cursor)?;
                self.device_cursor = cursor;
                Some(Ok(snapshot))
            }
            Err(error) if self.device_cursor == 0 => {
                self.device_cursor = 1;
                Some(Err(*error))
            }
            Err(_) => None,
        }
    }
    fn submit_device(&self, intent: DeviceIntent) -> Result<(), DeviceError> {
        self.services
            .devices
            .as_ref()
            .map_err(|e| *e)?
            .submit(intent)
    }
    fn account_snapshot(
        &self,
        scope: talk_core::ui::AccountScope,
    ) -> talk_core::preferences::account::AccountPreferenceSnapshot {
        self.services.accounts.as_ref().map_or(
            talk_core::preferences::account::AccountPreferenceSnapshot {
                scope,
                values: None,
                state: talk_core::preferences::account::AccountPreferenceState::Unavailable,
            },
            |accounts| accounts.borrow().account_snapshot(scope),
        )
    }
    fn submit_account(
        &self,
        command: talk_core::preferences::account::AccountPreferenceCommand,
    ) -> Result<(), talk_core::preferences::account::AccountPreferenceError> {
        self.services
            .accounts
            .as_ref()
            .ok_or(talk_core::preferences::account::AccountPreferenceError::Unavailable)?
            .borrow_mut()
            .submit_account(command)
    }
    fn system_locale(&self) -> Locale {
        self.services.preferences.snapshot().observed_system_locale
    }
    fn notification_state(&mut self) -> NotificationState {
        if let Ok(runtime) = &self.services.notifications {
            // Drain the worker's bounded completion queue on the existing 100 ms
            // composition poll. An obsolete Test never overwrites the latest one.
            for _ in 0..16 {
                let Some(done) = runtime.take_completion() else {
                    break;
                };
                if Some(done.request) == self.notification_worker_request {
                    self.notification_result = NotificationState::Finished {
                        request: self.notification_request.expect("correlated test request"),
                        banner: done.banner,
                        sound: done.sound,
                    };
                }
            }
            if let NotificationState::Pending(request) = runtime.snapshot()
                && Some(request) == self.notification_worker_request
            {
                self.notification_result = NotificationState::Pending(
                    self.notification_request.expect("correlated test request"),
                );
            }
        }
        self.notification_result
    }
    fn notification_capabilities(
        &self,
    ) -> talk_core::preferences::notifications::NativeNotificationCapabilities {
        use talk_core::preferences::notifications::{
            BannerBackend, NativeNotificationCapabilities, SoundBackend,
        };
        self.services.notifications.as_ref().map_or(
            NativeNotificationCapabilities {
                banners: BannerBackend::Unavailable,
                sounds: SoundBackend::Unavailable,
                native_call_ui: false,
                native_call_actions: false,
            },
            |runtime| runtime.native_capabilities(),
        )
    }
    fn notification_access(&self) -> NotificationAccessSnapshot {
        self.services
            .notifications
            .as_ref()
            .map_or(NotificationAccessSnapshot::unavailable(), |runtime| {
                runtime.access_snapshot()
            })
    }
    fn submit_notification_access(
        &self,
        request: RequestId,
        action: NotificationAccessAction,
    ) -> Result<(), NotificationError> {
        self.services
            .notifications
            .as_ref()
            .map_err(|error| *error)?
            .submit_access(request, action)
    }
    fn notification_source(
        &self,
        scope: Option<talk_core::ui::AccountScope>,
        category: NotificationCategory,
    ) -> talk_core::preferences::notifications::SourceAvailability {
        // Query only: this composition cannot register an authenticated source.
        self.services.notifications.as_ref().map_or(
            talk_core::preferences::notifications::SourceAvailability::Unavailable,
            |runtime| runtime.source_availability(scope, category),
        )
    }
    fn test_notification(
        &mut self,
        request: RequestId,
        category: NotificationCategory,
        title: &str,
        body: &str,
    ) -> Result<(), NotificationError> {
        let worker_request = self.services.submit_notification(NotificationEvent::test(
            request,
            category,
            Text::new(title).map_err(|_| NotificationError::InvalidContent)?,
            Text::new(body).map_err(|_| NotificationError::InvalidContent)?,
        ))?;
        self.notification_worker_request = Some(worker_request);
        self.notification_request = Some(request);
        self.notification_result = NotificationState::Pending(request);
        Ok(())
    }
}

/// One validated raw frame, never an image URI/file or serialized UI projection.
/// Conversion is called on GPUI's background executor; the previous RenderImage
/// is retired by the owning view when this newest image is installed or stopped.
pub fn camera_image(
    frame: &crate::devices::VideoFrame,
) -> Option<std::sync::Arc<gpui::RenderImage>> {
    let (width, height) = (frame.width(), frame.height());
    if width == 0 || height == 0 || width > 640 || height > 480 {
        return None;
    }
    let raw = frame.rgba();
    let expected = (width as usize)
        .checked_mul(height as usize)?
        .checked_mul(4)?;
    if raw.len() != expected {
        return None;
    }
    let mut bytes = raw.to_vec();
    for pixel in bytes.as_chunks_mut::<4>().0 {
        let alpha = u16::from(pixel[3]);
        let r = (u16::from(pixel[0]) * alpha / 255) as u8;
        let g = (u16::from(pixel[1]) * alpha / 255) as u8;
        let b = (u16::from(pixel[2]) * alpha / 255) as u8;
        pixel[0] = b;
        pixel[1] = g;
        pixel[2] = r;
    }
    let rgba = image::RgbaImage::from_raw(width, height, bytes)?;
    Some(std::sync::Arc::new(gpui::RenderImage::new([
        image::Frame::new(rgba),
    ])))
}

/// Owned decoded video; called on the background executor and bounded by the
/// admitted native source. No raw frame crosses the UI protocol DTO.
pub fn call_image(
    mut frame: talk_adapters::media::VideoImage,
) -> Option<std::sync::Arc<gpui::RenderImage>> {
    if frame.width == 0
        || frame.height == 0
        || frame.width > 1920
        || frame.height > 1080
        || frame.rgba.len() != (frame.width * frame.height * 4) as usize
    {
        return None;
    }
    for pixel in frame.rgba.as_chunks_mut::<4>().0 {
        pixel.swap(0, 2);
    }
    let image = image::RgbaImage::from_raw(frame.width, frame.height, frame.rgba)?;
    Some(std::sync::Arc::new(gpui::RenderImage::new([
        image::Frame::new(image),
    ])))
}

fn shutdown_pending(
    stopped: bool,
    deadline: Option<std::time::Instant>,
    now: std::time::Instant,
) -> bool {
    !stopped && deadline.is_none_or(|deadline| now < deadline)
}
#[cfg(test)]
mod shutdown_tests {
    #[test]
    fn explicit_quit_deadline_does_not_claim_resource_shutdown() {
        let now = std::time::Instant::now();
        let deadline = now + std::time::Duration::from_secs(5);
        assert!(super::shutdown_pending(false, Some(deadline), now));
        assert!(!super::shutdown_pending(false, Some(deadline), deadline));
        assert!(!super::shutdown_pending(true, Some(deadline), now));
        assert!(super::shutdown_pending(false, None, deadline));
    }
}
