//! Worker lifetime and the memory-only presentation adapter.
use crate::devices::DeviceRuntime;
use crate::presentation::preferences::ComputerSettingsPort;
use crate::settings::{SettingsRuntime, notifications::NotificationRuntime};
use std::rc::Rc;
use talk_core::preferences::devices::{DeviceError, DeviceIntent, DeviceSnapshot};
use talk_core::preferences::notifications::{
    NotificationCategory, NotificationError, NotificationEvent, NotificationState,
};
use talk_core::preferences::{
    Locale, PreferenceCommand, PreferenceError, PreferenceSnapshot, RequestId,
};
use talk_core::ui::Text;

pub struct Services {
    notification_sequence: std::cell::Cell<u64>,
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
            notification_sequence: std::cell::Cell::new(0),
            notification_sources: std::cell::RefCell::new(Vec::new()),
            updates: talk_adapters::updates::UpdateRuntime::start(env!("CARGO_PKG_VERSION")),
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
    pub fn poll_notifications(&self, background: bool) {
        let (Some(accounts), Ok(runtime)) = (&self.accounts, &self.notifications) else {
            return;
        };
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
    }
    pub fn request_shutdown(&self) {
        if let Ok(updates) = &self.updates {
            updates.request_shutdown();
        }
        self.preferences.request_shutdown();
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
    pub fn stopped(&self) -> bool {
        self.updates
            .as_ref()
            .map_or(true, |updates| updates.is_stopped())
            && self
                .accounts
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
    pub update_cursor: u64,
    pub services: Rc<Services>,
    pub cursor: u64,
    pub device_cursor: u64,
    pub permission_cursor: u64,
    pub notification_request: Option<RequestId>,
    pub notification_result: NotificationState,
}
impl ComputerSettingsPort for Bridge {
    fn update_snapshot(&self) -> talk_core::preferences::updates::UpdateSnapshot {
        self.services
            .updates
            .as_ref()
            .map(|u| u.snapshot())
            .unwrap_or_else(|error| talk_core::preferences::updates::UpdateSnapshot {
                revision: 0,
                current_version: Text::new(env!("CARGO_PKG_VERSION")).expect("package version"),
                channel: self.services.preferences.snapshot().active.update_channel,
                state: talk_core::preferences::updates::UpdateState::Failed(*error),
            })
    }
    fn take_update_snapshot(&mut self) -> Option<talk_core::preferences::updates::UpdateSnapshot> {
        let (cursor, snapshot) = self
            .services
            .updates
            .as_ref()
            .ok()?
            .poll(self.update_cursor)?;
        self.update_cursor = cursor;
        Some(snapshot)
    }
    fn check_updates(&self) -> Result<(), talk_core::preferences::updates::UpdateError> {
        self.services
            .updates
            .as_ref()
            .map_err(|e| *e)?
            .check(self.services.preferences.snapshot().active.update_channel)
    }
    fn open_release(&self) -> Result<(), talk_core::preferences::updates::UpdateError> {
        self.services
            .updates
            .as_ref()
            .map_err(|e| *e)?
            .open_release()
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
        self.services
            .devices
            .as_ref()
            .map(|d| d.snapshot())
            .map_err(|e| *e)
    }
    fn take_device_snapshot(&mut self) -> Option<Result<DeviceSnapshot, DeviceError>> {
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
