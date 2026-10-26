//! Explicit local device checks. Selection is durable; capture is a separate intent.
use super::controls::Availability;
use super::preferences::SettingsTab;
use super::{ButtonStyle, Desktop, LocalAction, Overlay};
use gpui::{
    Context, Div, ElementId, FontWeight, ImageSource, KeyDownEvent, Window, div, img, prelude::*,
    px,
};
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
                Err(DeviceError::Busy) => {}
                Err(error) => self.device_error = Some(error),
            }
        }
    }
    pub(super) fn enumerate_devices(&mut self) {
        if let Some(request) = self.next_device_request() {
            self.device_error = device_submission_error(
                self.device_error,
                self.settings_port
                    .submit_device(DeviceIntent::Enumerate { request }),
            );
        }
    }
    pub(super) fn refresh_device_kind(&mut self, kind: DeviceKind) {
        if let Some(request) = self.next_device_request() {
            self.device_error = device_submission_error(
                self.device_error,
                self.settings_port
                    .submit_device(DeviceIntent::Refresh { request, kind }),
            );
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
            Err(DeviceError::Busy) => {}
            Err(error) => self.device_error = Some(error),
        }
    }
    pub(super) fn stop_one_device(&mut self, kind: DeviceKind, cx: &mut Context<Self>) {
        if let Some(session) = self.device_session {
            self.device_error = device_submission_error(
                self.device_error,
                self.settings_port
                    .submit_device(DeviceIntent::StopDevice { session, kind }),
            );
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
                Err(DeviceError::Busy) => {}
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
    fn device_option_ids(&self, kind: DeviceKind) -> Vec<ElementId> {
        let mut ids = vec![("device-default", kind as usize).into()];
        if let Ok(snapshot) = &self.devices {
            ids.extend(
                snapshot
                    .devices
                    .as_slice()
                    .iter()
                    .filter(|d| d.kind == kind)
                    .map(|d| device_option_id(kind, &d.id)),
            );
        }
        ids
    }
    pub(super) fn toggle_device_picker(&mut self, kind: DeviceKind) {
        if self.device_picker == Some(kind) {
            self.device_picker = None;
            self.pending_device_focus = Some(("device-picker", kind as usize).into());
        } else {
            self.device_picker = Some(kind);
            let chosen = match kind {
                DeviceKind::Microphone => &self.preferences.active.devices.microphone,
                DeviceKind::Speaker => &self.preferences.active.devices.speaker,
                DeviceKind::Camera => &self.preferences.active.devices.camera,
            };
            let id = match chosen {
                DeviceSelection::SystemDefault => ("device-default", kind as usize).into(),
                DeviceSelection::Specific(id) => device_option_id(kind, id),
            };
            self.pending_device_focus = Some(if self.device_option_ids(kind).contains(&id) {
                id
            } else {
                ("device-default", kind as usize).into()
            });
        }
    }
    fn device_picker_key(
        &mut self,
        kind: DeviceKind,
        event: &KeyDownEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if event.keystroke.modifiers.control || event.keystroke.modifiers.platform {
            return;
        }
        let key = event.keystroke.key.as_str();
        if self.device_picker != Some(kind) {
            if matches!(key, "down" | "up") {
                self.toggle_device_picker(kind);
                cx.stop_propagation();
                cx.notify();
            }
            return;
        }
        if key == "escape" {
            self.toggle_device_picker(kind);
        } else if matches!(key, "down" | "up" | "home" | "end") {
            let ids = self.device_option_ids(kind);
            let current = ids.iter().position(|id| {
                self.control_focus
                    .current(id)
                    .is_some_and(|f| f.is_focused(window))
            });
            if let Some(index) = option_destination(current, ids.len(), key) {
                self.pending_device_focus = ids.get(index).cloned();
            }
        } else {
            return;
        }
        self.keyboard_navigation = true;
        self.hover_hints.cancel();
        cx.stop_propagation();
        cx.notify();
    }
    fn device_selector(
        &self,
        kind: DeviceKind,
        chosen: &DeviceSelection,
        label: String,
        speaker: Option<Availability>,
        cx: &Context<Self>,
    ) -> Div {
        let expanded = self.device_picker == Some(kind);
        let mut row = div()
            .flex()
            .items_center()
            .gap_2()
            .child(
                self.select_control(
                    ("device-picker", kind as usize),
                    label,
                    expanded,
                    self.settings_availability(),
                    LocalAction::ToggleDevicePicker(kind),
                    cx,
                )
                .flex_1(),
            )
            .child(self.icon_button(
                ("refresh-device", kind as usize),
                ("refresh", "Geräte aktualisieren"),
                ButtonStyle::Quiet,
                Availability::available(),
                LocalAction::RefreshDevice(kind),
                cx,
            ));
        if let Some(availability) = speaker {
            row = row.child(self.icon_button(
                "speaker-test",
                ("play", "Klingelton abspielen"),
                ButtonStyle::Secondary,
                availability,
                LocalAction::StartDevice(DeviceKind::Speaker),
                cx,
            ));
        }
        let mut selector = div()
            .flex()
            .flex_col()
            .gap_2()
            .child(row)
            .on_key_down(cx.listener(move |this, event, window, cx| {
                this.device_picker_key(kind, event, window, cx)
            }));
        if expanded {
            let option = |id: ElementId, label: String, selection: DeviceSelection| {
                let selected = *chosen == selection;
                self.control(
                    id,
                    ButtonStyle::Secondary,
                    self.settings_availability(),
                    LocalAction::SelectDevice(kind, selection),
                    cx,
                )
                .justify_between()
                .border_0()
                .bg(if selected {
                    self.theme.selected
                } else {
                    self.theme.surface
                })
                .child(div().min_w_0().flex_1().text_ellipsis().child(label))
                .child(div().w(px(16.)).when(selected, |d| {
                    d.child(self.scaled_icon("check", 16., self.theme.accent))
                }))
            };
            let mut choices = div()
                .flex_none()
                .flex()
                .flex_col()
                .gap_1()
                .p_1()
                .rounded_lg()
                .border_1()
                .border_color(self.theme.input_border)
                .bg(self.theme.surface)
                .child(option(
                    ("device-default", kind as usize).into(),
                    self.l("Systemstandard").to_owned(),
                    DeviceSelection::SystemDefault,
                ));
            if let Ok(snapshot) = &self.devices {
                for device in snapshot
                    .devices
                    .as_slice()
                    .iter()
                    .filter(|d| d.kind == kind)
                {
                    choices = choices.child(option(
                        device_option_id(kind, &device.id),
                        device.label.as_str().to_owned(),
                        DeviceSelection::Specific(device.id.clone()),
                    ));
                }
            }
            selector = selector.child(choices);
        }
        selector
    }
    pub(super) fn device_page(&self, cx: &Context<Self>) -> Div {
        let mut page = div()
            .flex()
            .flex_col()
            .gap_5()
            .child(self.dialog_page_header(
                "Geräte",
                Some("Geräte werden für Meetings und Gespräche verwendet."),
            ));
        let snapshot = match &self.devices {
            Ok(snapshot) => snapshot,
            Err(error) => {
                if let Some(message) = device_error_key(*error) {
                    page = page.child(self.hint(self.l(message)).text_color(self.theme.danger));
                }
                return page;
            }
        };
        for kind in [
            DeviceKind::Microphone,
            DeviceKind::Speaker,
            DeviceKind::Camera,
        ] {
            let (title, purpose, chosen, state, preview) = match kind {
                DeviceKind::Microphone => (
                    "Mikrofon",
                    "Damit andere dich hören.",
                    &self.preferences.active.devices.microphone,
                    &snapshot.microphone,
                    snapshot.microphone_preview,
                ),
                DeviceKind::Speaker => (
                    "Lautsprecher",
                    "Für Stimmen und Hinweistöne.",
                    &self.preferences.active.devices.speaker,
                    &snapshot.speaker,
                    snapshot.speaker_test,
                ),
                DeviceKind::Camera => (
                    "Kamera",
                    "Damit andere dich sehen.",
                    &self.preferences.active.devices.camera,
                    &snapshot.camera,
                    snapshot.camera_preview,
                ),
            };
            let mut section = div()
                .flex_none()
                .flex()
                .flex_col()
                .gap_3()
                .when(kind != DeviceKind::Camera, |d| {
                    d.pb_5().border_b_1().border_color(self.theme.border)
                })
                .child(
                    div()
                        .flex()
                        .flex_col()
                        .gap_1()
                        .child(div().font_weight(FontWeight::MEDIUM).child(self.l(title)))
                        .child(self.hint(self.l(purpose))),
                );
            let effective = match state {
                SelectionState::Selected(id)
                | SelectionState::DefaultFallback {
                    effective: Some(id),
                    ..
                } => snapshot
                    .devices
                    .as_slice()
                    .iter()
                    .find(|d| d.kind == kind && d.id == *id)
                    .map(|d| d.label.as_str()),
                _ => None,
            };
            let selected_label = match chosen {
                DeviceSelection::SystemDefault => effective.map_or_else(
                    || self.l("Systemstandard").to_owned(),
                    |name| format!("{} ({name})", self.l("Systemstandard")),
                ),
                DeviceSelection::Specific(id) => snapshot
                    .devices
                    .as_slice()
                    .iter()
                    .find(|d| d.kind == kind && d.id == *id)
                    .map_or_else(
                        || self.l("Ausgewähltes Gerät nicht verfügbar").to_owned(),
                        |d| d.label.as_str().to_owned(),
                    ),
            };
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
            );
            let stopping =
                self.device_stopping.is_some() || matches!(preview, PreviewState::Stopping { .. });
            let check_availability = if active || stopping {
                Availability::pending()
            } else {
                Availability::when(
                    available,
                    "Für diesen Test ist kein verfügbares Gerät mit Freigabe ausgewählt.",
                )
            };
            section = section.child(self.device_selector(
                kind,
                chosen,
                selected_label,
                (kind == DeviceKind::Speaker).then_some(check_availability),
                cx,
            ));
            let state_hint = match state {
                SelectionState::Unknown => None,
                SelectionState::DefaultFallback { effective: Some(_), .. } => Some(self.lf("Gespeichertes Gerät fehlt. Verwendet wird: {device}.", &[("device", effective.unwrap_or(self.l("Systemstandard")))])),
                SelectionState::DefaultFallback { effective: None, .. } => Some(self.l("Gespeichertes Gerät fehlt. Bitte Systemstandard oder ein anderes Gerät wählen.").to_owned()),
                SelectionState::Unavailable(error) => device_error_key(*error).map(|key| self.l(key).to_owned()),
                SelectionState::Selected(_) => None,
            };
            if let Some(message) = state_hint {
                section = section.child(self.hint(&message));
            }
            let denied = matches!(
                preview,
                PreviewState::Failed {
                    error: DeviceError::PermissionDenied,
                    ..
                }
            );
            if kind != DeviceKind::Speaker {
                let permission = if kind == DeviceKind::Microphone {
                    talk_core::preferences::permissions::PermissionKind::Microphone
                } else {
                    talk_core::preferences::permissions::PermissionKind::Camera
                };
                if let Some(notice) = self.device_permission_notice(permission, denied, cx) {
                    section = section.child(notice);
                }
                if kind == DeviceKind::Microphone
                    && self.observation(permission).allows_device_preview()
                    && !denied
                {
                    section = section.child(
                        div()
                            .flex()
                            .flex_col()
                            .gap_2()
                            .child(self.hint(self.l("Mikrofon-Eingangspegel")))
                            .child(self.level_meter(snapshot.microphone_level)),
                    );
                }
            }
            if let Some(message) = device_preview_error(preview) {
                section = section.child(self.hint(self.l(message)).text_color(self.theme.danger));
            }
            if kind == DeviceKind::Camera {
                let permitted = self
                    .observation(talk_core::preferences::permissions::PermissionKind::Camera)
                    .allows_device_preview();
                let mut preview_box = div()
                    .relative()
                    .w_full()
                    .flex_none()
                    .rounded_lg()
                    .overflow_hidden()
                    .bg(self.theme.stage);
                // The frame owns the 16:9 allocation; the image contains its real
                // aspect ratio within it and never contributes an intrinsic height.
                preview_box.style().aspect_ratio = Some(16. / 9.);
                if let Some(image) = self.camera_image.clone() {
                    preview_box = preview_box.child(
                        img(image)
                            .absolute()
                            .inset_0()
                            .size_full()
                            .rounded_lg()
                            .object_fit(gpui::ObjectFit::Contain),
                    );
                }
                if active {
                    if matches!(preview, PreviewState::Starting { .. }) {
                        preview_box = preview_box.child(
                            div()
                                .absolute()
                                .inset_0()
                                .flex()
                                .items_center()
                                .justify_center()
                                .p_4()
                                .child(self.button_with_leading_icon(
                                    "camera-start",
                                    (self.l("Kameravorschau starten"), "video"),
                                    ButtonStyle::Stage,
                                    Availability::pending(),
                                    LocalAction::StartDevice(DeviceKind::Camera),
                                    cx,
                                )),
                        );
                    }
                    preview_box = preview_box.child(div().absolute().top_2().right_2().child(
                        self.icon_button(
                            "camera-stop",
                            ("square", "Kameravorschau beenden"),
                            ButtonStyle::Stage,
                            if stopping {
                                Availability::pending()
                            } else {
                                Availability::available()
                            },
                            LocalAction::StopDevice(DeviceKind::Camera),
                            cx,
                        ),
                    ));
                } else {
                    preview_box = preview_box.child(div().absolute().inset_0().flex().items_center().justify_center().p_4()
                        .child(self.button_with_leading_icon(
                            "camera-start", (self.l("Kameravorschau starten"), "video"), ButtonStyle::Stage,
                            if stopping { Availability::pending() } else { Availability::when(available && permitted,
                                "Für diesen Test ist kein verfügbares Gerät mit Freigabe ausgewählt.") },
                            LocalAction::StartDevice(DeviceKind::Camera), cx,
                        )));
                }
                section = section.child(preview_box);
            }
            page = page.child(section);
        }
        if let Some(message) = self.device_error.and_then(device_error_key) {
            page = page.child(self.hint(self.l(message)).text_color(self.theme.danger));
        }
        page
    }
}
fn device_submission_error(
    current: Option<DeviceError>,
    result: Result<(), DeviceError>,
) -> Option<DeviceError> {
    match result {
        Err(DeviceError::Busy) => current,
        result => result.err(),
    }
}

