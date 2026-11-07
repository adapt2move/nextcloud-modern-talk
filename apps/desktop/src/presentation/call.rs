pub(super) mod consent;
mod dock;
pub(super) mod recovery;
use super::controls::Availability;
use super::{ButtonStyle, Desktop, LocalAction, Overlay};
use crate::presentation::motion::DialogSurface;
use gpui::{Context, Div, FontWeight, ImageSource, Window, div, img, prelude::*, px};
use talk_core::ui::UserIntent;
use talk_core::ui::presentation::{
    CallStage, CallTileId, CallTileKind, MediaState, RequiredGate, ScreenShareFailure,
    ScreenShareState, UiIntent,
};

// Sixteen remote cameras, the local camera, and one screen stage may coexist.
const MAX_CALL_TILES: usize = 18;

impl Desktop {
    pub(super) fn waiting_in_lobby(&self) -> bool {
        self.snapshot
            .call
            .as_ref()
            .is_some_and(|call| call.stage == CallStage::Required(RequiredGate::Lobby))
    }
    /// Resolve scoped stream frames through the authoritative snapshot metadata.
    pub fn set_call_images(
        &mut self,
        scope: Option<talk_core::ui::CallScope>,
        images: Vec<(u128, ImageSource)>,
        cx: &mut Context<Self>,
    ) {
        let images = images
            .into_iter()
            .filter_map(|(local_id, image)| {
                let stream = self
                    .snapshot
                    .call
                    .as_ref()
                    .filter(|call| Some(call.scope) == scope)?
                    .streams
                    .as_slice()
                    .iter()
                    .find(|stream| stream.local_id == local_id)?;
                Some((
                    CallTileId {
                        local_id,
                        kind: stream.kind,
                    },
                    image,
                ))
            })
            .collect();
        self.set_call_tiles(scope, images, cx);
    }

    pub fn set_call_tiles(
        &mut self,
        scope: Option<talk_core::ui::CallScope>,
        images: Vec<(CallTileId, ImageSource)>,
        cx: &mut Context<Self>,
    ) {
        let call = self.snapshot.call.as_ref().filter(|call| {
            Some(call.scope) == scope
                && matches!(call.stage, CallStage::Active | CallStage::Connecting)
        });
        let valid = |tile: CallTileId| {
            call.is_some_and(|call| {
                call.streams
                    .as_slice()
                    .iter()
                    .any(|stream| stream.local_id == tile.local_id && stream.kind == tile.kind)
            })
        };
        let mut changed = false;
        let mut index = 0;
        while index < self.call_images.len() {
            let (old_scope, tile, at, _) = &self.call_images[index];
            if Some(*old_scope) == scope
                && (at.elapsed() < std::time::Duration::from_secs(2)
                    || tile.kind == CallTileKind::Screen)
                && valid(*tile)
            {
                index += 1;
            } else {
                let (_, _, _, image) = self.call_images.remove(index);
                if let ImageSource::Render(old) = image {
                    cx.drop_image(old, None);
                }
                changed = true;
            }
        }
        if let Some(scope) = scope {
            for (tile, image) in images.into_iter().take(MAX_CALL_TILES) {
                if !valid(tile) {
                    continue;
                }
                if let Some(index) = self
                    .call_images
                    .iter()
                    .position(|(s, id, _, _)| *s == scope && *id == tile)
                {
                    let (_, _, _, old) = self.call_images.remove(index);
                    if let ImageSource::Render(old) = old {
                        cx.drop_image(old, None);
                    }
                }
                if self.call_images.len() < MAX_CALL_TILES {
                    self.call_images
                        .push((scope, tile, std::time::Instant::now(), image));
                    changed = true;
                }
            }
        }
        if changed {
            cx.notify();
        }
    }

