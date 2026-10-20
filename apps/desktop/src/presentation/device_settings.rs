//! Explicit local device checks. Selection is durable; capture is a separate intent.
use super::controls::Availability;
use super::preferences::SettingsTab;
use super::{ButtonStyle, Desktop, LocalAction, Overlay};
use gpui::{Context, Div, FontWeight, ImageSource, div, img, prelude::*, px};
use talk_core::preferences::{
    RequestId,
    devices::{
        DeviceError, DeviceIntent, DeviceKind, DeviceSelection, PreviewSession, PreviewState,
        SelectionState, VideoSurfaceId,
    },
};

impl Desktop {
    fn next_device_request(&mut self) -> Option<RequestId> {
        self.device_request = self.device_request.checked_add(1)?;
        Some(RequestId(self.device_request))
    }
    pub(super) fn apply_device_preferences(&mut self) {
        let value = (
            self.preferences.revision,
            self.preferences.active.devices.clone(),
        );
        if self.applied_devices.as_ref() == Some(&value) {
            return;
        }
        if let Some(request) = self.next_device_request() {
            match self
                .settings_port
                .submit_device(DeviceIntent::ApplySelection {
                    request,
                    revision: value.0,
                    values: value.1.clone(),
                }) {
                Ok(()) => {
                    self.applied_devices = Some(value);
                    self.device_error = None;
                }
                Err(error) => self.device_error = Some(error),
            }
        }
    }
    pub(super) fn enumerate_devices(&mut self) {
        if let Some(request) = self.next_device_request() {
            self.device_error = self
                .settings_port
                .submit_device(DeviceIntent::Enumerate { request })
                .err();
        }
    }
    pub(super) fn refresh_device_kind(&mut self, kind: DeviceKind) {
        if let Some(request) = self.next_device_request() {
            self.device_error = self
                .settings_port
                .submit_device(DeviceIntent::Refresh { request, kind })
                .err();
        }
        if kind == DeviceKind::Microphone {
            self.microphone_auto_attempted = false;
        }
    }
    pub(super) fn refresh_devices(&mut self, cx: &mut Context<Self>) {
        if let Some(snapshot) = self.settings_port.take_device_snapshot() {
            self.devices = snapshot;
            if self.device_stopping.is_some()
                && self.devices.as_ref().is_ok_and(|s| {
                    s.session.is_none()
                        && s.microphone_preview == PreviewState::Stopped
                        && s.camera_preview == PreviewState::Stopped
                        && s.speaker_test == PreviewState::Stopped
                })
            {
                self.device_stopping = None;
            }
            if self.preview_surface().is_none() {
                self.set_camera_image(None, cx);
            }
            cx.notify();
        }
        if self.devices_open()
            && self.permission_ui.microphone_ready
            && !self.microphone_auto_attempted
            && self.device_stopping.is_none()
            && self.settings_port.device_preferences_applied()
            && self.devices.as_ref().is_ok_and(|s| {
                matches!(
                    s.microphone_preview,
                    PreviewState::Stopped | PreviewState::Failed { .. }
                )
            })
        {
            self.microphone_auto_attempted = true;
            self.start_device_preview(DeviceKind::Microphone);
        }
    }
    pub(super) fn start_device_preview(&mut self, kind: DeviceKind) {
        if !self.devices_open() || self.device_stopping.is_some() {
            return;
        }
        let permission = match kind {
            DeviceKind::Microphone => {
                Some(talk_core::preferences::permissions::PermissionKind::Microphone)
            }
            DeviceKind::Camera => Some(talk_core::preferences::permissions::PermissionKind::Camera),
            DeviceKind::Speaker => None,
        };
        if permission.is_some_and(|k| !self.observation(k).allows_device_preview()) {
            return;
        }
        let session = match self.device_session {
            Some(session) => session,
            None => {
                let Some(next) = self.device_session_counter.checked_add(1) else {
                    return;
                };
                self.device_session_counter = next;
                PreviewSession(next)
            }
        };
        let Some(request) = self.next_device_request() else {
            return;
        };
        let intent = if kind == DeviceKind::Speaker {
            DeviceIntent::TestSpeaker { request, session }
        } else {
            DeviceIntent::StartPreview {
                request,
                session,
                kind,
            }
        };
        match self.settings_port.submit_device(intent) {
            Ok(()) => {
                self.device_session = Some(session);
                self.device_error = None;
            }
            Err(error) => self.device_error = Some(error),
        }
    }
    pub(super) fn stop_one_device(&mut self, kind: DeviceKind, cx: &mut Context<Self>) {
        if let Some(session) = self.device_session {
            self.device_error = self
                .settings_port
                .submit_device(DeviceIntent::StopDevice { session, kind })
                .err();
        }
        if kind == DeviceKind::Camera {
            self.set_camera_image(None, cx);
        }
    }
    pub(super) fn stop_device_preview(&mut self, cx: &mut Context<Self>) {
        if let Some(session) = self.device_session {
            match self
                .settings_port
                .submit_device(DeviceIntent::StopPreview { session })
            {
                Ok(()) => {
                    self.device_stopping = Some(session);
                    self.device_session = None;
                    self.device_error = None;
                }
                Err(error) => self.device_error = Some(error),
            }
        }
        self.set_camera_image(None, cx);
    }
    pub fn preview_surface(&self) -> Option<VideoSurfaceId> {
        if self.overlay != Some(Overlay::Settings)
            || self.settings_tab != SettingsTab::Devices
            || self.incoming_visible()
        {
            return None;
        }
        let snapshot = self.devices.as_ref().ok()?;
        if snapshot.session != self.device_session {
            return None;
        }
        match snapshot.camera_preview {
            PreviewState::Live { surface, .. } => surface,
            _ => None,
        }
    }
    pub fn set_camera_image(&mut self, image: Option<ImageSource>, cx: &mut Context<Self>) {
        let had_image = self.camera_image.is_some();
        if let Some(ImageSource::Render(old)) = self.camera_image.take() {
            cx.drop_image(old, None);
        }
        self.camera_image = image;
        if had_image || self.camera_image.is_some() {
            cx.notify();
        }
    }
    fn level_meter(&self, level: Option<u16>) -> Div {
        let lit = level.map_or(0, |v| usize::from(v.min(1000)).div_ceil(50));
        div().flex().gap_1().h(px(18.)).children((0..20).map(|i| {
            div().flex_1().h_full().rounded(px(2.)).bg(if i < lit {
                if i > 16 {
                    self.theme.warning
                } else {
                    self.theme.success
                }
            } else {
                self.theme.subtle
            })
        }))
    }
    pub(super) fn device_page(&self, cx: &Context<Self>) -> Div {
        let mut page = div()
            .flex()
            .flex_col()
            .gap_4()
            .child(self.title(self.l("Geräte")))
            .child(self.hint(self.l("Der Mikrofonpegel wird hier nach Freigabe automatisch gemessen. Kamera und Klingelton starten auf deinen Wunsch.")));
        let snapshot = match &self.devices {
            Ok(snapshot) => snapshot,
            Err(error) => return page.child(self.hint(self.l(device_error_key(*error)))),
        };
        for kind in [
            DeviceKind::Microphone,
            DeviceKind::Speaker,
            DeviceKind::Camera,
        ] {
            let (title, chosen, state, preview) = match kind {
                DeviceKind::Microphone => (
                    "Mikrofon",
                    &self.preferences.active.devices.microphone,
                    &snapshot.microphone,
                    snapshot.microphone_preview,
                ),
                DeviceKind::Speaker => (
                    "Lautsprecher",
                    &self.preferences.active.devices.speaker,
                    &snapshot.speaker,
                    snapshot.speaker_test,
                ),
                DeviceKind::Camera => (
                    "Kamera",
                    &self.preferences.active.devices.camera,
                    &snapshot.camera,
                    snapshot.camera_preview,
                ),
            };
            let mut section = div()
                .flex_none()
                .flex()
                .flex_col()
                .gap_2()
                .pb_4()
                .border_b_1()
                .border_color(self.theme.border)
                .child(div().font_weight(FontWeight::SEMIBOLD).child(self.l(title)));
            let selected_label = match chosen {
                DeviceSelection::SystemDefault => self.l("Systemstandard"),
                DeviceSelection::Specific(id) => snapshot
                    .devices
                    .as_slice()
                    .iter()
                    .find(|d| d.id == *id)
                    .map_or(self.l("Ausgewähltes Gerät"), |d| d.label.as_str()),
            };
            section = section.child(
                div()
                    .flex()
                    .items_center()
                    .gap_2()
                    .child(
                        self.button(
                            ("device-picker", kind as usize),
                            "",
                            ButtonStyle::Secondary,
                            true,
                            LocalAction::ToggleDevicePicker(kind),
                            cx,
                        )
                        .flex_1()
                        .min_w_0()
                        .justify_between()
                        .child(
                            div()
                                .flex_1()
                                .min_w_0()
                                .text_ellipsis()
                                .child(selected_label.to_owned()),
                        )
                        .child(self.scaled_icon(
                            "chevron-down",
                            16.,
                            self.theme.muted,
                        )),
                    )
                    .child(self.icon_button(
                        ("refresh-device", kind as usize),
                        ("refresh", "Geräte aktualisieren"),
                        ButtonStyle::Quiet,
                        Availability::available(),
                        LocalAction::RefreshDevice(kind),
                        cx,
                    )),
            );

            if self.device_picker == Some(kind) {
                let mut choices = div()
                    .flex_none()
                    .flex()
                    .flex_col()
                    .gap_2()
                    .p_2()
                    .rounded_lg()
                    .bg(self.theme.subtle)
                    .child(self.button(
                        ("device-default", kind as usize),
                        self.l("Systemstandard"),
                        if *chosen == DeviceSelection::SystemDefault {
                            ButtonStyle::Primary
                        } else {
                            ButtonStyle::Secondary
                        },
                        true,
                        LocalAction::SelectDevice(kind, DeviceSelection::SystemDefault),
                        cx,
                    ));
                for device in snapshot
                    .devices
                    .as_slice()
                    .iter()
                    .filter(|d| d.kind == kind)
                {
                    let selection = DeviceSelection::Specific(device.id.clone());
                    // Provider labels are data, never localization catalog keys.
                    choices = choices.child(
                        self.control(
                            gpui::SharedString::from(format!(
                                "device-{:?}-{}",
                                kind,
                                device.id.as_str()
                            )),
                            if *chosen == selection {
                                ButtonStyle::Primary
                            } else {
                                ButtonStyle::Secondary
                            },
                            true,
                            LocalAction::SelectDevice(kind, selection),
                            cx,
                        )
                        .justify_start()
                        .child(
                            div()
                                .min_w_0()
                                .text_ellipsis()
                                .child(device.label.as_str().to_owned()),
                        ),
                    );
                }
                section = section.child(choices);
            }
            let effective=match state {
                SelectionState::Selected(id)=>snapshot.devices.as_slice().iter().find(|d|d.id==*id).map(|d|d.label.as_str()).unwrap_or(self.l("Ausgewähltes Gerät")),
                SelectionState::Unknown=>self.l("Geräte werden ermittelt …"),
                SelectionState::DefaultFallback{effective:Some(_),..}=>self.l("Systemstandard wird verwendet, weil das gespeicherte Gerät fehlt."),
                SelectionState::DefaultFallback{effective:None,..}=>self.l("Gespeichertes Gerät fehlt. Bitte Systemstandard oder ein anderes Gerät wählen."),
                SelectionState::Unavailable(error)=>self.l(device_error_key(*error)),
            };
            section = section.child(self.hint(effective));
            let active = matches!(
                preview,
                PreviewState::Starting { .. } | PreviewState::Live { .. }
            );
            let available = matches!(
                state,
                SelectionState::Selected(_)
                    | SelectionState::DefaultFallback {
                        effective: Some(_),
                        ..
                    }
            ) && !matches!(preview, PreviewState::Stopping { .. });
            if kind != DeviceKind::Speaker {
                section = section.child(self.permission_panel(
                    if kind == DeviceKind::Microphone {
                        talk_core::preferences::permissions::PermissionKind::Microphone
                    } else {
                        talk_core::preferences::permissions::PermissionKind::Camera
                    },
                    cx,
                ));
            }
            if kind != DeviceKind::Microphone {
                let camera_allowed = kind != DeviceKind::Camera
                    || self
                        .observation(talk_core::preferences::permissions::PermissionKind::Camera)
                        .allows_device_preview();
                section = section.child(self.button(
                    ("device-check", kind as usize),
                    if kind == DeviceKind::Speaker {
                        "Klingelton abspielen"
                    } else if active {
                        "Kameravorschau beenden"
                    } else {
                        "Kameravorschau starten"
                    },
                    ButtonStyle::Secondary,
                    Availability::when(
                        self.device_stopping.is_none()
                            && (if kind == DeviceKind::Camera && active {
                                true
                            } else {
                                available && !active && camera_allowed
                            }),
                        if active && kind == DeviceKind::Speaker {
                            "Klingelton wird einmal abgespielt …"
                        } else if active {
                            "Test aktiv"
                        } else if self.device_stopping.is_some()
                            || matches!(preview, PreviewState::Stopping { .. })
                        {
                            "Gerät wird beendet …"
                        } else {
                            "Für diesen Test ist kein verfügbares Gerät mit Freigabe ausgewählt."
                        },
                    ),
                    if kind == DeviceKind::Camera && active {
                        LocalAction::StopDevice(kind)
                    } else {
                        LocalAction::StartDevice(kind)
                    },
                    cx,
                ));
            }
            match preview {
                PreviewState::Starting { .. } => {
                    section = section.child(self.hint(self.l("Gerät wird gestartet …")))
                }
                PreviewState::Stopping { .. } => {
                    section = section.child(self.hint(self.l("Gerät wird beendet …")))
                }
                PreviewState::Failed { error, .. } => {
                    section = section.child(self.hint(self.l(device_error_key(error))))
                }
                PreviewState::Live { .. } => {
                    section = section.child(self.hint(self.l(if kind == DeviceKind::Speaker {
                        "Klingelton wird einmal abgespielt …"
                    } else {
                        "Test aktiv"
                    })))
                }
                PreviewState::Stopped => {}
            }
            if kind != DeviceKind::Camera {
                let level = if kind == DeviceKind::Microphone {
                    snapshot.microphone_level
                } else {
                    snapshot.speaker_level
                };
                section = section.child(self.level_meter(level));
                if kind == DeviceKind::Microphone && level.is_none() {
                    section =
                        section.child(self.hint(self.l("Noch kein Eingangssignal gemessen.")));
                }
            }
            if kind == DeviceKind::Camera {
                if *chosen == DeviceSelection::SystemDefault {
                    section = section.child(
                        self.hint(self.l("Bei Kameras wird das erste verfügbare Gerät verwendet.")),
                    );
                }
                section = section.child(
                    div()
                        .w_full()
                        .h(px(180.))
                        .rounded_lg()
                        .overflow_hidden()
                        .bg(self.theme.stage)
                        .when_some(self.camera_image.clone(), |d, image| {
                            d.child(img(image).size_full().object_fit(gpui::ObjectFit::Contain))
                        })
                        .when(self.camera_image.is_none(), |d| {
                            d.flex()
                                .items_center()
                                .justify_center()
                                .child(self.hint(self.l("Keine Kameravorschau aktiv.")))
                        }),
                );
            }
            page = page.child(section);
        }
        if let Some(error) = self.device_error {
            page = page.child(self.hint(self.l(device_error_key(error))));
        }
        page
    }
}
pub(super) fn device_error_key(error: DeviceError) -> &'static str {
    match error {
        DeviceError::Unavailable => "Kein Gerät verfügbar.",
        DeviceError::PermissionDenied => {
            "Zugriff verweigert. Erlaube Mikrofon oder Kamera in den Systemeinstellungen."
        }
        DeviceError::Removed => "Das ausgewählte Gerät wurde entfernt.",
        DeviceError::Busy => "Bitte warte, bis der vorherige Gerätetest beendet ist.",
        DeviceError::Stale => "Die Geräteauswahl hat sich geändert. Bitte erneut versuchen.",
        DeviceError::InvalidSelection => "Bitte wähle ein verfügbares Gerät.",
        DeviceError::CaptureFailed => "Der Gerätetest ist fehlgeschlagen.",
    }
}