pub(super) fn device_error_key(error: DeviceError) -> Option<&'static str> {
    Some(match error {
        DeviceError::Unavailable => "Kein Gerät verfügbar.",
        DeviceError::PermissionDenied => {
            "Zugriff verweigert. Erlaube Mikrofon oder Kamera in den Systemeinstellungen."
        }
        DeviceError::Removed => "Das ausgewählte Gerät wurde entfernt.",
        // This label describes a terminal native-device result. Busy admission
        // is handled before storing an immediate submission error.
        DeviceError::Busy => {
            "Das Gerät konnte nicht geöffnet werden. Prüfe, ob eine andere App darauf zugreift, und versuche es erneut."
        }
        DeviceError::Stale => "Die Geräteauswahl hat sich geändert. Bitte erneut versuchen.",
        DeviceError::InvalidSelection => "Bitte wähle ein verfügbares Gerät.",
        DeviceError::CaptureFailed => "Der Gerätetest ist fehlgeschlagen.",
    })
}

fn device_preview_error(preview: PreviewState) -> Option<&'static str> {
    match preview {
        PreviewState::Failed { error, .. } if error != DeviceError::PermissionDenied => {
            device_error_key(error)
        }
        _ => None,
    }
}

fn device_option_id(kind: DeviceKind, id: &talk_core::preferences::devices::DeviceId) -> ElementId {
    gpui::SharedString::from(format!("device-{:?}-{}", kind, id.as_str())).into()
}