    pub fn prejoin_view(&self, window: &Window, cx: &Context<Self>) -> Div {
        let t = self.theme;
        let call = self.prejoin.expect("explicit conference route");
        let title = self
            .snapshot
            .conversations
            .as_slice()
            .iter()
            .find(|r| r.call_scope == Some(call))
            .map_or("Gespräch", |r| r.title.as_str());
        let wide = window.viewport_size().width >= px(1000.) && self.scale <= 125;
        let preview=div().flex_1().min_w_0().flex().flex_col().gap_4()
            .child(self.title(self.l("Bereit, wenn du es bist.")))
            .child(self.hint(self.l("Wähle, wie du beitreten möchtest.")))
            .child(div().h(px(304.)).w_full().rounded(px(12.)).bg(t.stage_panel).text_color(t.stage_text).flex().flex_col().items_center().justify_center().gap_4()
                .child(self.avatar(self.snapshot.accounts.as_slice().iter().find(|a|Some(a.scope)==self.snapshot.active_account).map_or("?",|a|a.initials.as_str()),true))
                .child(div().font_weight(FontWeight::MEDIUM).child(self.l("Deine Kamera ist nicht aktiv")))
                .child(div().text_size(self.sp(13.)).text_color(t.stage_muted).child(self.l("Noch keine Vorschau verfügbar"))))
            .child(div().flex().flex_wrap().gap_3()
                .child(self.button_with_icon("mic-intent",(if self.microphone_intent{"Mikrofon beim Beitritt an"}else{"Mikrofon beim Beitritt aus"},if self.microphone_intent{"mic"}else{"mic-off"}),ButtonStyle::Secondary,true,LocalAction::ToggleMicIntent,cx))
                .child(self.button_with_icon("camera-intent",(if self.camera_intent{"Kamera beim Beitritt an"}else{"Kamera beim Beitritt aus"},if self.camera_intent{"video"}else{"video-off"}),ButtonStyle::Secondary,true,LocalAction::ToggleCameraIntent,cx)))
            .child(self.hint(self.l("Bis zum Beitritt bleiben Mikrofon und Kamera aus. Erforderliche Freigaben werden vorher geprüft.")));
        let mut devices = div()
            .flex_none()
            .when(wide, |d| d.w(px(326.)))
            .when(!wide, |d| d.w_full())
            .flex()
            .flex_col()
            .gap_4()
            .child(
                div()
                    .font_weight(FontWeight::MEDIUM)
                    .text_size(self.sp(16.))
                    .child(self.l("Deine Geräte")),
            );
        if !self.settings_port.device_preferences_applied() {
            devices = devices
                .child(self.hint(self.l("Gespeicherte Geräteauswahl wird noch angewendet.")));
        }
        for (symbol, name, selection) in [
            ("video", "Kamera", &self.preferences.active.devices.camera),
            (
                "mic",
                "Mikrofon",
                &self.preferences.active.devices.microphone,
            ),
            (
                "speaker",
                "Lautsprecher",
                &self.preferences.active.devices.speaker,
            ),
        ] {
            let label=match selection {
                talk_core::preferences::devices::DeviceSelection::SystemDefault=>self.l("Systemstandard"),
                talk_core::preferences::devices::DeviceSelection::Specific(id)=>self.devices.as_ref().ok().and_then(|s|s.devices.as_slice().iter().find(|d|d.id==*id)).map_or(self.l("Gespeichertes Gerät fehlt. Bitte Systemstandard oder ein anderes Gerät wählen."),|d|d.label.as_str()),
            };
            devices = devices.child(
                div()
                    .flex()
                    .gap_3()
                    .items_center()
                    .child(self.scaled_icon(symbol, 20., t.muted))
                    .child(div().min_w_0().child(self.l(name)).child(self.hint(label))),
            );
        }
        devices = devices
            .child(self.button("device-settings",self.l("Geräteeinstellungen öffnen"),ButtonStyle::Secondary,true,LocalAction::OpenDeviceSettings,cx))
            .child(self.hint(self.l("Gerätetests sind in den Einstellungen verfügbar. Beim Öffnen dieses Beitritts startet keine Aufnahme.")))
            .child(
                div().pt_4().border_t_1().border_color(t.border).child(
                    self.button_with_icon(
                        "join-prepared",
                        ("Jetzt teilnehmen", "video"),
                        ButtonStyle::Primary,
                        true,
                        LocalAction::PrejoinJoin,
                        cx,
                    )
                    .w_full()
                    .min_h(px(46.)),
                ),
            )
            .child(self.hint(self.l("Du trittst erst mit „Jetzt teilnehmen“ bei.")))
            .when(self.local_feedback_visible(), |d| {
                d.child(self.feedback_view(cx))
            });
        div().size_full().p_3().bg(t.chrome()).child(
            self.dialog_panel(
                DialogSurface::Prejoin,
                self.content_card(
                    div()
                        .flex()
                        .flex_col()
                        .track_focus(&self.modal_focus)
                        .tab_group()
                        .child(
                            div()
                                .relative()
                                .flex_none()
                                .min_h(px(74.))
                                .px_6()
                                .py_3()
                                .border_b_1()
                                .border_color(t.border)
                                .flex()
                                .items_center()
                                .gap_4()
                                .child(self.button_with_leading_icon(
                                    "prejoin-back",
                                    ("Zurück zum Gespräch", "arrow-left"),
                                    ButtonStyle::Secondary,
                                    Availability::available(),
                                    LocalAction::Close,
                                    cx,
                                ))
                                .child(
                                    div()
                                        .min_w_0()
                                        .child(
                                            div()
                                                .font_weight(FontWeight::MEDIUM)
                                                .text_ellipsis()
                                                .child(title.to_owned()),
                                        )
                                        .child(self.hint(self.l("Beitritt vorbereiten"))),
                                ),
                        )
                        .child(
                            self.scroll_area(super::navigation::Region::Prejoin)
                                .flex_1()
                                .min_h_0()
                                .p_6()
                                .child(
                                    div()
                                        .flex()
                                        .when(!wide, |d| d.flex_col())
                                        .gap_8()
                                        .child(preview)
                                        .child(devices),
                                ),
                        ),
                ),
            ),
        )
    }
    pub fn call_view(&self, window: &Window, cx: &Context<Self>) -> Div {
        let t = self.theme;
        let call = self.snapshot.call.as_ref().expect("call projection");
        let wide = window.viewport_size().width >= px(900.) && self.scale < 150;
        if matches!(call.stage, CallStage::Ended | CallStage::Failed) {
            return self.call_outcome(call.stage == CallStage::Failed, cx);
        }
        // Waiting admission never paints room media, participants or chat.
        let lobby = self.waiting_in_lobby();
        let joined = matches!(
            call.stage,
            CallStage::Active | CallStage::Connecting | CallStage::Reconnecting
        );
        let show_chat = joined && super::docked_call_chat(wide, self.call_chat_open, self.overlay);
        let show_people = joined && wide && self.call_participants_open && self.overlay.is_none();
        let (stage_height, feature_height, _filmstrip_height) =
            stage_metrics(f32::from(window.viewport_size().height), self.scale);
        let rail_width = 160. * f32::from(self.scale) / 100.;
        let feature_height = (feature_height - (rail_width * 9. / 16. - 90.)).max(74.);
        let status = match call.stage {
            CallStage::Incoming => "Eingehender Anruf",
            CallStage::Outgoing => "Anruf wird angefragt …",
            CallStage::Connecting => "Verbindung wird aufgebaut …",
            CallStage::Active => "Im Gespräch",
            CallStage::Reconnecting => "Verbindung unterbrochen",
            CallStage::Required(_) => "Wartet auf Freigabe",
            CallStage::Ended => "Anruf beendet",
            CallStage::Failed => "Verbindung fehlgeschlagen",
        };
        let mut stage = self
            .scroll_area(super::navigation::Region::Call)
            .flex_1()
            .min_w_0()
            .min_h_0()
            .p_4()
            .flex()
            .flex_col()
            .gap_4();
        if let Some(label) = sharing_label(call.sharing).filter(|_| !lobby) {
            stage = stage.child(
                div()
                    .flex_none()
                    .p_3()
                    .rounded(px(10.))
                    .bg(t.stage_panel)
                    .text_color(if matches!(call.sharing, ScreenShareState::Failed(_)) {
                        t.warning
                    } else {
                        t.stage_text
                    })
                    .child(self.l(label)),
            );
        }
        if self
            .call_recovery
            .visible(call.scope, std::time::Instant::now())
        {
            stage = stage.child(
                div()
                    .flex_none()
                    .h(px(36.))
                    .px_3()
                    .rounded(px(8.))
                    .bg(gpui::rgb(0xfff4d6))
                    .text_color(gpui::rgb(0x795300))
                    .flex()
                    .items_center()
                    .gap_3()
                    .text_size(self.sp(12.))
                    .child(self.l("Ton oder Bild fehlen?"))
                    .child(self.button(
                        "recover-call-media",
                        "Neu beitreten",
                        ButtonStyle::Secondary,
                        true,
                        LocalAction::Intent(UiIntent::RecoverCallMedia(call.scope)),
                        cx,
                    )),
            );
        }
        if call.stage == CallStage::Reconnecting {
            stage = stage.child(
                div()
                    .flex_none()
                    .h(px(32.))
                    .px_3()
                    .rounded(px(8.))
                    .bg(gpui::rgb(0xfff4d6))
                    .text_color(gpui::rgb(0x795300))
                    .text_size(self.sp(12.))
                    .flex()
                    .items_center()
                    .gap_2()
                    .child(self.call_spinner(
                        "call-reconnecting-spinner".into(),
                        16.,
                        gpui::rgb(0x795300).into(),
                    ))
                    .child(self.l("Verbindung wird wiederhergestellt …"))
                    .when(
                        self.call_reconnecting_since.is_some_and(|(scope, at)| {
                            scope == call.scope
                                && at.elapsed() >= std::time::Duration::from_secs(15)
                        }),
                        |d| {
                            d.child(self.button(
                                "reconnect-call",
                                "Erneut verbinden",
                                ButtonStyle::Secondary,
                                true,
                                LocalAction::Intent(UiIntent::ReconnectCall(call.scope)),
                                cx,
                            ))
                        },
                    ),
            );
        }
        if let CallStage::Required(gate) = call.stage
            && self.overlay != Some(Overlay::RecordingConsent)
        {
            let (title, description) = match gate {
                RequiredGate::MicrophonePermission => (
                    "Mikrofonfreigabe erforderlich",
                    "Talk braucht Zugriff auf dein Mikrofon, um dich hörbar zu machen.",
                ),
                RequiredGate::CameraPermission => (
                    "Kamerafreigabe erforderlich",
                    "Talk braucht Zugriff auf deine Kamera, um dich sichtbar zu machen.",
                ),
                RequiredGate::ScreenPermission => (
                    "Bildschirmfreigabe erforderlich",
                    "Die Bildschirmfreigabe benötigt eine Freigabe.",
                ),
                RequiredGate::RecordingConsent => (
                    "Zustimmung zur Aufzeichnung erforderlich",
                    "Der Server kann Audio und Video aufzeichnen. Ohne Zustimmung kein Beitritt.",
                ),
                RequiredGate::SecurityPolicy => (
                    "Sicherheitsvorgabe noch nicht erfüllt",
                    "Die Sicherheitsvorgaben für diesen Anruf sind noch nicht erfüllt.",
                ),
                RequiredGate::Lobby => (
                    "Du wartest im Warteraum",
                    "Der Beitritt wird erst fortgesetzt, wenn diese Voraussetzung erfüllt ist.",
                ),
            };
            stage = stage.child(
                div()
                    .flex_1()
                    .w_full()
                    .flex()
                    .items_center()
                    .justify_center()
                    .child(
                        div()
                            .w_full()
                            .max_w(px(480.))
                            .p_6()
                            .rounded(px(12.))
                            .bg(if t.dark { t.stage_panel } else { t.surface })
                            .border_1().border_color(if t.dark { gpui::rgb(0x52677f).into() } else { t.border })
                            .text_color(if t.dark { t.stage_text } else { t.text })
                            .flex()
                            .flex_col()
                            .gap_4()
                            .child(
                                div()
                                    .text_size(self.sp(20.))
                                    .font_weight(FontWeight::MEDIUM)
                                    .child(self.l(title)),
                            )
                            .child(div().text_size(self.sp(14.)).text_color(if t.dark { t.stage_text } else { t.muted }).child(self.l(description)))
                            .when(gate == RequiredGate::Lobby, |d| {
                                let timer = self.snapshot.conversation_panel.as_ref()
                                    .filter(|p| !p.details_stale && p.context.account == call.scope.account && p.context.conversation == call.scope.conversation)
                                    .and_then(|p| p.details.as_ref()).and_then(|details| details.lobby_timer)
                                    .and_then(|at| self.settings_port.local_datetime(at))
                                    .and_then(|time| super::localization::local_datetime(self.locale, time.as_str()));
                                d.when_some(timer, |d,time| d.child(self.hint(&self.lf("Beginn: {time}", &[("time",time.as_str())]))))
                            })
                             .when_some(match gate {
                        RequiredGate::MicrophonePermission => Some(talk_core::preferences::permissions::PermissionKind::Microphone),
                        RequiredGate::CameraPermission => Some(talk_core::preferences::permissions::PermissionKind::Camera),
                        _ => None,
                    }, |d, kind| {
                        if self.observation(kind).can_open_settings {
                            d.child(self.button("gate-system-settings", "Systemeinstellungen öffnen", ButtonStyle::Secondary, true,
                                LocalAction::Permission(kind, talk_core::preferences::permissions::PermissionAction::OpenSystemSettings), cx))
                        } else {
                            d.child(div().text_size(self.sp(12.)).text_color(if t.dark { t.stage_text } else { t.muted })
                                .child(self.l("Systemeinstellungen sind auf diesem System nicht verfügbar.")))
                        }
                    })
                    .when(gate == RequiredGate::RecordingConsent, |d| {
                                d.child(self.button(
                                    "required-gate",
                                    "Zustimmung prüfen",
                                    ButtonStyle::Secondary,
                                    true,
                                    LocalAction::Open(Overlay::RecordingConsent),
                                    cx,
                                ))
                            })
                            .child(self.button("gate-leave", "Anruf verlassen", ButtonStyle::Quiet, true,
                                LocalAction::Intent(UiIntent::Legacy(UserIntent::Leave(call.scope))), cx)),
                    ),
            );
        }
        if matches!(call.stage, CallStage::Outgoing) {
            stage = stage.child(
                div()
                    .flex_1()
                    .w_full()
                    .flex()
                    .flex_col()
                    .items_center()
                    .justify_center()
                    .gap_4()
                    .child(
                        self.incoming_call_avatar(
                            self.call_stage_avatar(
                                call.participants
                                    .as_slice()
                                    .iter()
                                    .find(|p| !p.self_user)
                                    .map_or("?", |p| p.initials.as_str()),
                                96.,
                            ),
                        ),
                    )
                    .child(div().text_size(self.sp(20.)).child(
                        self.lf("{name} wird angerufen …", &[("name", call.title.as_str())]),
                    ))
                    .child(self.button(
                        "outgoing-leave",
                        "Auflegen",
                        ButtonStyle::Danger,
                        true,
                        LocalAction::Intent(UiIntent::Legacy(UserIntent::Leave(call.scope))),
                        cx,
                    )),
            );
        }
        let pinned = self.call_focus.and_then(|(scope, id)| {
            (scope == call.scope
                && call
                    .participants
                    .as_slice()
                    .iter()
                    .any(|p| p.local_id == id))
            .then_some(id)
        });
        if !matches!(call.stage, CallStage::Required(_) | CallStage::Outgoing) {
            if let Some(screen) = call
                .screen_stage()
                .filter(|screen| screen.tile.kind == CallTileKind::Screen)
            {
                let image = self
                    .call_images
                    .iter()
                    .find(|(scope, tile, _, _)| *scope == call.scope && *tile == screen.tile)
                    .map(|(_, _, _, image)| image);
                let visual = if let Some(image) = image {
                    div().w_full().h_full().child(
                        img(image.clone())
                            .w_full()
                            .h_full()
                            .object_fit(gpui::ObjectFit::Contain),
                    )
                } else {
                    div()
                        .w_full()
                        .h_full()
                        .flex()
                        .items_center()
                        .justify_center()
                        .gap_2()
                        .child(self.call_spinner(
                            "call-screen-waiting-spinner".into(),
                            18.,
                            t.stage_muted,
                        ))
                        .child(self.lf(
                            "{name} teilt den Bildschirm",
                            &[("name", screen.name.as_str())],
                        ))
                };
                let width = (f32::from(window.viewport_size().width)
                    - if show_chat || show_people { 332. } else { 32. })
                .min(feature_height * 16. / 9.)
                .max(160.);
                stage = stage.child(
                    div().w_full().flex().justify_center().child(
                        div()
                            .w(px(width))
                            .h(px(width * 9. / 16.))
                            .flex_none()
                            .rounded(px(12.))
                            .bg(t.stage_panel)
                            .child(visual),
                    ),
                );
            }
            // Reserve the docked chat before choosing a grid so tiles keep a
            // useful width when the panel opens or the window is narrow.
            let stage_width = (f32::from(window.viewport_size().width)
                - if show_chat || show_people { 300. } else { 0. }
                - 56.)
                .max(160.);
            let columns =
                gallery_columns(stage_width, self.scale, call.participants.as_slice().len());
            let focused = pinned.or_else(|| {
                if self.call_speaker != Some(call.scope) || call.screen_stage().is_some() {
                    return None;
                }
                call.participants
                    .as_slice()
                    .iter()
                    .find(|participant| participant.speaking)
                    .or_else(|| call.participants.as_slice().first())
                    .map(|participant| participant.local_id)
            });
            let filmstrip = focused.is_some() || call.screen_stage().is_some();
            let count = call.participants.as_slice().len();
            let pip = count == 2
                && !filmstrip
                && call.participants.as_slice().iter().any(|p| p.self_user)
                && call.participants.as_slice().iter().any(|p| !p.self_user);
            let strip_count = count.saturating_sub(usize::from(focused.is_some())).max(1);
            let rows = count.max(1).div_ceil(usize::from(columns));
            let tile_width = if pip {
                stage_width.min(stage_height * 16. / 9.)
            } else if filmstrip {
                rail_width
            } else {
                ((stage_width - f32::from(columns.saturating_sub(1)) * 12.) / f32::from(columns))
                    .min(
                        ((stage_height - rows.saturating_sub(1) as f32 * 12.) / rows as f32) * 16.
                            / 9.,
                    )
                    .max(160. * f32::from(self.scale) / 100.)
            };
            let tile_height = tile_width * 9. / 16.;
            // Leave enough space for enlarged text in the self-view without
            // letting it grow beyond the remote tile at narrow widths.
            let pip_width =
                (200. * (f32::from(self.scale) / 100.).sqrt()).min((tile_width - 24.).max(160.));
            let mut feature = None;
            let mut self_pip = None;
            let mut tiles = div()
                .flex()
                .flex_wrap()
                .justify_center()
                .gap_3()
                .when(filmstrip, |d| {
                    d.w(px(strip_count as f32 * rail_width
                        + strip_count.saturating_sub(1) as f32 * 12.))
                })
                .when(!filmstrip, |d| {
                    d.w(px(f32::from(if pip { 1 } else { columns }) * tile_width
                        + f32::from(if pip {
                            0
                        } else {
                            columns.saturating_sub(1)
                        }) * 12.))
                })
                // The viewport scrolls rows that cannot fit at narrow sizes.
                .flex_none();
            for participant in call.participants.as_slice() {
                let image = self
                    .call_images
                    .iter()
                    .find(|(scope, local, _, _)| {
                        *scope == call.scope
                            && Some(*local) == call.camera_tile(participant.local_id)
                    })
                    .map(|(_, _, _, image)| image);
                let visual = if let Some(image) = image {
                    div().w_full().flex_1().min_h_0().child(
                        img(image.clone())
                            .w_full()
                            .h_full()
                            .object_fit(gpui::ObjectFit::Contain),
                    )
                } else {
                    div()
                        .flex_1()
                        .min_h_0()
                        .pb(self.sp(18.))
                        .flex()
                        .items_center()
                        .justify_center()
                        .child(self.call_stage_avatar(
                            participant.initials.as_str(),
                            if focused == Some(participant.local_id) {
                                104.
                            } else if pip && participant.self_user {
                                56.
                            } else if filmstrip {
                                40.
                            } else {
                                (tile_height * 0.4).clamp(56., 128.)
                            },
                        ))
                };
                let tile = div()
                    .relative()
                    .min_w_0()
                    .when(focused == Some(participant.local_id), |tile| tile.w_full())
                    .when(focused != Some(participant.local_id), |tile| {
                        tile.w(px(if pip && participant.self_user {
                            pip_width
                        } else {
                            tile_width
                        }))
                    })
                    .h(px(if focused == Some(participant.local_id) {
                        feature_height
                    } else if pip && participant.self_user {
                        pip_width * 9. / 16.
                    } else {
                        tile_height
                    }))
                    .rounded(px(12.))
                    .border(px(if participant.speaking { 3. } else { 1. }))
                    .border_color(if participant.speaking {
                        stage_speaking_color(t)
                    } else {
                        t.stage_panel
                    })
                    .group("call-tile")
                    .when(call.stage == CallStage::Reconnecting, |d| d.opacity(0.7))
                    .bg(if pip && participant.self_user {
                        t.stage
                    } else {
                        t.stage_panel
                    })
                    .flex()
                    .flex_col()
                    .items_center()
                    .justify_center()
                    .gap_2()
                    .p_4()
                    .child(visual)
                    .when(call.stage == CallStage::Reconnecting, |d| {
                        d.child(
                            div()
                                .absolute()
                                .bottom(px(8.))
                                .right(px(8.))
                                .size(px(28.))
                                .rounded_full()
                                .bg(t.stage)
                                .flex()
                                .items_center()
                                .justify_center()
                                .child(self.scaled_icon("wifi-off", 16., t.stage_text)),
                        )
                    })
                    .when(participant.speaking, |d| {
                        d.child(
                            div()
                                .absolute()
                                .inset(px(3.))
                                .rounded(px(10.))
                                .border_1()
                                .border_color(gpui::rgba(0x8fe6b655)),
                        )
                    })
                    .children(self.call_reaction(
                        call.scope,
                        if participant.self_user {
                            None
                        } else {
                            Some(participant.local_id)
                        },
                    ))
                    .child(
                        div()
                            .absolute()
                            .bottom(px(8.))
                            .left(px(8.))
                            .max_w(px((if focused == Some(participant.local_id) {
                                stage_width
                            } else if pip && participant.self_user {
                                pip_width
                            } else {
                                tile_width
                            } - 16.)
                                .max(0.)))
                            .px_2()
                            .py_1()
                            .rounded(px(8.))
                            .bg(gpui::rgba(0x00000073))
                            .min_w_0()
                            .flex()
                            .items_center()
                            .gap_2()
                            .child(
                                div()
                                    .flex_1()
                                    .min_w_0()
                                    .text_size(self.sp(12.))
                                    .text_ellipsis()
                                    .font_weight(FontWeight::MEDIUM)
                                    .child(
                                        if participant.self_user
                                            && participant.name.as_str().is_empty()
                                        {
                                            self.l("Du").to_owned()
                                        } else {
                                            format!(
                                                "{}{}",
                                                participant.name.as_str(),
                                                if participant.self_user {
                                                    self.l(" (Du)")
                                                } else {
                                                    ""
                                                }
                                            )
                                        },
                                    ),
                            )
                            .when(participant.speaking, |row| {
                                row.child(
                                    self.call_speaking_bars(
                                        gpui::SharedString::from(format!(
                                            "speaking-bars-{}-{}",
                                            call.scope.call.as_local(),
                                            participant.local_id
                                        ))
                                        .into(),
                                        stage_speaking_color(t),
                                    ),
                                )
                            })
                            .when(participant.call_microphone == Some(false), |row| {
                                row.child(self.scaled_icon("mic-off", 14., t.stage_muted))
                            }),
                    )
                    .when(pinned == Some(participant.local_id), |tile| {
                        tile.child(
                            div()
                                .absolute()
                                .top(px(8.))
                                .left(px(8.))
                                .px_2()
                                .py_1()
                                .rounded(px(8.))
                                .bg(gpui::rgba(0x00000073))
                                .text_size(self.sp(12.))
                                .child(self.l("Angeheftet")),
                        )
                    })
                    .when(participant.raised_hand, |tile| {
                        tile.child(
                            self.call_hand_badge(
                                gpui::SharedString::from(format!(
                                    "call-hand-badge-{}-{}",
                                    call.scope.call.as_local(),
                                    participant.local_id
                                ))
                                .into(),
                                div()
                                    .absolute()
                                    .top(px(12.))
                                    .right(px(12.))
                                    .size(px(24.))
                                    .rounded_full()
                                    .bg(t.stage)
                                    .flex()
                                    .items_center()
                                    .justify_center()
                                    .child(self.scaled_icon(
                                        "hand",
                                        16.,
                                        if t.dark { t.warning } else { t.warning_bg },
                                    )),
                            ),
                        )
                    })
                    .child(
                        self.icon_button(
                            gpui::SharedString::from(format!(
                                "focus-call-{}",
                                participant.local_id
                            )),
                            (
                                "pin",
                                if pinned == Some(participant.local_id) {
                                    "Lösen"
                                } else if focused == Some(participant.local_id) {
                                    "Raster"
                                } else {
                                    "Person hervorheben"
                                },
                            ),
                            ButtonStyle::Stage,
                            Availability::available().hinted(),
                            LocalAction::FocusCallParticipant(
                                call.scope,
                                if focused == Some(participant.local_id) {
                                    None
                                } else {
                                    Some(participant.local_id)
                                },
                            ),
                            cx,
                        )
                        .absolute()
                        .top(px(8.))
                        .right(px(8.))
                        .opacity(if focused == Some(participant.local_id) {
                            1.
                        } else {
                            0.
                        })
                        .group_hover("call-tile", |style| style.opacity(1.))
                        .focus(|style| style.opacity(1.)),
                    );
                let tile = self.reveal(
                    gpui::SharedString::from(format!(
                        "call-tile-{}-{}",
                        call.scope.call.as_local(),
                        participant.local_id
                    )),
                    tile,
                );
                if pip && participant.self_user {
                    self_pip = Some(tile);
                } else if focused == Some(participant.local_id) {
                    feature = Some(tile);
                } else {
                    tiles = tiles.child(tile);
                }
            }
            if let Some(tile) = feature {
                stage = stage.child(tile);
            }
            if call.participants.as_slice().is_empty() {
                tiles = tiles.child(
                    div()
                        .p_6()
                        .child(self.l("Teilnehmerdaten noch nicht verfügbar.")),
                );
            }
            stage = if pip {
                stage.child(
                    div()
                        .flex_1()
                        .w_full()
                        .flex()
                        .items_center()
                        .justify_center()
                        .child(
                            div()
                                .relative()
                                .w(px(tile_width))
                                .h(px(tile_height))
                                .child(tiles)
                                .when_some(self_pip, |d, tile| {
                                    d.child(
                                        div().absolute().bottom(px(12.)).right(px(12.)).child(tile),
                                    )
                                }),
                        ),
                )
            } else if filmstrip {
                stage.child(
                    div()
                        .id("call-filmstrip")
                        .w_full()
                        .min_w_0()
                        .overflow_x_scroll()
                        .child(tiles),
                )
            } else {
                stage.child(
                    div()
                        .flex_1()
                        .w_full()
                        .flex()
                        .items_center()
                        .justify_center()
                        .child(tiles),
                )
            };
        }
        let body = div()
            .flex_1()
            .min_h_0()
            .flex()
            .child(stage)
            .when(show_chat, |d| {
                d.child(
                    div()
                        .w(px(300.))
                        .flex_none()
                        .bg(if t.dark {
                            gpui::rgb(0x16202f).into()
                        } else {
                            t.surface
                        })
                        .border_l_1()
                        .border_color(if t.dark {
                            gpui::rgb(0x2c3b52).into()
                        } else {
                            t.border
                        })
                        .text_color(t.text)
                        .child(self.call_chat(cx)),
                )
            })
            .when(show_people, |d| {
                d.child(
                    div()
                        .w(px(300.))
                        .flex_none()
                        .bg(if t.dark {
                            gpui::rgb(0x16202f).into()
                        } else {
                            t.surface
                        })
                        .border_l_1()
                        .border_color(if t.dark {
                            gpui::rgb(0x2c3b52).into()
                        } else {
                            t.border
                        })
                        .text_color(t.text)
                        .flex()
                        .flex_col()
                        .child(
                            div()
                                .px_4()
                                .py_4()
                                .border_b_1()
                                .border_color(t.border)
                                .text_size(self.sp(16.))
                                .font_weight(FontWeight::MEDIUM)
                                .child(call.title.as_str().to_owned()),
                        )
                        .child(
                            self.scroll_area(super::navigation::Region::Overlay)
                                .flex_1()
                                .min_h_0()
                                .p_4()
                                .child(self.call_participants_body(cx)),
                        ),
                )
            });
        let mut primary_controls = div()
            .flex_1()
            .min_w_0()
            .flex()
            .flex_wrap()
            .items_center()
            .justify_center()
            .gap_2();
        for (id, symbol, label, state, action) in [
            (
                "microphone",
                if call.microphone == MediaState::Active {
                    "mic"
                } else {
                    "mic-off"
                },
                if matches!(call.microphone, MediaState::Active | MediaState::Requested) {
                    "Stummschalten (M)"
                } else {
                    "Mikrofon einschalten (M)"
                },
                call.microphone,
                LocalAction::Intent(UiIntent::SetMicrophone {
                    call: call.scope,
                    enabled: !matches!(call.microphone, MediaState::Active | MediaState::Requested),
                }),
            ),
            (
                "camera",
                if call.camera == MediaState::Active {
                    "video"
                } else {
                    "video-off"
                },
                if matches!(call.camera, MediaState::Active | MediaState::Requested) {
                    "Kamera ausschalten (V)"
                } else {
                    "Kamera einschalten (V)"
                },
                call.camera,
                LocalAction::Intent(UiIntent::SetCamera {
                    call: call.scope,
                    enabled: !matches!(call.camera, MediaState::Active | MediaState::Requested),
                }),
            ),
            (
                "screen-share",
                "monitor-up",
                if sharing_active(call.sharing) {
                    "Freigabe beenden"
                } else {
                    "Bildschirm teilen"
                },
                if sharing_active(call.sharing) {
                    MediaState::Active
                } else if matches!(call.sharing, ScreenShareState::Failed(_)) {
                    MediaState::Off
                } else {
                    call.share
                },
                LocalAction::Intent(if sharing_active(call.sharing) {
                    UiIntent::StopScreen(call.scope)
                } else {
                    UiIntent::ShareScreen(call.scope)
                }),
            ),
        ] {
            let available = !call.end_pending
                && matches!(
                    state,
                    MediaState::Off | MediaState::Active | MediaState::Requested
                );
            let control = self
                .round_icon_button(
                    id,
                    (symbol, label),
                    ButtonStyle::Stage,
                    Availability::when(available, media_reason(state)).hinted(),
                    action,
                    cx,
                )
                .size(px(44.))
                .min_h(px(44.))
                .rounded_full()
                .when(id != "screen-share" && state == MediaState::Off, |d| {
                    d.bg(gpui::rgb(0x9e3e32)).text_color(gpui::white())
                });
            let mut group = div()
                .relative()
                .flex()
                .items_center()
                .rounded_full()
                .bg(t.stage_panel)
                .child(control);
            if let Some(kind) = match id {
                "microphone" => Some(talk_core::preferences::devices::DeviceKind::Microphone),
                "camera" => Some(talk_core::preferences::devices::DeviceKind::Camera),
                _ => None,
            } {
                group = group.child(
                    self.icon_button(
                        ("call-device", kind as usize),
                        (
                            "chevron-up",
                            if id == "microphone" {
                                "Mikrofon auswählen"
                            } else {
                                "Kamera auswählen"
                            },
                        ),
                        ButtonStyle::Stage,
                        Availability::available().hinted(),
                        LocalAction::CallDevicePicker(call.scope, kind),
                        cx,
                    )
                    .w(px(28.))
                    .border_l_1()
                    .border_color(t.stage)
                    .rounded_full(),
                );
                if self.call_device_picker == Some((call.scope, kind)) {
                    group = group
                        .children(self.call_device_popover(call.scope, cx))
                        .child(
                            div()
                                .absolute()
                                .bottom(px(42.))
                                .left(px(16.))
                                .child(self.scaled_icon("chevron-down", 14., t.surface)),
                        );
                }
            }
            primary_controls = primary_controls.child(group);
        }
        // Keep the controls on one line when the side panels become overlays.
        let narrow_controls = window.viewport_size().width < px(900.) || self.scale >= 150;
        let chat_unread = self
            .snapshot
            .conversations
            .as_slice()
            .iter()
            .find(|row| {
                row.context.account == call.scope.account
                    && row.context.conversation == call.scope.conversation
            })
            .map_or(0, |row| row.unread);
        let mut interaction_controls = div().flex().items_center().gap_2();
        interaction_controls = interaction_controls.child(
            self.round_icon_button(
                "call-hand",
                (
                    "hand",
                    if call.hand_raised {
                        "Hand senken (H)"
                    } else {
                        "Hand heben (H)"
                    },
                ),
                ButtonStyle::Stage,
                Availability::when(
                    call.hand_supported && !call.end_pending,
                    if call.end_pending {
                        "Eine Anfrage wird bereits bearbeitet."
                    } else {
                        "Diese Funktion ist noch nicht verfügbar."
                    },
                )
                .hinted(),
                LocalAction::Intent(UiIntent::SetCallHand {
                    call: call.scope,
                    raised: !call.hand_raised,
                }),
                cx,
            )
            .size(px(44.))
            .min_h(px(44.))
            .rounded_full(),
        );
        {
            let mut reaction_control = div().relative().child(
                self.round_icon_button(
                    "call-reactions",
                    ("smile", "Reaktion senden"),
                    ButtonStyle::Stage,
                    Availability::when(
                        !call.reactions.as_slice().is_empty() && !call.end_pending,
                        if call.end_pending {
                            "Eine Anfrage wird bereits bearbeitet."
                        } else {
                            "Diese Funktion ist noch nicht verfügbar."
                        },
                    )
                    .hinted(),
                    LocalAction::ToggleCallReactions(call.scope),
                    cx,
                )
                .size(px(44.))
                .min_h(px(44.))
                .rounded_full(),
            );
            if !call.reactions.as_slice().is_empty() && self.call_reactions_open == Some(call.scope)
            {
                let mut menu = div()
                    .id("call-reaction-menu")
                    .absolute()
                    .bottom(px(54.))
                    .left(px(-72.))
                    .w(px(320.))
                    .max_h(px(200.))
                    .overflow_y_scroll()
                    .p_2()
                    .rounded(px(12.))
                    .border_1()
                    .border_color(t.border)
                    .bg(t.stage_panel)
                    .shadow_lg()
                    .flex()
                    .flex_wrap()
                    .gap_1();
                for (index, emoji) in call.reactions.as_slice().iter().take(6).enumerate() {
                    menu = menu.child(
                        self.button(
                            ("call-reaction", index),
                            emoji.as_str().to_owned(),
                            ButtonStyle::Stage,
                            true,
                            LocalAction::Intent(UiIntent::SendCallReaction {
                                call: call.scope,
                                emoji: emoji.clone(),
                            }),
                            cx,
                        )
                        .min_w(px(44.))
                        .min_h(px(44.))
                        .text_size(self.sp(28.))
                        .p_1(),
                    );
                }
                reaction_control = reaction_control.child(menu);
            }
            if !call
                .participants
                .as_slice()
                .iter()
                .any(|person| person.self_user)
            {
                reaction_control = reaction_control.children(self.call_reaction(call.scope, None));
            }
            interaction_controls = interaction_controls.child(reaction_control);
        }
        let mut navigation_controls = div().flex().items_center().gap_2().ml_3();
        for (id, symbol, label, action) in [
            (
                "call-participants",
                "users",
                "Personen",
                LocalAction::ToggleCallParticipants,
            ),
            ("call-chat", "chat", "Chat", LocalAction::ToggleCallChat),
        ] {
            navigation_controls = navigation_controls.child(
                self.round_icon_button(
                    id,
                    (symbol, label),
                    ButtonStyle::Stage,
                    Availability::available().hinted(),
                    action,
                    cx,
                )
                .size(px(44.))
                .min_h(px(44.))
                .rounded_full()
                .relative()
                .when(
                    id == "call-chat" && !show_chat && self.overlay != Some(Overlay::CallChat),
                    |button| {
                        button.when(chat_unread > 0, |d| {
                            d.child(
                                div()
                                    .absolute()
                                    .top(px(-2.))
                                    .right(px(-2.))
                                    .min_w(px(18.))
                                    .h(px(18.))
                                    .px_1()
                                    .rounded_full()
                                    .bg(t.accent)
                                    .text_color(t.on_accent)
                                    .text_size(px(12.))
                                    .line_height(px(18.))
                                    .text_center()
                                    .child(chat_unread.min(99).to_string()),
                            )
                        })
                    },
                )
                .when(
                    (id == "call-chat" && show_chat) || (id == "call-participants" && show_people),
                    |d| d.bg(t.accent),
                ),
            );
        }
        let mut more = div().relative().child(
            self.round_icon_button(
                "call-more",
                ("more", "Weitere Anrufaktionen"),
                ButtonStyle::Stage,
                Availability::available().hinted(),
                LocalAction::ToggleCallSecondary,
                cx,
            )
            .size(px(44.))
            .min_h(px(44.))
            .rounded_full()
            .relative()
            .when(
                narrow_controls
                    && !show_chat
                    && self.overlay != Some(Overlay::CallChat)
                    && chat_unread > 0,
                |button| {
                    button.child(
                        div()
                            .absolute()
                            .top(px(-2.))
                            .right(px(-2.))
                            .min_w(px(18.))
                            .h(px(18.))
                            .px_1()
                            .rounded_full()
                            .bg(t.accent)
                            .text_color(t.on_accent)
                            .text_size(px(12.))
                            .line_height(px(18.))
                            .text_center()
                            .child(chat_unread.min(99).to_string()),
                    )
                },
            ),
        );
        if narrow_controls {
            if self.call_secondary_open {
                more = more.child(
                    div()
                        .absolute()
                        .bottom(px(54.))
                        .right(px(0.))
                        .p_2()
                        .rounded(px(12.))
                        .bg(t.stage_panel)
                        .shadow_lg()
                        .occlude()
                        .child(interaction_controls.child(navigation_controls)),
                );
            }
            primary_controls = primary_controls.child(more);
        } else {
            if self.call_secondary_open {
                more = more.child(
                    div()
                        .absolute()
                        .bottom(px(54.))
                        .right(px(0.))
                        .w(px(240.))
                        .p_2()
                        .rounded(px(12.))
                        .bg(t.stage_panel)
                        .shadow_lg()
                        .occlude()
                        .flex()
                        .flex_col()
                        .child(self.button_with_leading_icon(
                            "call-more-minimize",
                            ("Anruf minimieren", "chevron-down"),
                            ButtonStyle::Stage,
                            Availability::available(),
                            LocalAction::MinimizeCall(call.scope),
                            cx,
                        ))
                        .child(self.button_with_leading_icon(
                            "call-more-fullscreen",
                            ("Vollbild umschalten", "maximize-2"),
                            ButtonStyle::Stage,
                            Availability::available(),
                            LocalAction::ToggleCallFullscreen(call.scope),
                            cx,
                        )),
                );
            }
            primary_controls = primary_controls
                .child(interaction_controls.ml_3())
                .child(navigation_controls.child(more));
        }
        let controls = div()
            .flex_none()
            .flex()
            .items_center()
            .gap_2()
            .px_4()
            .py_3()
            .border_t_1()
            .border_color(t.stage_panel)
            .when(!narrow_controls, |d| d.child(div().w(px(120.)).flex_none()))
            .when(!lobby, |d| d.child(primary_controls))
            .child(
                self.button_with_icon(
                    "leave-call",
                    ("Verlassen", "phone-down"),
                    ButtonStyle::StageDanger,
                    true,
                    LocalAction::Intent(UiIntent::Legacy(UserIntent::Leave(call.scope))),
                    cx,
                )
                .min_w(px(120.))
                .h(px(44.))
                .min_h(px(44.))
                .rounded_full(),
            );
        div().size_full().p_3().bg(t.chrome()).child(
            self.content_card(
                div()
                    .relative()
                    .flex()
                    .flex_col()
                    .bg(t.stage)
                    .text_color(t.stage_text)
                    .child(
                        div()
                            .flex_none()
                            .flex()
                            .items_center()
                            .gap_4()
                            .px_5()
                            .py_3()
                            .min_h(px(64.))
                            .border_b_1()
                            .border_color(t.stage_panel)
                            .child(
                                div()
                                    .flex_1()
                                    .min_w_0()
                                    .child(
                                        div()
                                            .text_size(self.sp(16.))
                                            .font_weight(FontWeight::MEDIUM)
                                            .text_ellipsis()
                                            .child(format!("{}{}", call.title.as_str(), self.call_started.filter(|(scope, _)| *scope == call.scope).map(|(_, started)| { let seconds = started.elapsed().as_secs(); format!(" · {}:{:02}", seconds / 60, seconds % 60) }).unwrap_or_default())),
                                    )
                                    .child(
                                        div()
                                            .text_size(self.sp(12.))
                                            .text_color(t.stage_muted)
                                            .child(if matches!(call.stage, CallStage::Required(_)) { self.l(status).to_owned() } else { format!(
                                                "{} · {}",
                                                self.l(status),
                                                self.count(
                                                    super::localization::Count::Participants,
                                                    call.participants.as_slice().len() as u64
                                                )
                                            ) }),
                                    ),
                            )
                            .when(call.participants.as_slice().len() > 1, |header| {
                                let speaker_mode =
                                    self.call_speaker == Some(call.scope) && pinned.is_none();
                                header.child(self.icon_button(
                                    "call-layout",
                                    (
                                        if speaker_mode { "grid" } else { "user" },
                                        if speaker_mode {
                                            "Raster"
                                        } else {
                                            "Sprecheransicht"
                                        },
                                    ),
                                    ButtonStyle::Stage,
                                    Availability::available().hinted(),
                                    LocalAction::SpeakerCallLayout(call.scope, !speaker_mode),
                                    cx,
                                ))
                            })
                            .when(call.recording == talk_core::ui::presentation::CallRecordingState::Active, |header| {
                                header.child(div().px_3().py_1().rounded_full()
                                    .bg(if t.dark { t.danger_bg } else { t.danger })
                                    .text_color(if t.dark { t.danger } else { t.danger_bg }).text_size(self.sp(12.))
                                    .font_weight(FontWeight::BOLD).child(format!("● {}", self.l("Aufzeichnung läuft"))))
                            })
                            .child(self.icon_button(
                                "minimize-call",
                                ("chevron-down", "Anruf minimieren"),
                                ButtonStyle::Stage,
                                Availability::available().labelled(),
                                LocalAction::MinimizeCall(call.scope),
                                cx,
                            ))
                            .child(self.icon_button(
                                "fullscreen-call",
                                (if window.is_fullscreen() { "minimize-2" } else { "maximize-2" }, "Vollbild umschalten"),
                                ButtonStyle::Stage,
                                Availability::available().labelled(),
                                LocalAction::ToggleCallFullscreen(call.scope),
                                cx,
                            )),
                    )
                    .when(call.stage == CallStage::Connecting, |d| d.child(div().h(px(2.)).w_full().bg(t.stage_panel).child(div().h_full().w(gpui::relative(0.4)).bg(t.accent))))
                    .when(call.end_pending || call.end_failed, |d| d.child(div().px_5().py_2().text_size(self.sp(12.))
                        .child(self.l(if call.end_pending { "Anruf wird für alle beendet …" } else { "Der Anrufstatus ist unklar. Verlasse den Anruf und prüfe, ob er beendet wurde." }))))
                    .when_some(
                        match call.recording {
                            talk_core::ui::presentation::CallRecordingState::Starting => Some("Aufzeichnung startet …"),
                            talk_core::ui::presentation::CallRecordingState::Stopping => Some("Aufzeichnung wird beendet …"),
                            _ => None,
                        },
                        |d, status| d.child(div().flex_none().px_5().py_2().text_size(self.sp(12.)).child(self.l(status))),
                    )
                    .when(self.local_feedback_visible(), |d| {
                        d.child(
                            div()
                                .flex_none()
                                .px_5()
                                .py_2()
                                .child(self.feedback_view(cx)),
                        )
                    })
                    .child(body)
                    .when(!matches!(call.stage, CallStage::Outgoing | CallStage::Required(_)), |d| d.child(controls))
                    .when(call.recording == talk_core::ui::presentation::CallRecordingState::Failed && self.dismissed_recording_failure != Some(call.scope), |d| d.child(
                        div().absolute().top(px(68.)).left(px(16.)).right(px(16.)).min_h(px(36.)).px_3().rounded(px(8.))
                            .bg(t.danger_bg).text_color(t.danger).flex().items_center().gap_3()
                            .child(div().flex_1().min_w_0().text_size(self.sp(12.)).child(self.l("Aufzeichnung fehlgeschlagen. Bitte die moderierende Person, sie zu prüfen.")))
                            .child(self.icon_button("dismiss-recording-failure", ("x", "Schließen"), ButtonStyle::Quiet, Availability::available().hinted(), LocalAction::DismissRecordingFailure(call.scope), cx)))),
            )
            .bg(t.stage),
        )
    }
    fn call_reaction(
        &self,
        scope: talk_core::ui::CallScope,
        participant: Option<u128>,
    ) -> Option<gpui::AnyElement> {
        let (current, reaction, at) = self.recent_call_reaction.as_ref()?;
        let fixture = self.fixture_call_reaction == Some((scope, reaction.sequence));
        if *current != scope
            || reaction.participant != participant
            || (!fixture && at.elapsed() >= std::time::Duration::from_millis(1800))
        {
            return None;
        }
        let id = gpui::SharedString::from(format!("call-reaction-animation-{}", reaction.sequence));
        let element = div()
            .absolute()
            .left(gpui::relative(0.5))
            .bottom(gpui::relative(0.5))
            .text_size(self.sp(34.))
            .child(reaction.emoji.as_str().to_owned());
        Some(if fixture {
            element.into_any_element()
        } else {
            self.call_reaction_motion(id.into(), element)
        })
    }
    fn call_stage_avatar(&self, initials: &str, size: f32) -> Div {
        let seed = initials
            .bytes()
            .fold(0usize, |n, b| n.wrapping_mul(31).wrapping_add(b as usize));
        let colors = [0x645287, 0x436d80, 0x87583e, 0x4e725e, 0x725366];
        div()
            .flex_none()
            .size(px(size))
            .rounded(px(size / 3.))
            .bg(gpui::rgb(colors[seed % colors.len()]))
            .text_color(gpui::white())
            .text_size(px(size * 0.36))
            .line_height(px(size * 0.5))
            .font_weight(FontWeight::MEDIUM)
            .flex()
            .items_center()
            .justify_center()
            .child(initials.to_owned())
    }
    fn call_outcome(&self, failed: bool, cx: &Context<Self>) -> Div {
        let t = self.theme;
        let call = self
            .snapshot
            .call
            .as_ref()
            .expect("terminal call projection");
        let rejoin_allowed = self.selected_call().is_some_and(|entry| match entry {
            super::state::CallEntry::Direct(scope) | super::state::CallEntry::Conference(scope) => {
                scope == call.scope
            }
        });
        let duration = self
            .call_ended_duration
            .filter(|(scope, _)| *scope == call.scope)
            .map(|(_, duration)| duration)
            .or_else(|| {
                self.call_started
                    .filter(|(scope, _)| *scope == call.scope)
                    .map(|(_, started)| started.elapsed())
            })
            .map(|duration| {
                let seconds = duration.as_secs();
                self.lf(
                    "Dauer {duration}",
                    &[("duration", &format!("{}:{:02}", seconds / 60, seconds % 60))],
                )
            });
        div().size_full().p_3().bg(t.chrome()).child(self.content_card(
            div().size_full().bg(t.subtle).text_color(t.text).flex().flex_col()
                .child(div().flex_none().px_6().py_4().border_b_1().border_color(t.border)
                    .font_weight(FontWeight::MEDIUM).child(call.title.as_str().to_owned()))
                .child(div().flex_1().min_h_0().flex().items_center().justify_center().p_6().child(
                    div().w_full().max_w(px(320.)).flex().flex_col().items_center().gap_4().text_center()
                        .child(div().size(px(48.)).rounded_full().bg(if failed { t.danger_bg } else { t.surface })
                            .flex().items_center().justify_center().child(self.scaled_icon(if failed { "cloud" } else { "check-circle" }, 24., if failed { t.danger } else { t.muted })))
                        .child(div().text_size(self.sp(20.)).font_weight(FontWeight::MEDIUM).child(self.l(if failed { "Verbindung fehlgeschlagen" } else { "Anruf beendet" })))
                        .child(div().text_size(self.sp(14.)).child(if failed { self.l("Der Anruf konnte nicht verbunden werden.").to_owned() } else { duration.unwrap_or_else(|| self.l("Du bist nicht mehr im Anruf.").to_owned()) }))
                        .child(self.button("rejoin-call", if call.direct { "Erneut anrufen" } else { "Erneut beitreten" }, ButtonStyle::Primary,
                            Availability::when(rejoin_allowed, "Ein erneuter Anruf ist für dieses Gespräch derzeit nicht freigegeben."), LocalAction::Call, cx).w_full()
                            .when(!rejoin_allowed, |button| button.bg(t.surface).text_color(t.muted)))
                        .when(!rejoin_allowed, |d| d.child(self.hint(self.l("Ein erneuter Anruf ist für dieses Gespräch derzeit nicht freigegeben."))))
                        .child(self.button("return-conversation", "Zurück zum Gespräch", ButtonStyle::Secondary, true, LocalAction::ReturnConversation, cx).w_full())
                        .when(self.local_feedback_visible(), |d| d.child(self.feedback_view(cx)))
                ))))
    }
    pub(super) fn call_chat(&self, cx: &Context<Self>) -> Div {
        let t = self.theme;
        let Some(call) = &self.snapshot.call else {
            return div();
        };
        let row = self.selected().filter(|r| {
            r.context.account == call.scope.account
                && r.context.conversation == call.scope.conversation
        });
        let mut body = div().size_full().flex().flex_col();
        if self.overlay != Some(Overlay::CallChat) {
            body = body.child(
                div()
                    .flex_none()
                    .px_4()
                    .py_4()
                    .border_b_1()
                    .border_color(t.border)
                    .font_weight(FontWeight::MEDIUM)
                    .text_size(self.sp(16.))
                    .child(call.title.as_str().to_owned()),
            );
        }
        if let Some(row) = row {
            body = body
                .child(self.conversation_search(cx))
                .child(self.call_message_timeline(cx))
                .child(self.composer_view(row, true, cx));
        } else {
            body = body.child(self.state_note(
                "chat",
                "Gespräch öffnen",
                "Nachrichten dieses Anrufs sind noch nicht geöffnet.",
            ));
            if let Some(row) = self
                .snapshot
                .conversations
                .as_slice()
                .iter()
                .find(|r| r.call_scope == Some(call.scope))
            {
                body = body.child(self.button(
                    "call-open-chat",
                    "Gespräch öffnen",
                    ButtonStyle::Secondary,
                    true,
                    LocalAction::Navigate(row.context),
                    cx,
                ));
            }
        }
        body
    }
}
fn media_reason(state: MediaState) -> &'static str {
    match state {
        MediaState::Denied => "Die Gerätefreigabe wurde nicht erteilt.",
        MediaState::Requested => "Freigabe ausstehend",
        _ => "Diese Medienfunktion ist derzeit nicht verfügbar.",
    }
}

