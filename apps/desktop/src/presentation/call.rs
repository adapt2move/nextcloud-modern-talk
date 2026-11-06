mod dock;
use super::controls::Availability;
use super::{ButtonStyle, Desktop, LocalAction, Overlay};
use crate::presentation::motion::DialogSurface;
use gpui::{Context, Div, FontWeight, ImageSource, Window, div, img, prelude::*, px};
use talk_core::ui::UserIntent;
use talk_core::ui::presentation::{
    CallStage, CallTileId, CallTileKind, MediaState, RequiredGate, UiIntent,
};

impl Desktop {
    /// Existing camera-frame producer. Screen producers can pass typed tile
    /// identities to `set_call_tiles` once their scoped snapshot is connected.
    pub fn set_call_images(
        &mut self,
        scope: Option<talk_core::ui::CallScope>,
        images: Vec<(u128, ImageSource)>,
        cx: &mut Context<Self>,
    ) {
        self.set_call_tiles(
            scope,
            images
                .into_iter()
                .map(|(participant, image)| {
                    (
                        CallTileId {
                            participant,
                            kind: CallTileKind::Camera,
                        },
                        image,
                    )
                })
                .collect(),
            cx,
        );
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
            call.is_some_and(|call| match tile.kind {
                CallTileKind::Camera => call
                    .participants
                    .as_slice()
                    .iter()
                    .any(|p| p.local_id == tile.participant),
                CallTileKind::Screen => call
                    .screen_stage
                    .as_ref()
                    .is_some_and(|screen| screen.tile == tile),
            })
        };
        let mut changed = false;
        let mut index = 0;
        while index < self.call_images.len() {
            let (old_scope, tile, at, _) = &self.call_images[index];
            if Some(*old_scope) == scope
                && at.elapsed() < std::time::Duration::from_secs(2)
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
            for (tile, image) in images.into_iter().take(17) {
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
                if self.call_images.len() < 17 {
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
        let wide = window.viewport_size().width >= px(1000.) && self.scale < 150;
        if matches!(call.stage, CallStage::Ended | CallStage::Failed) {
            return self.call_outcome(call.stage == CallStage::Failed, cx);
        }
        let show_chat = super::docked_call_chat(wide, self.call_chat_open, self.overlay);
        let stage_height = (f32::from(window.viewport_size().height) - 260.).max(180.);
        let feature_height = (stage_height - 176.).clamp(220., 360.);
        let status = match call.stage {
            CallStage::Incoming => "Eingehender Anruf",
            CallStage::Outgoing => "Anruf wird angefragt …",
            CallStage::Connecting => "Verbindung wird aufgebaut …",
            CallStage::Active => "Im Gespräch",
            CallStage::Reconnecting => "Verbindung unterbrochen",
            CallStage::Required(_) => "Beitritt wartet auf Freigabe",
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
        if call.stage == CallStage::Reconnecting {
            stage = stage.child(
                self.reveal(
                    "call-reconnecting",
                    div()
                        .flex_none()
                        .p_4()
                        .rounded(px(10.))
                        .bg(t.warning_bg)
                        .text_color(t.warning)
                        .child(self.l("Verbindung unterbrochen"))
                        .child(div().text_size(self.sp(12.)).child(self.l(
                            "Mikrofon, Kamera und Freigabe bleiben beim erneuten Verbinden aus.",
                        )))
                        .child(self.button(
                            "reconnect-call",
                            "Erneut verbinden",
                            ButtonStyle::Secondary,
                            true,
                            LocalAction::Intent(UiIntent::ReconnectCall(call.scope)),
                            cx,
                        )),
                ),
            );
        }
        if let CallStage::Required(gate) = call.stage {
            let reason = match gate {
                RequiredGate::MicrophonePermission => "Mikrofonfreigabe erforderlich",
                RequiredGate::CameraPermission => "Kamerafreigabe erforderlich",
                RequiredGate::ScreenPermission => "Bildschirmfreigabe erforderlich",
                RequiredGate::RecordingConsent => "Zustimmung zur Aufzeichnung erforderlich",
                RequiredGate::SecurityPolicy => "Sicherheitsvorgabe noch nicht erfüllt",
                RequiredGate::Lobby => "Du wartest im Warteraum",
            };
            stage = stage.child(
                div()
                    .flex_none()
                    .p_4()
                    .rounded(px(10.))
                    .bg(t.warning_bg)
                    .text_color(t.warning)
                    .flex()
                    .flex_col()
                    .gap_3()
                    .child(self.l(reason))
                    .child(self.hint(self.l(
                        "Der Beitritt wird erst fortgesetzt, wenn diese Voraussetzung erfüllt ist.",
                    )))
                    .child(self.button(
                        "required-gate",
                        if gate == RequiredGate::RecordingConsent {
                            "Aufzeichnung zustimmen"
                        } else {
                            "Freigabe prüfen"
                        },
                        ButtonStyle::Secondary,
                        gate == RequiredGate::RecordingConsent,
                        LocalAction::Intent(UiIntent::ResolveRequiredGate {
                            call: call.scope,
                            gate,
                        }),
                        cx,
                    )),
            );
        }
        if let Some(screen) = call
            .screen_stage
            .as_ref()
            .filter(|screen| screen.tile.kind == CallTileKind::Screen)
        {
            let image = self
                .call_images
                .iter()
                .find(|(scope, tile, _, _)| *scope == call.scope && *tile == screen.tile)
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
                    .flex()
                    .flex_col()
                    .items_center()
                    .justify_center()
                    .gap_4()
                    .child(self.scaled_icon("monitor-up", 40., t.stage_muted))
                    .child(
                        div()
                            .text_size(self.sp(13.))
                            .text_color(t.stage_muted)
                            .child(self.l("Der geteilte Inhalt ist hier noch nicht verfügbar.")),
                    )
            };
            stage = stage.child(
                div()
                    .w_full()
                    .h(px(feature_height))
                    .flex_none()
                    .rounded(px(12.))
                    .bg(t.stage_panel)
                    .flex()
                    .flex_col()
                    .items_center()
                    .gap_4()
                    .p_4()
                    .child(visual)
                    .child(self.lf(
                        "{name} teilt den Bildschirm",
                        &[("name", screen.name.as_str())],
                    )),
            );
        } else if call.share == MediaState::Active {
            stage = stage.child(
                div()
                    .p_4()
                    .rounded(px(10.))
                    .bg(t.stage_panel)
                    .text_color(t.stage_text)
                    .child(
                        div()
                            .font_weight(FontWeight::MEDIUM)
                            .child(self.l("Bildschirmfreigabe")),
                    )
                    .child(self.hint(self.l("Der geteilte Inhalt ist hier noch nicht verfügbar."))),
            );
        }
        // Reserve the docked chat before choosing a grid so tiles keep a
        // useful width when the panel opens or the window is narrow.
        let stage_width =
            (f32::from(window.viewport_size().width) - if show_chat { 300. } else { 0. } - 56.)
                .max(160.);
        let columns = gallery_columns(stage_width, self.scale, call.participants.as_slice().len());
        let pinned = self.call_focus.and_then(|(scope, id)| {
            (scope == call.scope
                && call
                    .participants
                    .as_slice()
                    .iter()
                    .any(|p| p.local_id == id))
            .then_some(id)
        });
        let focused = pinned.or_else(|| {
            if self.call_speaker != Some(call.scope) || call.screen_stage.is_some() {
                return None;
            }
            call.participants
                .as_slice()
                .iter()
                .find(|participant| participant.speaking)
                .or_else(|| call.participants.as_slice().first())
                .map(|participant| participant.local_id)
        });
        let filmstrip = focused.is_some() || call.screen_stage.is_some();
        let count = call.participants.as_slice().len();
        let strip_count = count.saturating_sub(usize::from(focused.is_some())).max(1);
        let rows = count.max(1).div_ceil(usize::from(columns));
        let tile_height = if filmstrip {
            160.
        } else if count <= 2 {
            330.
        } else {
            ((stage_height - rows.saturating_sub(1) as f32 * 12.) / rows as f32).clamp(160., 210.)
        };
        let mut feature = None;
        let tile_width = if filmstrip {
            200.
        } else {
            ((stage_width - f32::from(columns.saturating_sub(1)) * 12.) / f32::from(columns))
                .max(160.)
        };
        let mut tiles = div()
            .flex()
            .flex_wrap()
            .justify_center()
            .gap_3()
            .when(filmstrip, |d| {
                d.w(px(
                    strip_count as f32 * 200. + strip_count.saturating_sub(1) as f32 * 12.
                ))
            })
            .when(!filmstrip, |d| d.w_full())
            // The viewport scrolls rows that cannot fit at narrow sizes.
            .flex_none();
        for participant in call.participants.as_slice() {
            let image = self
                .call_images
                .iter()
                .find(|(scope, local, _, _)| {
                    *scope == call.scope
                        && *local
                            == CallTileId {
                                participant: participant.local_id,
                                kind: CallTileKind::Camera,
                            }
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
                    .flex()
                    .items_center()
                    .justify_center()
                    .child(self.person_avatar(
                        participant.initials.as_str(),
                        if focused == Some(participant.local_id) {
                            104.
                        } else if filmstrip {
                            64.
                        } else {
                            84.
                        },
                    ))
            };
            let tile = div()
                .relative()
                .min_w_0()
                .when(focused == Some(participant.local_id), |tile| tile.w_full())
                .when(focused != Some(participant.local_id), |tile| {
                    tile.w(px(tile_width))
                })
                .min_h(px(if focused == Some(participant.local_id) {
                    feature_height
                } else {
                    tile_height
                }))
                .rounded(px(12.))
                .border_2()
                .border_color(if participant.speaking {
                    stage_speaking_color(t)
                } else {
                    t.stage_panel
                })
                .group("call-tile")
                .bg(t.stage_panel)
                .flex()
                .flex_col()
                .items_center()
                .justify_center()
                .gap_2()
                .p_4()
                .child(visual)
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
                        .flex_none()
                        .w_full()
                        .min_w_0()
                        .flex()
                        .items_center()
                        .gap_2()
                        .child(
                            div()
                                .flex_1()
                                .min_w_0()
                                .text_size(self.sp(13.))
                                .font_weight(FontWeight::MEDIUM)
                                .child(
                                    if participant.self_user && participant.name.as_str().is_empty()
                                    {
                                        self.l("Du").to_owned()
                                    } else {
                                        format!(
                                            "{}{}",
                                            participant.name.as_str(),
                                            if participant.self_user {
                                                self.l(" · Du")
                                            } else {
                                                ""
                                            }
                                        )
                                    },
                                ),
                        )
                        .when(participant.call_microphone == Some(false), |row| {
                            row.child(self.scaled_icon("mic-off", 14., t.stage_muted))
                        }),
                )
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
                                .left(px(12.))
                                .size(px(30.))
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
                        gpui::SharedString::from(format!("focus-call-{}", participant.local_id)),
                        (
                            "pin",
                            if focused == Some(participant.local_id) {
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
            if focused == Some(participant.local_id) {
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
        stage = if filmstrip {
            stage.child(
                div()
                    .id("call-filmstrip")
                    .w_full()
                    .min_w_0()
                    .overflow_x_scroll()
                    .child(tiles),
            )
        } else {
            stage.child(tiles)
        };
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
                        .bg(t.surface)
                        .text_color(t.text)
                        .child(self.call_chat(cx)),
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
                if call.share == MediaState::Active {
                    "Freigabe beenden"
                } else {
                    "Bildschirm teilen"
                },
                call.share,
                LocalAction::Intent(if call.share == MediaState::Active {
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
                .rounded_full();
            let mut group = div().relative().flex().items_center().child(control);
            if id != "screen-share" && matches!(state, MediaState::Unavailable | MediaState::Denied)
            {
                group = group.child(
                    div()
                        .absolute()
                        .top(px(0.))
                        .left(px(34.))
                        .size(px(9.))
                        .rounded_full()
                        .bg(if t.dark { t.warning } else { t.warning_bg })
                        .border_1()
                        .border_color(t.stage),
                );
            }
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
                    .w(px(22.))
                    .rounded_full(),
                );
            }
            primary_controls = primary_controls.child(group);
        }
        if call.hand_supported {
            primary_controls = primary_controls.child(
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
                    Availability::when(!call.end_pending, "Eine Anfrage wird bereits bearbeitet.")
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
        }
        if !call.reactions.as_slice().is_empty() {
            let mut reaction_control = div().relative().child(
                self.round_icon_button(
                    "call-reactions",
                    ("smile", "Reaktion senden"),
                    ButtonStyle::Stage,
                    Availability::when(!call.end_pending, "Eine Anfrage wird bereits bearbeitet.")
                        .hinted(),
                    LocalAction::ToggleCallReactions(call.scope),
                    cx,
                )
                .size(px(44.))
                .min_h(px(44.))
                .rounded_full(),
            );
            if self.call_reactions_open == Some(call.scope) {
                let mut menu = div()
                    .id("call-reaction-menu")
                    .absolute()
                    .bottom(px(54.))
                    .left(px(-72.))
                    .w(px(192.))
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
                for (index, emoji) in call.reactions.as_slice().iter().enumerate() {
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
                        .min_w(px(40.))
                        .min_h(px(40.))
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
            primary_controls = primary_controls.child(reaction_control);
        }
        for (id, symbol, label, action) in [
            (
                "call-participants",
                "users",
                "Personen",
                LocalAction::Open(Overlay::CallParticipants),
            ),
            ("call-chat", "chat", "Chat", LocalAction::ToggleCallChat),
        ] {
            primary_controls = primary_controls.child(
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
                .rounded_full(),
            );
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
            .child(div().w(px(56.)).flex_none())
            .child(primary_controls)
            .child(
                self.icon_button(
                    "leave-call",
                    (
                        "phone-down",
                        if call.direct {
                            "Auflegen"
                        } else {
                            "Anruf verlassen"
                        },
                    ),
                    ButtonStyle::StageDanger,
                    Availability::available().hinted(),
                    LocalAction::Intent(UiIntent::Legacy(UserIntent::Leave(call.scope))),
                    cx,
                )
                .w(px(56.))
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
                                            .child(call.title.as_str().to_owned()),
                                    )
                                    .child(
                                        div()
                                            .text_size(self.sp(12.))
                                            .text_color(t.stage_muted)
                                            .child(format!(
                                                "{} · {}",
                                                self.l(status),
                                                self.count(
                                                    super::localization::Count::VisibleParticipants,
                                                    call.participants.as_slice().len() as u64
                                                )
                                            )),
                                    ),
                            )
                            .when(call.participants.as_slice().len() > 1, |header| {
                                let speaker_mode =
                                    self.call_speaker == Some(call.scope) && pinned.is_none();
                                header.child(self.icon_button(
                                    "call-layout",
                                    (
                                        if speaker_mode { "grid" } else { "speaker" },
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
                                    .font_weight(FontWeight::BOLD).child(self.l("REC")))
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
                                ("square", "Vollbild umschalten"),
                                ButtonStyle::Stage,
                                Availability::available().labelled(),
                                LocalAction::ToggleCallFullscreen(call.scope),
                                cx,
                            )),
                    )
                    .when(call.end_pending || call.end_failed, |d| d.child(div().px_5().py_2().text_size(self.sp(12.))
                        .child(self.l(if call.end_pending { "Anruf wird für alle beendet …" } else { "Der Anrufstatus ist unklar. Verlasse den Anruf und prüfe, ob er beendet wurde." }))))
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
                    .child(controls)
                    .children(self.call_device_popover(call.scope, cx)),
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
            .right(px(12.))
            .bottom(px(20.))
            .text_size(self.sp(34.))
            .child(reaction.emoji.as_str().to_owned());
        Some(if fixture {
            element.into_any_element()
        } else {
            self.call_reaction_motion(id.into(), element)
        })
    }
    fn call_outcome(&self, failed: bool, cx: &Context<Self>) -> Div {
        let t = self.theme;
        let call = self
            .snapshot
            .call
            .as_ref()
            .expect("terminal call projection");
        let outcome = if failed {
            div()
                .p_5()
                .rounded(px(12.))
                .bg(t.danger_bg)
                .text_color(t.danger)
                .flex()
                .flex_col()
                .items_center()
                .gap_3()
                .text_center()
                .child(self.scaled_icon("phone-down", 26., t.danger))
                .child(
                    div()
                        .text_size(self.sp(18.))
                        .font_weight(FontWeight::MEDIUM)
                        .child(self.l("Verbindung fehlgeschlagen")),
                )
                .child(
                    div()
                        .text_size(self.sp(12.))
                        .child(self.l("Der Anruf konnte nicht verbunden werden.")),
                )
        } else {
            self.state_note(
                "phone-down",
                "Anruf beendet",
                "Du bist nicht mehr im Anruf.",
            )
        };
        div().size_full().p_3().bg(t.chrome()).child(self.content_card(
            div().text_color(t.text)
                .flex().flex_col()
                .child(div().flex_none().px_6().py_4().border_b_1().border_color(t.border)
                    .font_weight(FontWeight::MEDIUM).child(call.title.as_str().to_owned()))
                .child(self.scroll_area(super::navigation::Region::Call).flex_1().min_h_0().p_6()
                    .child(div().max_w(px(520.)).mx_auto().py_6().flex().flex_col().gap_4()
                        .child(self.reveal(("call-outcome", usize::from(failed)), outcome))
                        .child(self.button("return-conversation", "Zurück zum Gespräch", ButtonStyle::Primary,
                            true, LocalAction::ReturnConversation, cx))
                        .child(self.button("rejoin-call", if call.direct { "Erneut anrufen" } else { "Erneut beitreten" },
                            ButtonStyle::Secondary,
                            Availability::when(self.selected_call().is_some_and(|entry| match entry {
                                super::state::CallEntry::Direct(scope) | super::state::CallEntry::Conference(scope) => scope == call.scope,
                            }), "Ein erneuter Anruf ist für dieses Gespräch derzeit nicht freigegeben."),
                            LocalAction::Call, cx))
                        .when(self.local_feedback_visible(), |d| d.child(self.feedback_view(cx)))))))
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
        let mut body = div().size_full().flex().flex_col().child(
            div()
                .flex_none()
                .px_4()
                .py_4()
                .border_b_1()
                .border_color(t.border)
                .font_weight(FontWeight::MEDIUM)
                .child(self.l("Chat")),
        );
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
    use super::{gallery_columns, stage_speaking_color};

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
