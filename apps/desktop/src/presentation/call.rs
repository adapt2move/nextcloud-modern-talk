use super::controls::Availability;
use super::{ButtonStyle, Desktop, LocalAction, Overlay};
use gpui::{Context, Div, FontWeight, ImageSource, Window, div, img, prelude::*, px};
use talk_core::ui::UserIntent;
use talk_core::ui::presentation::{CallStage, MediaState, RequiredGate, UiIntent};

impl Desktop {
    pub fn set_call_images(
        &mut self,
        scope: Option<talk_core::ui::CallScope>,
        images: Vec<(u128, ImageSource)>,
        cx: &mut Context<Self>,
    ) {
        let admitted = scope.is_some_and(|s| {
            self.snapshot.call.as_ref().is_some_and(|c| {
                c.scope == s && matches!(c.stage, CallStage::Active | CallStage::Connecting)
            })
        });
        let mut changed = false;
        let mut index = 0;
        while index < self.call_images.len() {
            let (old_scope, participant, at, _) = &self.call_images[index];
            let retain = admitted
                && Some(*old_scope) == scope
                && at.elapsed() < std::time::Duration::from_secs(2)
                && self.snapshot.call.as_ref().is_some_and(|c| {
                    c.participants
                        .as_slice()
                        .iter()
                        .any(|p| p.local_id == *participant)
                });
            if retain {
                index += 1;
            } else {
                let (_, _, _, image) = self.call_images.remove(index);
                if let ImageSource::Render(old) = image {
                    cx.drop_image(old, None);
                }
                changed = true;
            }
        }
        if let Some(scope) = scope.filter(|_| admitted) {
            for (participant, image) in images.into_iter().take(16) {
                if !self.snapshot.call.as_ref().is_some_and(|c| {
                    c.participants
                        .as_slice()
                        .iter()
                        .any(|p| p.local_id == participant)
                }) {
                    continue;
                }
                if let Some(index) = self
                    .call_images
                    .iter()
                    .position(|(s, p, _, _)| *s == scope && *p == participant)
                {
                    let (_, _, _, old) = self.call_images.remove(index);
                    if let ImageSource::Render(old) = old {
                        cx.drop_image(old, None);
                    }
                }
                if self.call_images.len() < 16 {
                    self.call_images
                        .push((scope, participant, std::time::Instant::now(), image));
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
        if call.share == MediaState::Active {
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
        let columns = if wide { 2 } else { 1 };
        let mut tiles = div()
            .grid()
            .grid_cols(columns)
            .gap_3()
            .w_full()
            // The viewport scrolls the complete bounded projection. Shrinking
            // this grid would hide later rows outside its reported scroll extent.
            .flex_none()
            .min_h(px(if wide { 400. } else { 240. }));
        for participant in call.participants.as_slice() {
            let image = self
                .call_images
                .iter()
                .find(|(scope, local, _, _)| *scope == call.scope && *local == participant.local_id)
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
                    .child(self.avatar(participant.initials.as_str(), true))
                    .child(
                        div()
                            .text_size(self.sp(13.))
                            .text_color(t.stage_muted)
                            .child(self.l(
                                if participant.self_user && call.camera == MediaState::Off {
                                    "Deine Kamera ist aus"
                                } else {
                                    "Kein Videobild verfügbar"
                                },
                            )),
                    )
            };
            tiles = tiles.child(
                div()
                    .relative()
                    .min_w_0()
                    .min_h(px(if call.participants.as_slice().len() <= 2 {
                        330.
                    } else {
                        210.
                    }))
                    .rounded(px(12.))
                    .bg(t.stage_panel)
                    .flex()
                    .flex_col()
                    .items_center()
                    .justify_center()
                    .gap_4()
                    .p_6()
                    .child(visual)
                    .child(
                        div()
                            .flex_none()
                            .w_full()
                            .min_w_0()
                            .flex()
                            .items_center()
                            .gap_2()
                            .child(self.scaled_icon("user", 14., t.stage_muted))
                            .child(
                                div()
                                    .flex_1()
                                    .min_w_0()
                                    .text_ellipsis()
                                    .text_size(self.sp(12.))
                                    .font_weight(FontWeight::MEDIUM)
                                    .child(format!(
                                        "{}{}",
                                        participant.name.as_str(),
                                        if participant.self_user {
                                            self.l(" · Du")
                                        } else {
                                            ""
                                        }
                                    )),
                            ),
                    ),
            );
        }
        if call.participants.as_slice().is_empty() {
            tiles = tiles.child(
                div()
                    .p_6()
                    .child(self.l("Teilnehmerdaten noch nicht verfügbar.")),
            );
        }
        stage = stage.child(tiles);
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
        let mut controls = div()
            .flex_none()
            .flex()
            .flex_wrap()
            .items_center()
            .justify_center()
            .gap_4()
            .p_4()
            .border_t_1()
            .border_color(t.stage_panel);
        for (id, symbol, label, state, action) in [
            (
                "microphone",
                if call.microphone == MediaState::Active {
                    "mic"
                } else {
                    "mic-off"
                },
                media_label(true, call.microphone),
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
                media_label(false, call.camera),
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
            controls = controls.child(
                div()
                    .flex()
                    .flex_col()
                    .items_center()
                    .gap_1()
                    .child(
                        self.icon_button(
                            id,
                            (symbol, label),
                            ButtonStyle::Stage,
                            Availability::when(
                                matches!(
                                    state,
                                    MediaState::Off | MediaState::Active | MediaState::Requested
                                ),
                                media_reason(state),
                            )
                            .labelled(),
                            action,
                            cx,
                        )
                        .size(px(44.)),
                    )
                    .child(
                        div()
                            .text_size(self.sp(12.))
                            .text_color(t.stage_muted)
                            .child(self.l(label)),
                    ),
            );
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
            controls = controls.child(
                div()
                    .flex()
                    .flex_col()
                    .items_center()
                    .gap_1()
                    .child(
                        self.icon_button(
                            id,
                            (symbol, label),
                            ButtonStyle::Stage,
                            Availability::available().labelled(),
                            action,
                            cx,
                        )
                        .size(px(44.)),
                    )
                    .child(
                        div()
                            .text_size(self.sp(12.))
                            .text_color(t.stage_muted)
                            .child(self.l(label)),
                    ),
            );
        }
        controls = controls.child(
            div()
                .flex()
                .flex_col()
                .items_center()
                .gap_1()
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
                        Availability::available().labelled(),
                        LocalAction::Intent(UiIntent::Legacy(UserIntent::Leave(call.scope))),
                        cx,
                    )
                    .w(px(56.))
                    .h(px(44.)),
                )
                .child(
                    div()
                        .text_size(self.sp(12.))
                        .text_color(t.stage_muted)
                        .child(self.l(if call.direct { "Auflegen" } else { "Verlassen" })),
                ),
        );
        div().size_full().p_3().bg(t.chrome()).child(
            self.content_card(
                div()
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
                            ),
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
                    .child(controls),
            )
            .bg(t.stage),
        )
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
                .child(self.scaled_icon("info", 26., t.danger))
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
                .child(self.message_timeline(false, cx))
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
        MediaState::Requested => "Bitte warte auf die ausstehende Freigabe.",
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