fn media_label(microphone: bool, state: MediaState) -> &'static str {
    match (microphone, state) {
        (true, MediaState::Active) => "Mikrofon an",
        (true, MediaState::Off) => "Mikrofon aus",
        (true, MediaState::Requested) => "Mikrofon angefragt",
        (true, MediaState::Denied) => "Mikrofon nicht erlaubt",
        (true, MediaState::Unavailable) => "Mikrofon nicht verfügbar",
        (false, MediaState::Active) => "Kamera an",
        (false, MediaState::Off) => "Kamera aus",
        (false, MediaState::Requested) => "Kamera angefragt",
        (false, MediaState::Denied) => "Kamera nicht erlaubt",
        (false, MediaState::Unavailable) => "Kamera nicht verfügbar",
    }
}

fn stage_metrics(window_height: f32, scale: u16) -> (f32, f32, f32) {
    // The header gains height with text scaling; reserve that before fitting a
    // single large tile or a shared surface and its participant rail.
    let extra_header = ((f32::from(scale) / 100.) - 1.).max(0.) * 40.;
    let available = (window_height - 250. - extra_header).max(180.);
    let filmstrip = 90.;
    let feature = (available - filmstrip - 16.).max(74.);
    (available, feature, filmstrip)
}

// The stage remains dark in both app appearances. Lift the existing success
// token for that surface instead of using its light-surface foreground unchanged.
fn stage_speaking_color(theme: super::Theme) -> gpui::Hsla {
    gpui::Hsla {
        l: theme.success.l.max(0.70),
        ..theme.success
    }
}