fn option_destination(current: Option<usize>, count: usize, key: &str) -> Option<usize> {
    if count == 0 {
        return None;
    }
    match key {
        "home" => Some(0),
        "end" => Some(count - 1),
        "down" => Some(current.map_or(0, |i| (i + 1) % count)),
        "up" => Some(current.map_or(count - 1, |i| (i + count - 1) % count)),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn busy_device_admission_does_not_replace_a_capture_failure() {
        assert!(device_error_key(DeviceError::Busy).is_some());
        assert_eq!(device_submission_error(None, Err(DeviceError::Busy)), None);
        assert_eq!(
            device_submission_error(Some(DeviceError::CaptureFailed), Err(DeviceError::Busy)),
            Some(DeviceError::CaptureFailed),
        );
        assert!(device_error_key(DeviceError::CaptureFailed).is_some());
        assert!(device_error_key(DeviceError::PermissionDenied).is_some());
        assert_eq!(
            device_submission_error(Some(DeviceError::CaptureFailed), Ok(())),
            None
        );
    }

    #[test]
    fn native_busy_preview_failure_remains_visible_after_admission() {
        assert_eq!(device_submission_error(None, Err(DeviceError::Busy)), None);
        assert_eq!(
            device_preview_error(PreviewState::Starting {
                request: RequestId(4)
            }),
            None,
        );
        let failed = PreviewState::Failed {
            request: RequestId(4),
            error: DeviceError::Busy,
        };
        assert_eq!(
            device_preview_error(failed),
            device_error_key(DeviceError::Busy)
        );
        assert!(device_preview_error(failed).is_some());
        assert_eq!(
            device_submission_error(Some(DeviceError::Busy), Err(DeviceError::Busy)),
            Some(DeviceError::Busy),
        );
        assert_eq!(device_preview_error(PreviewState::Stopped), None);
    }

    #[test]
    fn picker_navigation_is_bounded_and_wraps_without_committing_a_selection() {
        assert_eq!(option_destination(None, 0, "down"), None);
        assert_eq!(option_destination(None, 3, "down"), Some(0));
        assert_eq!(option_destination(None, 3, "up"), Some(2));
        assert_eq!(option_destination(Some(2), 3, "down"), Some(0));
        assert_eq!(option_destination(Some(0), 3, "up"), Some(2));
        assert_eq!(option_destination(Some(1), 3, "home"), Some(0));
        assert_eq!(option_destination(Some(1), 3, "end"), Some(2));
        assert_eq!(option_destination(Some(1), 3, "enter"), None);
        assert_eq!(option_destination(Some(0), 1, "up"), Some(0));
    }
}
