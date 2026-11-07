pub(super) mod chat_header;

use super::controls::{Availability, CONTENT_RADIUS, Skeleton};
use super::{ButtonStyle, Desktop, LocalAction, Overlay};
use gpui::{Context, Div, Focusable, FontWeight, Window, div, prelude::*, px};
use talk_core::ui::Decision;
use talk_core::ui::messaging::MessageFeature;
use talk_core::ui::presentation::{
    ContentState, ConversationKind, ConversationView, Presence, UiIntent,
};

impl Desktop {
    pub(super) fn narrow_navigation(&self, window: &Window) -> bool {
        window.viewport_size().width < px(880.) || self.scale >= 150
    }

    pub(super) fn sidebar_visible(&self, window: &Window) -> bool {
        if self.narrow_navigation(window) {
            self.navigation_open
        } else {
            !self.sidebar_hidden
        }
    }

    pub(super) fn conversation_avatar(&self, row: &ConversationView) -> Div {
        let t = self.theme;
        let avatar = self
            .projected_avatar(
                talk_core::ui::content::AssetTarget::ConversationAvatar(row.context),
                36.,
            )
            .unwrap_or_else(|| {
                if row.kind == ConversationKind::Direct {
                    self.person_avatar(row.initials.as_str(), 36.)
                } else {
                    self.avatar_shape(36.)
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
        if row.kind == ConversationKind::Direct {
            self.avatar_with_presence(avatar, row.presence, 36., t.surface)
        } else {
            avatar
        }
    }

    // Wrap the avatar so projected images' clipping cannot hide the status dot.
    pub(super) fn avatar_with_presence(
        &self,
        avatar: Div,
        presence: Presence,
        avatar_size: f32,
        background: gpui::Hsla,
    ) -> Div {
        if presence == Presence::Unknown {
            return avatar;
        }
        // Small sidebar avatars need more clearance for their initials. Keep
        // most of the indicator outside while larger avatars retain their edge.
        let (right, bottom) = if avatar_size <= 24. {
            (9., 8.)
        } else {
            (7., 7.)
        };
        div().flex_none().relative().child(avatar).child(
            self.presence_indicator(self.theme.presence_dot(presence), background)
                .absolute()
                .right(px(-right))
                .bottom(px(-bottom)),
        )
    }
    pub fn shell(&self, window: &Window, cx: &Context<Self>) -> Div {
        let narrow = self.narrow_navigation(window);
        let sidebar_visible = self.sidebar_visible(window);
        let sidebar = sidebar_visible.then(|| self.sidebar(narrow, cx));
        let content = (!narrow || !sidebar_visible).then(|| {
            div()
                .tab_group()
                .tab_index(1)
                .flex_1()
                .min_w_0()
                .h_full()
                .py_3()
                .pr_3()
                .when(!sidebar_visible || narrow, |d| d.pl_3())
                .flex()
                .flex_col()
                .gap_2()
                .when(
                    !sidebar_visible
                        && self.docked_call()
                        && !matches!(self.overlay, Some(Overlay::Settings | Overlay::Accounts)),
                    |content| content.child(self.call_dock(cx)),
                )
                .child(div().tab_group().tab_index(1).flex_1().min_h_0().child(
                    match self.destination {
                        super::SidebarDestination::Inbox => self.inbox(cx),
                        super::SidebarDestination::Threads => self.threads_page(cx),
                        _ => self.conversation(
                            window.viewport_size().height < self.sp(160.) + px(210.),
                            cx,
                        ),
                    },
                ))
        });
        let layout = div().size_full().relative().flex();
        let layout = if let Some(sidebar) = sidebar {
            if narrow {
                layout.child(
                    div().size_full().flex().flex_col().child(
                        self.scroll_area(super::navigation::Region::Rooms)
                            .flex_1()
                            .min_h_0()
                            .child(sidebar.w_full()),
                    ),
                )
            } else {
                layout
                    .child(sidebar.tab_group().tab_index(0))
                    .children(content)
            }
        } else {
            layout.children(content)
        };
        let layout = layout.when(self.thread_is_docked(window), |d| {
            d.child(self.thread_pane(window, cx))
        });
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
            let state = super::collection::CollectionState::new(
                self.snapshot.conversation_list.content,
                !self.snapshot.conversations.as_slice().is_empty(),
            );
            if state.initial_loading() {
                return self.content_card(
                    div()
                        .flex()
                        .flex_col()
                        .child(self.collection_skeleton(Skeleton::Messages, "shell-messages")),
                );
            }
            return self.content_card(
                self.empty_state(
                    self.scaled_icon("chat", 24., t.muted),
                    self.l("Wähle ein Gespräch.").to_owned(),
                    "",
                )
                .when(self.local_feedback_visible(), |d| {
                    d.child(self.feedback_view(cx))
                }),
            );
        };
        let joining = self
            .snapshot
            .discover_panel
            .as_ref()
            .is_some_and(|panel| panel.joining == Some(row.context) && panel.state.pending());
        let kind = if joining {
            "Beitritt läuft …"
        } else {
            match row.kind {
                ConversationKind::Direct => "Persönliches Gespräch",
                ConversationKind::Group => "Gruppengespräch",
                ConversationKind::GuestAccessible => "Gespräch mit Gastzugang",
                ConversationKind::NoteToSelf => "Deine Notizen",
                ConversationKind::Unknown => "Unbekannte Gesprächsart",
            }
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
        let title_availability = Availability::when(!joining, "Der Beitritt wird überprüft.");
        let search_availability = self.message_available(MessageFeature::Search);
        let h = chat_header::theme(t, self.palette);
        let presence = if row.kind == ConversationKind::Direct {
            match row.presence {
                Presence::Available => Some(self.l("Verfügbar")),
                Presence::Away => Some(self.l("Abwesend")),
                Presence::DoNotDisturb => Some(self.l("Nicht stören")),
                Presence::Offline => Some(self.l("Offline")),
                Presence::Unknown => None,
            }
        } else {
            None
        };
        let custom_status = [
            row.status_icon.as_ref().map(|v| v.as_str()),
            row.status_message.as_ref().map(|v| v.as_str()),
        ]
        .into_iter()
        .flatten()
        .filter(|value| !value.trim().is_empty())
        .collect::<Vec<_>>()
        .join(" ");
        let status_line = match (presence, custom_status.is_empty()) {
            (Some(presence), false) => format!("{presence} · {custom_status}"),
            (Some(presence), true) => presence.to_owned(),
            (None, false) if row.kind == ConversationKind::Direct => custom_status,
            _ => self.l(kind).to_owned(),
        };
        let header = div()
            .id("conversation-header")
            .group("conversation-header")
            .relative()
            .rounded_t(px(CONTENT_RADIUS))
            .bg(if h.reduced_transparency { h.surface } else { gpui::transparent_black() })
            .children((!h.reduced_transparency).then(|| {
                super::controls::glass_corners(h.surface, h.dark, gpui::Corners {
                    top_left: px(CONTENT_RADIUS),
                    top_right: px(CONTENT_RADIUS),
                    bottom_left: px(0.),
                    bottom_right: px(0.),
                })
            }))
            .child(super::controls::inner_highlight(h, CONTENT_RADIUS))
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
                                self.style_panel_title(self.control(
                                    "conversation-title",
                                    ButtonStyle::Quiet,
                                    title_availability.hinted().explained_elsewhere(),
                                    LocalAction::MessageIn(
                                        row.context,
                                        super::messaging::Action::Files,
                                    ),
                                    cx,
                                ))
                                .min_w_0()
                                .justify_start()
                                .p_0()
                                .min_h(px(28.))
                                .text_color(h.text)
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
                            )
                            .when(self.can_start_thread(), |title| {
                                title.child(
                                    self.icon_button(
                                        "new-thread",
                                        ("chat", "Neuen Thread beginnen"),
                                        ButtonStyle::Quiet,
                                        Availability::when(
                                            self.message_ui.editing.is_none()
                                                && !self.has_staged_attachments()
                                                && !self.drafts.pending(row.context),
                                            "Schließe zuerst die aktuelle Bearbeitung oder den Dateientwurf.",
                                        ),
                                        LocalAction::NewThread(row.context),
                                        cx,
                                    )
                                    .opacity(0.)
                                    .group_hover("conversation-header", |s| s.opacity(1.))
                                    .focus(|s| s.opacity(1.)),
                                )
                            }),
                    )
                    .child(
                        self.hint(&status_line)
                            .min_w_0()
                            .text_ellipsis()
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
                !joining
                    && !matches!(
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
            .when(!joining && row.kind != ConversationKind::NoteToSelf, |d| {
                d.child(if row.call_active == Some(true) {
                    self.control(
                        "call",
                        ButtonStyle::Quiet,
                        call_availability.hinted(),
                        LocalAction::Call,
                        cx,
                    )
                    .relative()
                    .size(px(34.))
                    .min_h(px(34.))
                    .p_0()
                    .child(self.scaled_icon("headphones", 18., t.success))
                    .child(
                        div()
                            .absolute()
                            .top(px(3.))
                            .right(px(3.))
                            .child(self.call_live_dot()),
                    )
                } else {
                    self.icon_button(
                        "call",
                        (call_icon, call_name),
                        ButtonStyle::Quiet,
                        call_availability.explained_elsewhere(),
                        LocalAction::Call,
                        cx,
                    )
                })
            })
            .when(row.kind == ConversationKind::Direct, |d| {
                d.child(self.icon_button(
                    "call-video",
                    ("video", "Mit Video anrufen"),
                    ButtonStyle::Quiet,
                    call_availability.explained_elsewhere(),
                    LocalAction::StartDirectWithVideo,
                    cx,
                ))
            })
            .when(!joining && row.notification_muted.is_some(), |d| {
                d.child(self.icon_button(
                    "room-notification-settings",
                    (
                        if row.notification_muted == Some(true) {
                            "bell-off"
                        } else {
                            "bell"
                        },
                        "Gesprächsbenachrichtigungen",
                    ),
                    ButtonStyle::Quiet,
                    Availability::available().explained_elsewhere(),
                    LocalAction::OpenRoomNotifications,
                    cx,
                ))
            })
            .when(!joining, |d| {
                d.child(self.icon_button(
                    "room-info",
                    ("settings", "Gesprächseinstellungen"),
                    ButtonStyle::Quiet,
                    Availability::available().explained_elsewhere(),
                    LocalAction::Open(Overlay::RoomInfo),
                    cx,
                ))
            });
        // Header hints protect the entire neighboring title/action/status region.
        // Its measured bounds grow with wrapping/text scale; no toolbar offsets.
        let header = div()
            .relative()
            .flex_none()
            .when(cfg!(target_os = "windows"), |d| d.bg(gpui::rgb(0xffffff)))
            .child(header)
            .when(!self.background_inert, |d| {
                d.when(!joining && row.kind != ConversationKind::NoteToSelf, |d| {
                    d.child(self.reason_tip(
                        "call".into(),
                        call_availability.below_control(),
                        Some(call_name),
                        cx,
                    ))
                })
                .when(row.kind == ConversationKind::Direct, |d| {
                    d.child(self.reason_tip(
                        "call-video".into(),
                        call_availability.below_control(),
                        Some("Mit Video anrufen"),
                        cx,
                    ))
                })
                .when(!joining && row.notification_muted.is_some(), |d| {
                    d.child(self.reason_tip(
                        "room-notification-settings".into(),
                        Availability::available().below_control(),
                        Some("Gesprächsbenachrichtigungen"),
                        cx,
                    ))
                })
                .when(!joining, |d| {
                    d.child(self.reason_tip(
                        "room-info".into(),
                        Availability::available().below_control(),
                        Some("Gesprächseinstellungen"),
                        cx,
                    ))
                })
                .child(self.reason_tip(
                    "conversation-title".into(),
                    title_availability.below_control(),
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
                    !joining
                        && !matches!(
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
            .children(
                self.account_connection_failure(row.context.account)
                    .and_then(|failure| self.connection_failure_footer(row.context, failure, cx))
                    .map(|banner| div().px_4().py_1().child(banner)),
            )
            .children(self.pin_banner(row.context, cx))
            .child(self.conversation_search(cx).bg(chat_header::surface(h)));
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
            .flex()
            .flex_col()
            .child(timeline.tab_group().tab_index(0))
            .child(composer.tab_group().tab_index(1));
        let drop_context = row.context;
        let drop_enabled = self.composer_available()
            && self.overlay.is_none()
            && !self.drafts.target(row.context).new_thread
            && self.drafts.target(row.context).private_reply.is_none()
            && !self.background_inert
            && self.snapshot.content == ContentState::Ready
            && row.send == Decision::Allowed
            && self.message_feature_enabled(MessageFeature::Files);
        let drop_hint = (drop_enabled && cx.has_active_drag()).then(|| {
            div()
                .absolute()
                .inset_2()
                .rounded(px(CONTENT_RADIUS))
                .border_2()
                .border_color(t.accent)
                .bg(t.surface.opacity(0.94))
                .flex()
                .items_center()
                .justify_center()
                .opacity(0.)
                .group_drag_over::<gpui::ExternalPaths>("chat-file-drop", |style| style.opacity(1.))
                .child(
                    self.reveal(
                        "chat-file-drop-reveal",
                        div()
                            .flex()
                            .flex_col()
                            .items_center()
                            .gap_2()
                            .child(self.scaled_icon("paperclip", 32., t.accent))
                            .child(
                                div()
                                    .font_weight(FontWeight::SEMIBOLD)
                                    .text_color(t.text)
                                    .child(self.l("Dateien hier ablegen")),
                            ),
                    ),
                )
        });
        self.content_card(
            div().child(
                self.scroll_area(super::navigation::Region::Conversation)
                    .size_full()
                    .child(if constrained {
                        panel.w_full()
                    } else {
                        panel.size_full()
                    }),
            ),
        )
        .group("chat-file-drop")
        .children(drop_hint)
        .when(drop_enabled, |card| {
            card.can_drop(|value, _, _| value.is::<gpui::ExternalPaths>())
                .drag_over::<gpui::ExternalPaths>(|style, _, _, _| style)
                .on_drop(
                    cx.listener(move |this, paths: &gpui::ExternalPaths, window, cx| {
                        this.stage_dropped_files(drop_context, paths, window, cx);
                    }),
                )
        })
    }
    pub(super) fn composer_view(
        &self,
        row: &ConversationView,
        compact: bool,
        cx: &Context<Self>,
    ) -> Div {
        self.shared_composer(row, compact, false, cx)
    }
    pub(super) fn thread_composer_view(&self, cx: &Context<Self>) -> Div {
        self.selected()
            .map_or_else(div, |row| self.shared_composer(row, false, true, cx))
    }
    fn shared_composer(
        &self,
        row: &ConversationView,
        compact: bool,
        thread: bool,
        cx: &Context<Self>,
    ) -> Div {
        let t = self.theme;
        if let Some(voice) = self
            .content_ui
            .voice
            .filter(|voice| !thread && voice.context == row.context)
        {
            return self.voice_composer(voice, compact, cx);
        }
        let input = if thread {
            &self.thread_composer
        } else {
            &self.composer
        };
        let attach_id = if thread { "thread-attach" } else { "attach" };
        let emoji_id = if thread {
            "thread-emoji-picker"
        } else {
            "emoji-picker"
        };
        let send_id = if thread { "thread-send" } else { "send" };
        let ready = super::state::composer_send_decision(&self.snapshot, row) == Decision::Allowed;
        let pending = if thread {
            self.thread_drafts.pending(row.context) || self.thread_edit_pending()
        } else {
            self.drafts.pending(row.context)
        };
        let uncertain = if thread {
            self.thread_drafts.uncertain(row.context)
        } else {
            self.drafts.uncertain(row.context)
        };
        let attached = !thread && self.has_staged_attachments();
        let unloaded = !thread && self.message_ui.composer_load_failed == Some(row.context);
        let emoji_availability = self.emoji_availability_for(thread, cx).above_composer();
        let options_availability = Availability::when(
            self.composer_available(),
            "Öffne zuerst das Nachrichtenfeld.",
        )
        .above_composer();
        let send_availability = if pending && !uncertain {
            Availability::pending()
        } else {
            Availability::when(
                ready
                    && !pending
                    && !unloaded
                    && (!attached || self.can_send_attachments(cx))
                    && (thread
                        || !self.drafts.target(row.context).new_thread
                        || self.can_start_thread())
                    && if thread {
                        self.thread_send_available(cx)
                    } else {
                        input.read(cx).submitted_value().is_some()
                    },
                if attached && !self.can_send_attachments(cx) {
                    "Diese Dateioption ist derzeit nicht verfügbar."
                } else if uncertain {
                    "Sendestatus unklar. Vor erneutem Senden ist ein Abgleich erforderlich."
                } else if !attached && input.read(cx).content.trim().is_empty() {
                    "Nachricht eingeben."
                } else {
                    "Senden ist derzeit nicht verfügbar. Dein Entwurf bleibt erhalten."
                },
            )
        }
        .above_composer();
        let send_availability = if !thread
            && self.snapshot.content == ContentState::Offline
            && self
                .snapshot
                .message_panel
                .as_ref()
                .is_some_and(|panel| panel.context == row.context && panel.queue_available)
        {
            send_availability
                .help("Die Nachricht wird gesendet, sobald die Verbindung wieder da ist.")
        } else {
            send_availability
        };
        let show_voice = !thread
            && input.read(cx).content.is_empty()
            && !attached
            && !self.drafts.target(row.context).new_thread
            && self.message_feature_enabled(MessageFeature::Voice);
        div()
            .when(!thread, |d| d.track_focus(&self.composer_focus))
            .when(thread, |d| d.track_focus(&self.thread_composer_focus))
            .flex_none()
            .px(if compact { px(12.) } else { px(24.) })
            .pb_4()
            .pt_2()
            .flex()
            .flex_col()
            .gap_2()
            .when(!thread, |d| d.child(self.typing_slot(row.context)))
            .children(if thread==(self.overlay==Some(Overlay::Thread)) {self.action_hint_view(cx)} else {None})
            .child(
                self.composer_frame()
                    .child(self.field_ring(input.focus_handle(cx), 12.))
                    .border_color(if input.read(cx).rejected {
                        t.danger
                    } else {
                        t.input_border
                    })
                    .on_mouse_down(
                        gpui::MouseButton::Left,
                        cx.listener(move |this, _, window, cx| {
                            if this.composer_available() {
                                if thread {
                                    this.thread_composer.focus_handle(cx).focus(window);
                                } else {
                                    this.composer.focus_handle(cx).focus(window);
                                }
                                window.prevent_default();
                                cx.stop_propagation();
                            }
                        }),
                    )
                    .children(if thread { self.thread_composer_context(cx) } else { self.composer_context(cx) })
                    .when(!unloaded, |d| d.child(input.clone()))
                    .when(unloaded, |d| {
                        d.child(self.hint(self.l(crate::presentation::input::RICH_REJECTION)))
                    })
                    .when(
                        self.composer_feedback_visible(row.context, thread)
                            && self.feedback.is_some_and(|f| {
                                f.context == Some(row.context)
                                    || (f.context.is_none() && self.snapshot.call.is_none())
                            })
                            && self.composer_available(),
                        |d| d.child(self.feedback_view_on(thread.then_some(Overlay::Thread), cx)),
                    )
                    .when(!self.background_inert, |d| {
                        d.child(self.reason_tip(
                            attach_id.into(),
                            options_availability,
                            Some("Weitere Optionen"),
                            cx,
                        ))
                        .child(self.reason_tip(
                            emoji_id.into(),
                            emoji_availability,
                            Some("Emoji auswählen"),
                            cx,
                        ))
                        .child(self.reason_tip(
                            send_id.into(),
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
                                    .child(
                                        self.icon_button(
                                            attach_id,
                                            ("plus", "Weitere Optionen"),
                                            // The open + menu keeps its trigger visibly pressed.
                                            if self.content_ui.menu {
                                                ButtonStyle::Secondary
                                            } else {
                                                ButtonStyle::Quiet
                                            },
                                            options_availability,
                                            LocalAction::Content(super::content::Action::Menu(thread)),
                                            cx,
                                        )
                                        .child(self.composer_menu_anchor(row.context, thread)),
                                    )
                                    .child(self.icon_button(
                                        emoji_id,
                                        ("smile", "Emoji auswählen"),
                                        ButtonStyle::Quiet,
                                        emoji_availability,
                                        if thread {
                                            LocalAction::ThreadEmojiPicker(row.context)
                                        } else {
                                            LocalAction::EmojiPicker(row.context)
                                        },
                                        cx,
                                    )),
                            )
                            .child(
                                div()
                                    .flex()
                                    .items_center()
                                    .gap_3()
                                    .when(
                                        show_voice,
                                        |d| {
                                            d.child(self.icon_button(
                                                if thread { "thread-voice-start" } else { "voice-start" },
                                                ("mic", "Sprachnachricht aufnehmen"),
                                                ButtonStyle::Primary,
                                                if thread { Availability::when(false, "Anhänge in Threads sind noch nicht verfügbar.") } else { ready.into() },
                                                LocalAction::Content(
                                                    super::content::Action::VoiceStart(row.context),
                                                ),
                                                cx,
                                            ))
                                        },
                                    )
                                    .when(!thread && !show_voice && !self.drafts.target(row.context).new_thread && self.drafts.target(row.context).private_reply.is_none() && self.message_feature_enabled(MessageFeature::Schedules), |d| {
                                        d.child(self.icon_button(
                                            "schedule-send", ("chevron-down", "Später senden"),
                                            ButtonStyle::Quiet,
                                            Availability::when(send_availability.is_enabled() && !attached && self.message_ui.editing.is_none(),
                                                "Zum Planen eine Nachricht ohne Dateien verfassen."),
                                            LocalAction::MessageIn(row.context, super::messaging::Action::Schedule), cx,
                                        ))
                                    })
                                    .when(!show_voice, |d| d.child(self.icon_button(
                                        send_id,
                                        ("arrow-up", "Senden"),
                                        ButtonStyle::Primary,
                                        send_availability,
                                        if thread {
                                            LocalAction::SendThread
                                        } else {
                                            LocalAction::Send
                                        },
                                        cx,
                                    ))),
                            ),
                    ),
            )
            .when_some(self.content_ui.voice_error, |d, error| {
                d.child(self.hint(self.l(error)))
            })
            .when(self.draft_storage_unavailable == Some(row.context), |d| {
                d.child(self.hint(self.l("Dieser Entwurf bleibt nur für diese Sitzung erhalten.")))
            })
            .when(input.read(cx).rejected, |d| {
                d.child(
                    div().text_color(t.danger).text_size(self.sp(12.)).child(
                        self.l(input
                            .read(cx)
                            .rejection
                            .unwrap_or("Der Text ist zu lang. Dein Entwurf bleibt erhalten.")),
                    ),
                )
            })
    }
    fn emoji_availability_for(&self, thread: bool, cx: &Context<Self>) -> Availability {
        let supported = cfg!(target_os = "macos") || self.settings_port.emoji_picker_available();
        let composing = self.input_composing(cx);
        Availability::when(
            supported
                && self.composer_available()
                && !composing
                && (!thread || self.overlay == Some(super::Overlay::Thread)),
            if composing {
                "Beende zuerst die laufende Texteingabe."
            } else if !supported {
                "Die native Emoji-Auswahl ist auf diesem System nicht verfügbar."
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
    pub(super) fn show_composer_emoji(
        &mut self,
        thread: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let editor = if thread {
            &self.thread_composer
        } else {
            &self.composer
        };
        if !self.composer_available()
            || self.input_composing(cx)
            || !editor.read(cx).interactive
            || thread && self.overlay != Some(super::Overlay::Thread)
            || self.emoji_picker_pending.is_some()
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
        editor.focus_handle(cx).focus(window);
        self.emoji_picker_pending = Some((context, generation));
        let view = cx.weak_entity();
        window.on_next_frame(move |window, cx| {
            let _ = view.update(cx, |this, cx| {
                if this.emoji_picker_pending != Some((context, generation)) {
                    return;
                }
                this.emoji_picker_pending = None;
                let editor = if thread { &this.thread_composer } else { &this.composer };
                if this.snapshot.selected != Some(context)
                    || this.projection_blocked
                    || !window.is_window_active()
                    || !this.composer_available()
                    || this.snapshot.content == ContentState::Denied
                    || this.selected().is_none_or(|row| row.send == Decision::Denied)
                    || this.input_composing(cx)
                    || thread && this.overlay != Some(super::Overlay::Thread)
                    || !editor.read(cx).interactive
                    || !editor.focus_handle(cx).is_focused(window)
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
        self.dialog_scrim().items_center().justify_center().p_5()
            .child(self.style_dialog_shell(self.scroll_area(super::navigation::Region::Call)).max_h_full().w_full().max_w(px(360.)).p_6()
                .child(div().flex().flex_col().items_center().gap_4().text_center()
                    .child(div().flex().items_center().gap_2().child(self.scaled_icon("phone",16.,t.muted)).child(self.hint(self.l("Eingehender Direktanruf"))))
                    .child(self.incoming_call_avatar(self.avatar(call.participants.as_slice().iter().find(|p|!p.self_user).map_or("?",|p|p.initials.as_str()),true)))
                    .child(div().text_size(self.sp(24.)).font_weight(FontWeight::MEDIUM).child(self.lf("{name} ruft an", &[("name", call.title.as_str())])))
                    .child(self.hint(&self.lf("Persönliches Gespräch · {account}",&[("account",account)])))
                    .child(self.hint(self.l("Mikrofon und Kamera bleiben bis zur Annahme aus.")))
                    .child(div().flex().justify_center().gap_3().py_3()
                        .child(div().flex().flex_col().items_center().gap_2().child(self.round_icon_button("decline",("phone-down","Ablehnen"),ButtonStyle::StageDanger,Availability::available().labelled(),LocalAction::Intent(UiIntent::DeclineDirect(call.scope)),cx).size(px(56.)).min_h(px(56.))).child(self.l("Ablehnen")))
                        .child(div().flex().flex_col().items_center().gap_2().child(self.round_icon_button("accept",("phone","Annehmen"),ButtonStyle::Primary,Availability::when(call.accept==Decision::Allowed,"Der Anruf kann momentan nicht angenommen werden.").labelled().below_control(),LocalAction::Accept,cx).size(px(56.)).min_h(px(56.))).child(self.l("Annehmen")))
                        .child(div().flex().flex_col().items_center().justify_center().gap_2().child(self.round_icon_button("accept-video", ("video", "Mit Video annehmen"), ButtonStyle::Primary, Availability::when(call.accept==Decision::Allowed,"Der Anruf kann momentan nicht angenommen werden.").labelled().below_control(), LocalAction::AcceptCallWithVideo, cx).size(px(56.)).min_h(px(56.))).child(self.l("Mit Video annehmen"))))
                    .child(self.button("dismiss","Später ansehen",ButtonStyle::Quiet,true,LocalAction::DismissIncoming,cx))))
    }
}