/// Bound the gallery to readable tiles without relying on participant identity
/// or ordering. The scroll area owns rows that do not fit vertically.
fn gallery_columns(width: f32, scale: u16, participants: usize) -> u16 {
    let logical_width = width.max(0.) * 100. / f32::from(scale.max(1));
    let capacity = if logical_width >= 900. {
        3
    } else if logical_width >= 520. {
        2
    } else {
        1
    };
    capacity.min(participants.clamp(1, 3) as u16)
}

#[cfg(test)]
mod gallery_tests {
    use super::{gallery_columns, stage_metrics, stage_speaking_color};

    #[test]
    fn shared_stage_and_filmstrip_fit_at_minimum_window_height() {
        for height in [580., 800.] {
            for scale in [100, 200] {
                let (available, feature, filmstrip) = stage_metrics(height, scale);
                let rail = filmstrip * f32::from(scale) / 100.;
                let feature = (feature - (rail - filmstrip)).max(74.);
                assert!(feature + rail + 16. <= available);
            }
            assert!(stage_metrics(height, 200).0 < stage_metrics(height, 100).0);
        }
    }

    #[test]
    fn speaking_ring_contrasts_with_the_stage_in_both_appearances() {
        fn luminance(color: gpui::Hsla) -> f32 {
            let color = gpui::Rgba::from(color);
            let linear = |value: f32| {
                if value <= 0.04045 {
                    value / 12.92
                } else {
                    ((value + 0.055) / 1.055).powf(2.4)
                }
            };
            0.2126 * linear(color.r) + 0.7152 * linear(color.g) + 0.0722 * linear(color.b)
        }
        for palette in [
            crate::presentation::theme::Palette::Blue,
            crate::presentation::theme::Palette::Sage,
        ] {
            for appearance in [
                crate::presentation::theme::Appearance::Light,
                crate::presentation::theme::Appearance::Dark,
            ] {
                let theme = crate::presentation::theme::Theme::resolve(
                    palette,
                    appearance,
                    gpui::WindowAppearance::Light,
                );
                let ring = luminance(stage_speaking_color(theme));
                let panel = luminance(theme.stage_panel);
                assert!((ring.max(panel) + 0.05) / (ring.min(panel) + 0.05) >= 3.);
            }
        }
    }

