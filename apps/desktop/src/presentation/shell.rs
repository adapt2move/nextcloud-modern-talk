pub(super) mod chat_header;

use super::controls::Availability;
use super::{ButtonStyle, Desktop, LocalAction, Overlay};
use gpui::{Context, Div, Focusable, FontWeight, Window, div, prelude::*, px};
use talk_core::ui::Decision;
use talk_core::ui::messaging::MessageFeature;
use talk_core::ui::presentation::{
    ContentState, ConversationKind, ConversationView, Presence, UiIntent,
};

impl Desktop {
    pub(super) fn conversation_avatar(&self, row: &ConversationView) -> Div {
        let t = self.theme;
        let mut avatar = self
            .projected_avatar(
                talk_core::ui::content::AssetTarget::ConversationAvatar(row.context),
                36.,
            )
            .unwrap_or_else(|| {
                if row.kind == ConversationKind::Direct {
                    self.person_avatar(row.initials.as_str(), 36.)
                } else {
                    div()
                        .flex_none()
                        .size(px(36.))
                        .rounded(px(12.))
                        .bg(t.selected)
                        .flex()
                        .items_center()
                        .justify_center()
                        .child(self.scaled_icon(
                            match row.kind {
                                ConversationKind::Group => "users",
                                ConversationKind::GuestAccessible => "link",
                                ConversationKind::NoteToSelf => "file",
                                _ => "chat",
                            },
                            20.,
                            t.selected_text,
                        ))
                }
            });
        if row.kind == ConversationKind::Direct && row.presence != Presence::Unknown {
            avatar = avatar.relative().child(
                div()
                    .absolute()
                    .right(px(-1.))
                    .bottom(px(-1.))
                    .size(px(12.))
                    .rounded_full()
                    .border_2()
                    .border_color(if self.snapshot.selected == Some(row.context) {
                        t.selected
                    } else {
                        t.sidebar
                    })
                    .bg(match row.presence {
                        Presence::Available => t.success,
                        Presence::DoNotDisturb => t.danger,
                        Presence::Away => t.warning,
                        _ => t.muted,
                    }),
            );
        }
        avatar
    }
    pub fn shell(&self, window: &Window, cx: &Context<Self>) -> Div {
        let narrow = window.viewport_size().width < px(880.) || self.scale >= 150;
        let sidebar = self.sidebar(narrow, cx);
        let content = div()
            .tab_group()
            .tab_index(1)
            .flex_1()
            .min_w_0()
            .h_full()
            .py_3()
            .pr_3()
            .when(narrow, |d| d.pl_3())
            .flex()
            .flex_col()
            .gap_2()
            .when(narrow, |d| {
                d.child(
                    div()
                        .tab_group()
                        .tab_index(0)
                        .flex()
                        .items_center()
                        .justify_between()
                        .child(self.icon_button(
                            "show-navigation",
                            ("sidebar", "Gespräche"),
                            ButtonStyle::Quiet,
                            Availability::available(),
                            LocalAction::ShowNavigation,
                            cx,
                        ))
                        .child(self.icon_button(
                            "small-settings",
                            ("settings", "Einstellungen"),
                            ButtonStyle::Quiet,
                            Availability::available(),
                            LocalAction::Open(Overlay::Settings),
                            cx,
                        )),
                )
            })
            .child(div().tab_group().tab_index(1).flex_1().min_h_0().child(
                if self.destination == super::SidebarDestination::Inbox {
                    self.inbox(cx)
                } else {
                    self.conversation(window.viewport_size().height < self.sp(160.) + px(210.), cx)
                },
            ));
        let layout = div().size_full().relative().flex();
        let layout = if narrow && self.navigation_open {
            layout.child(
                div()
                    .size_full()
                    .flex()
                    .flex_col()
                    .child(
                        self.button(
                            "hide-navigation",
                            "Zurück zum Gespräch",
                            ButtonStyle::Quiet,
                            true,
                            LocalAction::HideNavigation,
                            cx,
                        )
                        .mx_3()
                        .mt_2(),
                    )
                    .child(
                        self.scroll_area(super::navigation::Region::Rooms)
                            .flex_1()
                            .min_h_0()
                            .child(sidebar.w_full()),
                    ),
            )
        } else if narrow {
            layout.child(content)
        } else {
            layout
                .child(sidebar.tab_group().tab_index(0))
                .child(content)
        };
        layout.when(
            self.snapshot.call.as_ref().is_some_and(|c| {
                c.stage == talk_core::ui::presentation::CallStage::Incoming
                    && Some(c.scope) == self.dismissed_incoming
            }),
            |d| {
                d.child(
                    div()
                        .tab_group()
                        .tab_index(2)
                        .occlude()
                        .absolute()
                        .top_4()
                        .right_4()
                        .child(self.button(
                            "show-incoming",
                            "Eingehender Anruf ansehen",
                            ButtonStyle::Primary,
                            true,
                            LocalAction::ShowIncoming,
                            cx,
                        )),
                )
            },
        )
    }
    fn conversation(&self, constrained: bool, cx: &Context<Self>) -> Div {
        let t = self.theme;
        let Some(row) = self.selected() else {
            return div()
                .relative()
                .size_full()
                .rounded(px(13.))
                .flex()
                .items_center()
                .justify_center()
                .bg(t.surface)
                .flex_col()
                .gap_4()
                .child(self.l("Wähle ein Gespräch."))
                .when(self.local_feedback_visible(), |d| {
                    d.child(self.feedback_view(cx))
                })
                .child(t.inset_card_edge(13.));
        };
        let kind = match row.kind {
            ConversationKind::Direct => "Persönliches Gespräch",
            ConversationKind::Group => "Gruppengespräch",
            ConversationKind::GuestAccessible => "Gespräch mit Gastzugang",
            ConversationKind::NoteToSelf => "Deine Notizen",
            ConversationKind::Unknown => "Unbekannte Gesprächsart",
        };
        let call_availability = Availability::when(
            self.selected_call().is_some(),
            super::state::call_unavailable_reason(row, self.snapshot.content),
        );
        let prepared = matches!(
            row.organization.category,
            talk_core::ui::presentation::ConversationCategory::Conference(_)
        );
        let call_name = if prepared {
            "Meeting vorbereiten"
        } else {
            "Gespräch starten"
        };
        let call_icon = match row.organization.category {
            talk_core::ui::presentation::ConversationCategory::Conference(
                talk_core::ui::presentation::ConferencePreset::Presentation,
            ) => "monitor-up",
            talk_core::ui::presentation::ConversationCategory::Conference(_) => "video",
            _ => "headphones",
        };
        let favorite_name = if self.favorite_value(row) == Some(true) {
            "Aus Favoriten entfernen"
        } else {
            "Zu Favoriten hinzufügen"
        };
        let favorite_availability = Availability::when(
            self.can_favorite(row),
            "Favoriten sind für dieses Gespräch nicht verfügbar.",
        );
        let search_availability = self.message_available(MessageFeature::Search);
        let h = t.chat_header_theme(self.palette);
        let header = div()
            .id("conversation-header")
            .group("conversation-header")
            .bg(h.chat_header_surface())
            .text_color(h.text)
            .flex_none()
            .flex()
            .flex_wrap()
            .items_center()
            .gap_3()
            .px_6()
            .py(px(14.))
            .min_h(px(74.))
            .border_b_1()
            .border_color(h.border)
            .child(self.conversation_avatar(row))
            .child(
                div()
                    .flex_1()
                    .min_w(px(if self.scale >= 150 { 280. } else { 160. }))
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .gap_2()
                            .child(
                                self.control(
                                    "conversation-title",
                                    ButtonStyle::Quiet,
                                    Availability::available().hinted().explained_elsewhere(),
                                    LocalAction::MessageIn(
                                        row.context,
                                        super::messaging::Action::Files,
                                    ),
                                    cx,
                                )
                                .min_w_0()
                                .justify_start()
                                .p_0()
                                .min_h(px(28.))
                                .text_color(h.text)
                                .text_size(self.sp(18.))
                                .font_weight(FontWeight::BOLD)
                                .child(row.title.as_str().to_owned()),
                            )
                            .child(
                                self.icon_button(
                                    "favorite-conversation",
                                    (
                                        if self.favorite_value(row) == Some(true) {
                                            "star-filled"
                                        } else {
                                            "star"
                                        },
                                        favorite_name,
                                    ),
                                    ButtonStyle::Quiet,
                                    favorite_availability.explained_elsewhere(),
                                    LocalAction::Favorite(
                                        row.context,
                                        self.favorite_value(row) != Some(true),
                                    ),
                                    cx,
                                )
                                .flex_none()
                                .opacity(if self.favorite_value(row) == Some(true) {
                                    1.
                                } else {
                                    0.
                                })
                                .group_hover("conversation-header", |s| s.opacity(1.))
                                .focus(|s| s.opacity(1.)),
                            )
                            .child(
                                self.icon_button(
                                    "history-search",
                                    ("search", "Im Gespräch suchen"),
                                    ButtonStyle::Quiet,
                                    search_availability.explained_elsewhere(),
                                    LocalAction::MessageIn(
                                        row.context,
                                        super::messaging::Action::ToggleSearch,
                                    ),
                                    cx,
                                )
                                .opacity(0.)
                                .group_hover("conversation-header", |s| s.opacity(1.))
                                .focus(|s| s.opacity(1.)),
                            ),
                    )
                    .child(
                        self.hint(&format!(
                            "{}{}",
                            self.l(kind),
                            row.participants.map_or(String::new(), |n| format!(
                                " · {}",
                                self.count(super::localization::Count::Participants, u64::from(n))
                            ))
                        ))
                        .text_color(h.sidebar_secondary),
                    ),
            )
            .when(row.organization.classified == Some(true), |d| {
                d.child(
                    div()
                        .flex()
                        .items_center()
                        .gap_1()
                        .text_size(self.sp(12.))
                        .text_color(h.sidebar_secondary)
                        .child(self.scaled_icon("lock", 14., h.sidebar_secondary))
                        .child(self.l("Vertraulich")),
                )
            })
            .when(
                !matches!(
                    row.kind,
                    ConversationKind::Direct | ConversationKind::NoteToSelf
                ),
                |d| {
                    d.child(
                        self.icon_button(
                            "room-members",
                            ("users", "Mitglieder"),
                            ButtonStyle::Quiet,
                            Availability::available().explained_elsewhere(),
                            LocalAction::Open(Overlay::Participants),
                            cx,
                        )
                        .w_auto()
                        .px_2()
                        .when_some(row.participants, |d, count| d.child(count.to_string())),
                    )
                },
            )
            .when(row.kind != ConversationKind::NoteToSelf, |d| {
                d.child(self.icon_button(
                    "call",
                    (call_icon, call_name),
                    ButtonStyle::Quiet,
                    call_availability.explained_elsewhere(),
                    LocalAction::Call,
                    cx,
                ))
            })
            .child(self.icon_button(
                "room-info",
                ("settings", "Gesprächseinstellungen"),
                ButtonStyle::Quiet,
                Availability::available().explained_elsewhere(),
                LocalAction::Open(Overlay::RoomInfo),
                cx,
            ));
        // Header hints protect the entire neighboring title/action/status region.
        // Its measured bounds grow with wrapping/text scale; no toolbar offsets.
        let header = div()
            .relative()
            .flex_none()
            .when(cfg!(target_os = "windows"), |d| d.bg(gpui::rgb(0xffffff)))
            .child(header)
            .when(!self.background_inert, |d| {
                d.when(row.kind != ConversationKind::NoteToSelf, |d| {
                    d.child(self.reason_tip(
                        "call".into(),
                        call_availability.below_control(),
                        Some(call_name),
                        cx,
                    ))
                })
                .child(self.reason_tip(
                    "room-info".into(),
                    Availability::available().below_control(),
                    Some("Gesprächseinstellungen"),
                    cx,
                ))
                .child(self.reason_tip(
                    "conversation-title".into(),
                    Availability::available().below_control(),
                    Some("Geteilte Inhalte"),
                    cx,
                ))
                .child(self.reason_tip(
                    "favorite-conversation".into(),
                    favorite_availability.below_control(),
                    Some(favorite_name),
                    cx,
                ))
                .child(self.reason_tip(
                    "history-search".into(),
                    search_availability.below_control(),
                    Some("Im Gespräch suchen"),
                    cx,
                ))
                .when(
                    !matches!(
                        row.kind,
                        ConversationKind::Direct | ConversationKind::NoteToSelf
                    ),
                    |d| {
                        d.child(self.reason_tip(
                            "room-members".into(),
                            Availability::available().below_control(),
                            Some("Mitglieder"),
                            cx,
                        ))
                    },
                )
            });
        let header = div()
            .flex()
            .flex_col()
            .flex_none()
            .child(header)
            .child(self.conversation_search(cx).bg(t.surface));
        let timeline = self.conversation_timeline(header, constrained, cx);
        let composer = if (self.snapshot.content == ContentState::Denied
            || row.send == Decision::Denied)
            && self
                .account_connection_failure(row.context.account)
                .is_none()
        {
            div()
                .flex_none()
                .px_6()
                .py_4()
                .border_t_1()
                .border_color(t.border)
                .child(self.hint(self.l(
                    "Du kannst in diesem Gespräch nicht schreiben. Dein Entwurf bleibt erhalten.",
                )))
                .when(self.local_feedback_visible(), |d| {
                    d.child(self.feedback_view(cx))
                })
        } else {
            self.composer_view(row, false, cx)
        };
        let panel = div()
            .rounded(px(12.))
            .flex()
            .flex_col()
            .bg(t.surface)
            .child(timeline.tab_group().tab_index(0))
            .child(composer.tab_group().tab_index(1));
        div()
            .relative()
            .size_full()
            .rounded(px(13.))
            .overflow_hidden()
            .border_1()
            .border_color(t.border)
            .child(
                self.scroll_area(super::navigation::Region::Conversation)
                    .size_full()
                    .child(if constrained {
                        panel.w_full()
                    } else {
                        panel.size_full()
                    }),
            )
            .child(t.inset_card_edge(12.))
    }
    pub(super) fn composer_view(
        &self,
        row: &ConversationView,
        compact: bool,
        cx: &Context<Self>,
    ) -> Div {
        let t = self.theme;
        let connection_failure = self.account_connection_failure(row.context.account);
        let ready = super::state::composer_send_decision(&self.snapshot, row) == Decision::Allowed;
        let pending = self.drafts.pending(row.context);
        let emoji_availability = self.emoji_availability(cx).above_composer();
        let options_availability = Availability::available().above_composer();
        let send_availability = Availability::when(
            ready && !pending,
            if self.drafts.uncertain(row.context) {
                "Sendestatus unklar. Vor erneutem Senden ist ein Abgleich erforderlich."
            } else if pending {
                "Bitte warte auf das Ergebnis. Dein Entwurf bleibt erhalten."
            } else {
                "Senden ist derzeit nicht verfügbar. Dein Entwurf bleibt erhalten."
            },
        )
        .above_composer();
        div()
            .track_focus(&self.composer_focus)
            .flex_none()
            .px(if compact { px(12.) } else { px(24.) })
            .pb_4()
            .pt_2()
            .flex()
            .flex_col()
            .gap_2()
            .children(self.composer_context(cx))
            .children(self.composer_more_menu(cx))
            .child(
                div()
                    .min_h(px(96.))
                    .relative()
                    .child(self.field_ring(self.composer.focus_handle(cx), 10.))
                    .rounded(px(10.))
                    .border_1()
                    .border_color(if self.composer.read(cx).rejected {
                        t.danger
                    } else {
                        t.input_border
                    })
                    .on_mouse_down(
                        gpui::MouseButton::Left,
                        cx.listener(|this, _, window, cx| {
                            if this.composer_available() {
                                this.composer.focus_handle(cx).focus(window);
                                window.prevent_default();
                                cx.stop_propagation();
                            }
                        }),
                    )
                    .p_3()
                    .flex()
                    .flex_col()
                    .gap_2()
                    .child(self.composer.clone())
                    .when(
                        self.feedback.is_some_and(|f| {
                            f.context == Some(row.context)
                                || (f.context.is_none() && self.snapshot.call.is_none())
                        }) && self.composer_available(),
                        |d| d.child(self.feedback_view(cx)),
                    )
                    .when(!self.background_inert, |d| {
                        d.child(self.reason_tip(
                            "attach".into(),
                            options_availability,
                            Some("Weitere Optionen"),
                            cx,
                        ))
                        .child(self.reason_tip(
                            "emoji-picker".into(),
                            emoji_availability,
                            Some("Emoji auswählen"),
                            cx,
                        ))
                        .child(self.reason_tip(
                            "send".into(),
                            send_availability,
                            Some("Senden"),
                            cx,
                        ))
                    })
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .justify_between()
                            .gap_3()
                            .child(
                                div()
                                    .flex()
                                    .items_center()
                                    .gap_1()
                                    .child(self.icon_button(
                                        "attach",
                                        ("plus", "Weitere Optionen"),
                                        // The open + menu keeps its trigger visibly pressed.
                                        if self.content_ui.menu {
                                            ButtonStyle::Secondary
                                        } else {
                                            ButtonStyle::Quiet
                                        },
                                        options_availability,
                                        LocalAction::Content(super::content::Action::Menu),
                                        cx,
                                    ))
                                    .child(self.icon_button(
                                        "emoji-picker",
                                        ("smile", "Emoji auswählen"),
                                        ButtonStyle::Quiet,
                                        emoji_availability,
                                        LocalAction::EmojiPicker(row.context),
                                        cx,
                                    )),
                            )
                            .child(div().flex().items_center().gap_3().child(self.icon_button(
                                "send",
                                ("arrow-up", "Senden"),
                                ButtonStyle::Primary,
                                send_availability,
                                LocalAction::Send,
                                cx,
                            ))),
                    ),
            )
            .when_some(connection_failure, |d, failure| {
                d.child(div().text_color(t.danger).text_size(self.sp(12.)).child(
                    self.selectable_text(
                        "composer-connection-error",
                        self.l(super::state::connection_failure_copy(failure).1),
                    ),
                ))
                .child(self.hint(self.l(
                    "Dein Entwurf bleibt erhalten. Nachrichten werden nicht automatisch gesendet.",
                )))
            })
            .when(
                self.snapshot.content == ContentState::Offline && connection_failure.is_none(),
                |d| {
                    d.child(
                        self.hint(self.l("Offline · Entwürfe werden nicht automatisch gesendet.")),
                    )
                },
            )
            .when(self.composer.read(cx).rejected, |d| {
                d.child(
                    div().text_color(t.danger).text_size(self.sp(12.)).child(
                        self.l(self
                            .composer
                            .read(cx)
                            .rejection
                            .unwrap_or("Der Text ist zu lang. Dein Entwurf bleibt erhalten.")),
                    ),
                )
            })
    }
    fn emoji_availability(&self, cx: &Context<Self>) -> Availability {
        let supported = cfg!(target_os = "macos") || self.settings_port.emoji_picker_available();
        let clipboard = !cfg!(target_os = "linux") || self.composer.read(cx).clipboard_allowed;
        let composing = self.input_composing(cx);
        Availability::when(
            supported && clipboard && self.composer_available() && !composing,
            if composing {
                "Beende zuerst die laufende Texteingabe."
            } else if !supported {
                "Die native Emoji-Auswahl ist auf diesem System nicht verfügbar."
            } else if !clipboard {
                "Die Emoji-Auswahl benötigt die für diesen Entwurf gesperrte Zwischenablage."
            } else {
                "Öffne zuerst das Nachrichtenfeld."
            },
        )
        .help(if cfg!(target_os = "linux") && supported {
            "Wähle ein Emoji und füge es danach mit Strg+V in die Nachricht ein."
        } else {
            ""
        })
    }
    pub(super) fn show_composer_emoji(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if !self.composer_available()
            || self.input_composing(cx)
            || !self.composer.read(cx).interactive
            || self.emoji_picker_pending.is_some()
            || (cfg!(target_os = "linux") && !self.composer.read(cx).clipboard_allowed)
        {
            return;
        }
        let Some(context) = self.snapshot.selected else {
            return;
        };
        if !cfg!(target_os = "macos") && !self.settings_port.emoji_picker_available() {
            return;
        }
        let Some(generation) = self.emoji_picker_generation.checked_add(1) else {
            return;
        };
        self.emoji_picker_generation = generation;
        // Focus alone leaves the exact selection, marked text, history and draft
        // untouched. Wait one frame for the native text handler to own focus.
        self.composer.focus_handle(cx).focus(window);
        self.emoji_picker_pending = Some((context, generation));
        let view = cx.weak_entity();
        window.on_next_frame(move |window, cx| {
            let _ = view.update(cx, |this, cx| {
                if this.emoji_picker_pending != Some((context, generation)) {
                    return;
                }
                this.emoji_picker_pending = None;
                if this.snapshot.selected != Some(context)
                    || this.projection_blocked
                    || !window.is_window_active()
                    || !this.composer_available()
                    || this.snapshot.content == ContentState::Denied
                    || this.selected().is_none_or(|row| row.send == Decision::Denied)
                    || this.input_composing(cx)
                    || !this.composer.read(cx).interactive
                    || (cfg!(target_os = "linux") && !this.composer.read(cx).clipboard_allowed)
                    || !this.composer.focus_handle(cx).is_focused(window)
                {
                    return;
                }
                if cfg!(target_os = "macos") {
                    window.show_character_palette();
                } else {
                    this.emoji_picker_context = None;
                    match this.settings_port.show_emoji_picker() {
                        Ok(()) if cfg!(target_os = "linux") => {
                            this.emoji_picker_context = Some(context);
                            this.feedback = Some(super::feedback::Feedback::info(
                                "Wähle ein Emoji und füge es danach mit Strg+V in die Nachricht ein.",
                            ).in_conversation(context));
                        }
                        Ok(()) => this.emoji_picker_context = Some(context),
                        Err(error) => {
                            this.feedback = Some(super::feedback::Feedback::error(error.label())
                                .in_conversation(context));
                        }
                    }
                    cx.notify();
                }
            });
        });
    }
    pub(super) fn retire_emoji_picker(&mut self) {
        self.emoji_picker_pending = None;
        self.emoji_picker_context = None;
        self.settings_port.cancel_emoji_picker();
    }
    pub(super) fn incoming(&self, cx: &Context<Self>) -> Div {
        let call = self.snapshot.call.as_ref().expect("incoming projection");
        let t = self.theme;
        let account = self
            .snapshot
            .accounts
            .as_slice()
            .iter()
            .find(|a| a.scope == call.scope.account)
            .map_or("Konto unbekannt", |a| a.label.as_str());
        div().absolute().inset_0().occlude().bg(t.scrim).flex().items_center().justify_center().p_5()
            .child(self.scroll_area(super::navigation::Region::Call).max_h_full().w_full().max_w(px(420.)).rounded(px(16.)).bg(t.surface).border_1().border_color(t.border).shadow_lg().p_6()
                .child(div().flex().flex_col().items_center().gap_4().text_center()
                    .child(div().flex().items_center().gap_2().child(self.scaled_icon("phone",16.,t.muted)).child(self.hint(self.l("Eingehender Direktanruf"))))
                    .child(self.avatar(call.participants.as_slice().iter().find(|p|!p.self_user).map_or("?",|p|p.initials.as_str()),true))
                    .child(self.title(call.title.as_str()))
                    .child(self.hint(&self.lf("Persönliches Gespräch · {account}",&[("account",account)])))
                    .child(self.hint(self.l("Bis zur Annahme bleiben Mikrofon und Kamera aus.\nBeim Annehmen: Mikrofon an, Kamera aus — nach Freigabe.")))
                    .child(div().flex().justify_center().gap_8().py_3()
                        .child(div().flex().flex_col().items_center().gap_2().child(self.round_icon_button("decline",("phone-down","Ablehnen"),ButtonStyle::Danger,Availability::available().labelled(),LocalAction::Intent(UiIntent::DeclineDirect(call.scope)),cx)).child(self.l("Ablehnen")))
                        .child(div().flex().flex_col().items_center().gap_2().child(self.round_icon_button("accept",("phone","Annehmen"),ButtonStyle::Primary,Availability::when(call.accept==Decision::Allowed,"Der Anruf kann momentan nicht angenommen werden.").labelled().below_control(),LocalAction::Accept,cx)).child(self.l("Annehmen"))))
                    .child(self.button("dismiss","Später ansehen",ButtonStyle::Quiet,true,LocalAction::DismissIncoming,cx))))
    }
}