    #[test]
    fn gallery_respects_width_scale_and_participant_count() {
        assert_eq!(gallery_columns(1200., 100, 8), 3);
        assert_eq!(gallery_columns(700., 100, 8), 2);
        assert_eq!(gallery_columns(700., 150, 8), 1);
        assert_eq!(gallery_columns(1200., 100, 1), 1);
        assert_eq!(gallery_columns(0., 100, 0), 1);
    }
}

fn sharing_active(state: ScreenShareState) -> bool {
    matches!(
        state,
        ScreenShareState::Choosing | ScreenShareState::Starting | ScreenShareState::Sharing
    )
}
fn sharing_label(state: ScreenShareState) -> Option<&'static str> {
    match state {
        ScreenShareState::Idle | ScreenShareState::Failed(ScreenShareFailure::Cancelled) => None,
        ScreenShareState::Choosing => {
            Some("Wähle den Bildschirm oder das Fenster für die Freigabe.")
        }
        ScreenShareState::Starting => Some("Bildschirmfreigabe wird gestartet …"),
        ScreenShareState::Sharing => Some("Dein Bildschirm wird geteilt."),
        ScreenShareState::Failed(ScreenShareFailure::Unavailable) => {
            Some("Bildschirmfreigabe ist auf diesem System nicht verfügbar.")
        }
        ScreenShareState::Failed(ScreenShareFailure::PermissionDenied) => {
            Some("Bildschirmzugriff verweigert. Prüfe die Systemeinstellungen.")
        }
        ScreenShareState::Failed(ScreenShareFailure::SourceClosed) => Some(
            "Die geteilte Quelle wurde geschlossen. Wähle erneut einen Bildschirm oder ein Fenster.",
        ),
        ScreenShareState::Failed(ScreenShareFailure::TimedOut) => {
            Some("Die Bildschirmfreigabe hat zu lange gedauert. Versuche es erneut.")
        }
    }
}

#[cfg(test)]
mod media_contract_tests {
    use super::*;
    #[test]
    fn share_controls_and_copy_follow_the_authoritative_state() {
        for state in [
            ScreenShareState::Choosing,
            ScreenShareState::Starting,
            ScreenShareState::Sharing,
        ] {
            assert!(sharing_active(state));
            assert!(sharing_label(state).is_some());
        }
        assert!(!sharing_active(ScreenShareState::Idle));
        assert_eq!(
            sharing_label(ScreenShareState::Failed(ScreenShareFailure::Cancelled)),
            None
        );
        for reason in [
            ScreenShareFailure::Unavailable,
            ScreenShareFailure::PermissionDenied,
            ScreenShareFailure::SourceClosed,
            ScreenShareFailure::TimedOut,
        ] {
            assert!(!sharing_active(ScreenShareState::Failed(reason)));
            let key = sharing_label(ScreenShareState::Failed(reason)).unwrap();
            for locale in [
                talk_core::preferences::Locale::English,
                talk_core::preferences::Locale::Spanish,
            ] {
                assert_ne!(crate::presentation::localization::text(locale, key), key);
            }
        }
    }
}
