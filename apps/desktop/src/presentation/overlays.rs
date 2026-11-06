use super::collection::CollectionState;
use super::controls::{Availability, scoped_id};
use super::{ButtonStyle, Desktop, LocalAction, Overlay, SearchKind};
use crate::presentation::motion::DialogSurface;
use gpui::{Context, Div, FontWeight, Window, div, prelude::*, px};
use talk_core::ui::presentation::{ContentState, ConversationKind, ParticipantView, Presence};

#[derive(Clone)]
struct ThreadResize;
impl gpui::Render for ThreadResize {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        div().size(px(1.)).opacity(0.)
    }
}

impl Desktop {
    pub(super) fn thread_is_docked(&self, window: &Window) -> bool {
        self.overlay == Some(Overlay::Thread)
            && self.snapshot.selected.is_some()
            && self.prejoin.is_none()
            && !self.onboarding_open
            && !self.snapshot.call.as_ref().is_some_and(|call| {
                call.stage != super::CallStage::Incoming
                    && !(matches!(
                        call.stage,
                        super::CallStage::Ended | super::CallStage::Failed
                    ) && self.dismissed_terminal_call == Some(call.scope))
            })
            && window.viewport_size().width >= px(1050.)
    }

    pub(super) fn thread_pane(&self, window: &Window, cx: &Context<Self>) -> Div {
        let t = self.theme;
        let available = (f32::from(window.viewport_size().width) - 600.).max(288.);
        let width = self.thread_width.min(available);
        let resize_focus = self
            .control_focus
            .get("thread-resize-handle".into(), cx)
            .tab_stop(true);
        let keyboard_focus = resize_focus.clone();
        div()
            .relative()
            .flex_none()
            .w(px(width))
            .h_full()
            .py_3()
            .pr_3()
            .child(
                self.dialog_panel(
                    DialogSurface::Overlay(Overlay::Thread),
                    div()
                        .size_full()
                        .min_h_0()
                        .flex()
                        .flex_col()
                        .rounded(px(13.))
                        .border_1()
                        .border_color(t.border)
                        .bg(t.surface)
                        .track_focus(&self.modal_focus)
                        .tab_group()
                        .child(
                            div()
                                .flex_none()
                                .flex()
                                .items_center()
                                .gap_1()
                                .px_4()
                                .py_3()
                                .border_b_1()
                                .border_color(t.border)
                                .child(self.thread_heading(cx))
                                .child(
                                    self.icon_button(
                                        "close-overlay",
                                        ("x", "Schließen"),
                                        ButtonStyle::Quiet,
                                        Availability::available(),
                                        LocalAction::Close,
                                        cx,
                                    )
                                    .size(px(32.)),
                                ),
                        )
                        .when(self.thread_options_open, |pane| {
                            pane.child(self.thread_options(cx))
                        })
                        .child(
                            self.scroll_area(super::navigation::Region::Overlay)
                                .flex_1()
                                .min_h_0()
                                .child(self.thread_body(cx)),
                        )
                        .child(self.thread_composer_view(cx)),
                ),
            )
            .child(
                div()
                    .id("thread-resize-handle")
                    .absolute()
                    .left_0()
                    .top_3()
                    .bottom_3()
                    .w(px(8.))
                    .track_focus(&resize_focus)
                    .cursor(gpui::CursorStyle::ResizeLeftRight)
                    .on_mouse_down(gpui::MouseButton::Left, move |_, window, _| {
                        resize_focus.focus(window)
                    })
                    .on_key_down(cx.listener(
                        move |this, event: &gpui::KeyDownEvent, window, cx| {
                            if !keyboard_focus.is_focused(window) {
                                return;
                            }
                            let change = match event.keystroke.key.as_str() {
                                "left" => 32.,
                                "right" => -32.,
                                _ => return,
                            };
                            let maximum =
                                (f32::from(window.viewport_size().width) - 600.).clamp(288., 608.);
                            this.thread_width = (this.thread_width + change).clamp(288., maximum);
                            cx.stop_propagation();
                            cx.notify();
                        },
                    ))
                    .on_drag(ThreadResize, |_, _, _, cx| cx.new(|_| ThreadResize))
                    .on_drag_move::<ThreadResize>(cx.listener(
                        |this, event: &gpui::DragMoveEvent<ThreadResize>, window, cx| {
                            let max = (f32::from(window.viewport_size().width) - 600.)
                                .max(288.)
                                .min(608.);
                            this.thread_width = (f32::from(window.viewport_size().width)
                                - f32::from(event.event.position.x))
                            .clamp(288., max);
                            cx.notify();
                        },
                    )),
            )
    }

    fn thread_heading(&self, cx: &Context<Self>) -> Div {
        let heading = self
            .current_thread_details()
            .map(|details| details.title.as_str())
            .filter(|title| !title.trim().is_empty())
            .unwrap_or(self.l("Thread"));
        let room = self.selected().map_or("", |room| room.title.as_str());
        div()
            .flex_1()
            .min_w_0()
            .flex()
            .flex_col()
            .child(if self.current_thread_details().is_some() {
                self.control(
                    "thread-options",
                    ButtonStyle::Quiet,
                    Availability::available(),
                    LocalAction::ToggleThreadOptions,
                    cx,
                )
                .justify_start()
                .p_0()
                .min_h(px(24.))
                .font_weight(FontWeight::BOLD)
                .child(heading.to_owned())
                .child(self.scaled_icon("chevron-down", 14., self.theme.muted))
                .into_any_element()
            } else {
                div()
                    .font_weight(FontWeight::BOLD)
                    .child(heading.to_owned())
                    .into_any_element()
            })
            .child(self.hint(room))
    }

    pub fn overlay_view(&self, window: &Window, cx: &Context<Self>) -> Div {
        if self.overlay == Some(Overlay::Forward) {
            return self.forward_dialog(window, cx);
        }
        if self.overlay == Some(Overlay::ContentPreview) {
            return self.media_view(window, cx);
        }
        if matches!(
            self.overlay,
            Some(Overlay::RoomInfo | Overlay::Participants)
        ) {
            return self.conversation_dialog(window, cx);
        }
        if matches!(self.overlay, Some(Overlay::Settings | Overlay::Accounts)) {
            return self.settings_view(window, cx);
        }
        let overlay = self.overlay.expect("overlay selected");
        let t = self.theme;
        let side = matches!(
            overlay,
            Overlay::RoomInfo
                | Overlay::Participants
                | Overlay::CallParticipants
                | Overlay::CallChat
        );
        let (title, width) = match overlay {
            Overlay::RoomInfo | Overlay::Participants => ("Gesprächsdetails", 448.),
            Overlay::Files => ("Geteilte Inhalte", 700.),
            Overlay::Thread => ("Antworten", 400.),
            Overlay::CallParticipants => ("Im Anruf", 400.),
            Overlay::CallChat => ("Chat im Anruf", 440.),
            Overlay::Search => ("Suchen", 700.),
            Overlay::Create => ("Neues Gespräch", 716.),
            Overlay::Discover => ("Offene Gespräche", 760.),
            Overlay::Organize => ("Eigene Gruppen", 520.),
            Overlay::ContentPreview => ("Dateivorschau", 820.),
            Overlay::CreatePoll => ("Umfrage erstellen", 620.),
            Overlay::Forward => unreachable!("forward dialog handled above"),
            _ => unreachable!("settings handled above"),
        };
        let body = match overlay {
            Overlay::Files => self.shared_contents(cx),
            Overlay::RoomInfo | Overlay::Participants => self.details_body(overlay, cx),
            Overlay::CallParticipants => {
                let mut body = div().flex().flex_col().gap_3();
                if let Some(call) = &self.snapshot.call {
                    body = body.child(self.hint(call.title.as_str()));
                    for p in call.participants.as_slice() {
                        body = body.child(self.participant_row(p));
                    }
                }
                body
            }
            Overlay::CallChat => self.call_chat(cx),
            Overlay::Thread => self.thread_body(cx),
            Overlay::Search => self.search_body(cx),
            Overlay::Create => self.creation_body(cx),
            Overlay::Discover => self.discover_body(cx),
            Overlay::Organize => self.organization_body(cx),
            Overlay::ContentPreview => unreachable!("media viewer handled above"),
            Overlay::CreatePoll => self.poll_creation_body(cx),
            Overlay::Forward => unreachable!("forward dialog handled above"),
            _ => unreachable!(),
        };
        let content = if overlay == Overlay::CallChat {
            div().flex_1().min_h_0().child(body).into_any_element()
        } else {
            self.scroll_area(super::navigation::Region::Overlay)
                .min_h_0()
                .flex_1()
                .when(overlay != Overlay::Thread, |d| d.p_6())
                .child(body)
                .into_any_element()
        };
        div()
            .absolute()
            .inset_0()
            .occlude()
            .bg(t.scrim)
            .flex()
            .when(side, |d| d.justify_end().p_3())
            .when(!side, |d| d.items_center().justify_center().p_5())
            .child(
                self.dialog_panel(
                    DialogSurface::Overlay(overlay),
                    div()
                        .w_full()
                        .max_w(px(width))
                        .max_h_full()
                        .min_h_0()
                        .when(side, |d| d.h_full())
                        .when(overlay == Overlay::Create, |d| {
                            d.h(px((f32::from(window.viewport_size().height)
                                - super::window_chrome::HEIGHT
                                - 66.)
                                .min(632.)))
                        })
                        .when(!side, |d| {
                            d.max_h(px((f32::from(window.viewport_size().height)
                                - super::window_chrome::HEIGHT
                                - 66.)
                                .min(632.)))
                        })
                        .flex()
                        .flex_col()
                        .bg(t.surface)
                        .rounded(px(13.))
                        .border_1()
                        .border_color(t.border)
                        .shadow_lg()
                        .track_focus(&self.modal_focus)
                        .tab_group()
                        .occlude()
                        .child(
                            div()
                                .flex_none()
                                .flex()
                                .items_center()
                                .justify_between()
                                .gap_3()
                                .px_6()
                                .py_4()
                                .border_b_1()
                                .border_color(t.border)
                                .when(overlay == Overlay::Thread, |header| {
                                    header.child(self.thread_heading(cx))
                                })
                                .when(overlay != Overlay::Thread, |header| {
                                    header.child(
                                        div()
                                            .min_w_0()
                                            .text_size(self.sp(if side { 18. } else { 24. }))
                                            .font_weight(FontWeight::BOLD)
                                            .child(self.l(title)),
                                    )
                                })
                                .child(self.icon_button(
                                    "close-overlay",
                                    ("x", "Schließen"),
                                    ButtonStyle::Quiet,
                                    Availability::available(),
                                    LocalAction::Close,
                                    cx,
                                )),
                        )
                        .when(
                            overlay == Overlay::Thread && self.thread_options_open,
                            |pane| pane.child(self.thread_options(cx)),
                        )
                        .child(content)
                        .when(overlay == Overlay::Thread, |d| {
                            d.child(self.thread_composer_view(cx))
                        })
                        .when(overlay == Overlay::Create, |d| {
                            d.child(self.creation_footer(cx))
                        })
                        .when(
                            self.feedback_visible() && overlay != Overlay::CallChat,
                            |d| {
                                d.child(
                                    div()
                                        .flex_none()
                                        .px_4()
                                        .pb_3()
                                        .child(self.feedback_view(cx)),
                                )
                            },
                        ),
                ),
            )
    }
    fn participant_row(&self, p: &ParticipantView) -> Div {
        let t = self.theme;
        div()
            .flex()
            .items_center()
            .gap_3()
            .py_3()
            .border_b_1()
            .border_color(t.border)
            .child(self.person_avatar(p.initials.as_str(), 36.))
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .child(
                        div()
                            .font_weight(FontWeight::MEDIUM)
                            .text_ellipsis()
                            .child(format!(
                                "{}{}",
                                p.name.as_str(),
                                if p.self_user { self.l(" · Du") } else { "" }
                            )),
                    )
                    .child(self.hint(p.role_label.as_str())),
            )
            .child(
                div()
                    .text_size(self.sp(12.))
                    .text_color(t.muted)
                    .child(self.l(match p.presence {
                        Presence::Available => "Verfügbar",
                        Presence::Away => "Abwesend",
                        Presence::DoNotDisturb => "Nicht stören",
                        Presence::Offline => "Offline",
                        Presence::Unknown => "Status unbekannt",
                    })),
            )
    }
    fn details_body(&self, overlay: Overlay, cx: &Context<Self>) -> Div {
        let mut body = div().flex().flex_col().gap_5();
        let Some(row) = self.selected() else {
            return body.child(self.hint(self.l("Das Gespräch ist nicht mehr verfügbar.")));
        };
        body = body
            .child(
                div()
                    .flex()
                    .flex_col()
                    .items_center()
                    .gap_3()
                    .py_3()
                    .child(self.conversation_avatar(row))
                    .child(
                        div()
                            .text_size(self.sp(22.))
                            .font_weight(FontWeight::BOLD)
                            .text_center()
                            .child(row.title.as_str().to_owned()),
                    ),
            )
            .child(
                div().flex().flex_wrap().gap_3().children(
                    [
                        (Overlay::RoomInfo, "Übersicht"),
                        (Overlay::Participants, "Personen"),
                        (Overlay::Files, "Dateien"),
                    ]
                    .into_iter()
                    .enumerate()
                    .map(|(i, (o, l))| {
                        self.chip(
                            ("details-tab", i),
                            l,
                            (overlay == o, i, 3),
                            LocalAction::Open(o),
                            cx,
                        )
                    }),
                ),
            );
        match overlay {
            Overlay::RoomInfo => {
                body = body
                    .child(self.detail_value(
                        "Gesprächsart",
                        match row.kind {
                            ConversationKind::Direct => "Persönliches Gespräch",
                            ConversationKind::Group => "Gruppengespräch",
                            ConversationKind::GuestAccessible => "Gespräch mit Gastzugang",
                            ConversationKind::NoteToSelf => "Notiz an mich",
                            ConversationKind::Unknown => "Noch nicht bekannt",
                        },
                    ))
                    .child(
                        self.detail_value(
                            "Teilnehmende",
                            &row.participants
                                .map_or_else(|| "Noch nicht bekannt".into(), |n| n.to_string()),
                        ),
                    )
                    .child(self.hint(self.l("Zugang & Einladungen")));
                body = body.child(self.button(
                    "archive-conversation",
                    self.l(if row.archived == Some(true) {
                        "Aus Archiv holen"
                    } else {
                        "Archivieren"
                    }),
                    ButtonStyle::Secondary,
                    Availability::when(
                        row.archived.is_some()
                            && row.archive == talk_core::ui::Decision::Allowed
                            && self.snapshot.conversation_list.content == ContentState::Ready,
                        "Der Archivstatus kann derzeit nicht geändert werden.",
                    ),
                    LocalAction::Archive(row.context, row.archived != Some(true)),
                    cx,
                ));
                for (id, title, name, reason) in [
                    (
                        "room-invite",
                        "Personen einladen",
                        "users",
                        "Einladungen sind derzeit nicht verfügbar.",
                    ),
                    (
                        "room-guest",
                        "Gastzugang",
                        "link",
                        "Die Zugangseinstellungen sind noch nicht verfügbar.",
                    ),
                    (
                        "room-directory",
                        "Im Verzeichnis anzeigen",
                        "compass",
                        "Die Sichtbarkeit ist noch nicht bekannt.",
                    ),
                ] {
                    body = body.child(self.option_row(
                        id,
                        (name, title, ""),
                        Availability::when(false, reason),
                        LocalAction::Notice(""),
                        cx,
                    ));
                }
            }
            Overlay::Participants => {
                for p in self.snapshot.participants.as_slice() {
                    body = body.child(self.participant_row(p));
                }
                if row.participants.is_some_and(|count| {
                    usize::from(count) > self.snapshot.participants.as_slice().len()
                }) {
                    body =
                        body.child(self.hint(self.l("Weitere Teilnehmende sind nicht geladen.")));
                }
                if self.snapshot.participants.as_slice().is_empty() {
                    body = body.child(self.state_note(
                        "users",
                        "Teilnehmende nicht verfügbar",
                        "Die Teilnehmerliste ist noch nicht geladen.",
                    ));
                }
            }
            Overlay::Files => return self.shared_files(cx),
            _ => (),
        }
        body
    }
    fn detail_value(&self, label: &str, value: &str) -> Div {
        div()
            .flex()
            .justify_between()
            .gap_4()
            .pb_3()
            .border_b_1()
            .border_color(self.theme.border)
            .child(self.hint(self.l(label)))
            .child(
                div()
                    .min_w_0()
                    .font_weight(FontWeight::MEDIUM)
                    .child(self.l(value).to_owned()),
            )
    }
    pub(super) fn state_note(&self, symbol: &'static str, title: &str, detail: &str) -> Div {
        let t = self.theme;
        div()
            .flex()
            .flex_col()
            .items_center()
            .gap_3()
            .py_6()
            .px_3()
            .text_center()
            .child(
                div()
                    .size(px(52.))
                    .rounded(px(16.))
                    .bg(t.subtle)
                    .flex()
                    .items_center()
                    .justify_center()
                    .child(self.scaled_icon(symbol, 26., t.accent)),
            )
            .child(
                div()
                    .text_size(self.sp(18.))
                    .font_weight(FontWeight::MEDIUM)
                    .child(self.l(title).to_owned()),
            )
            .child(self.hint(self.l(detail)))
    }
    fn thread_body(&self, cx: &Context<Self>) -> Div {
        let mut body = div().flex().flex_col();
        let Some((context, root)) = self
            .thread_parent
            .filter(|(context, _)| Some(*context) == self.snapshot.selected)
        else {
            return body;
        };
        let panel = self
            .snapshot
            .message_panel
            .as_ref()
            .filter(|p| p.context == context && p.thread_root == Some(root));
        let load = self
            .message_ui
            .thread_load
            .as_ref()
            .filter(|load| load.context == context && load.root == root);
        let projected = super::messaging::ThreadLoad {
            context,
            root,
            request: Some(talk_core::ui::presentation::RequestId(panel.map_or(
                0,
                |p| {
                    use talk_core::ui::conversation::RemoteState;
                    match p.thread_state {
                        RemoteState::Idle => 0,
                        RemoteState::Loading(id)
                        | RemoteState::Saving(id)
                        | RemoteState::Failed(id, _)
                        | RemoteState::Uncertain(id) => id.0,
                    }
                },
            ))),
        };
        let state = load.unwrap_or(&projected).content_state(panel);
        // The thread history endpoint can return replies without the root. The
        // main timeline is already room-scoped and can supply it immediately.
        let root_message = self
            .snapshot
            .messages
            .as_slice()
            .iter()
            .find(|message| message.local_id == root)
            .or_else(|| panel.and_then(|panel| panel.thread_root_message.as_ref()))
            .or_else(|| {
                panel.and_then(|panel| {
                    panel
                        .thread_messages
                        .as_slice()
                        .iter()
                        .find(|message| message.local_id == root)
                })
            });
        if let Some(message) = root_message {
            body = body.child(self.thread_message(message, false, cx));
        } else if panel.is_some_and(|panel| {
            matches!(
                panel.thread_root_state,
                talk_core::ui::conversation::RemoteState::Failed(_, _)
            )
        }) {
            body = body.child(self.hint(self.l("Thread-Anfang konnte nicht geladen werden.")));
            body = body.child(self.button(
                "thread-root-retry",
                "Erneut versuchen",
                ButtonStyle::Quiet,
                true,
                LocalAction::MessageIn(
                    context,
                    super::messaging::Action::Command(
                        talk_core::ui::messaging::MessageAction::ThreadRoot(root),
                    ),
                ),
                cx,
            ));
        } else {
            body = body.child(self.hint(self.l("Thread-Anfang wird geladen …")));
        }
        let collection = CollectionState::new(
            state,
            panel.is_some_and(|panel| !panel.thread_messages.as_slice().is_empty()),
        );
        if state != ContentState::Ready {
            body = body.child(self.collection_feedback(
                collection,
                "retry-thread",
                LocalAction::Thread(context, root),
                cx,
            ));
        }
        if let Some(panel) =
            panel.filter(|_| collection.shows_content() || collection.confirmed_empty())
        {
            let loaded = panel
                .thread_messages
                .as_slice()
                .iter()
                .filter(|m| m.local_id != root)
                .count() as u64;
            let count =
                root_message.map_or(loaded, |message| loaded.max(u64::from(message.reply_count)));
            body = body.child(
                div()
                    .flex()
                    .items_center()
                    .gap_2()
                    .px_6()
                    .py_3()
                    .child(self.hint(&self.count(super::localization::Count::Replies, count)))
                    .child(div().h(px(1.)).flex_1().bg(self.theme.border)),
            );
            let mut previous = None;
            for message in panel
                .thread_messages
                .as_slice()
                .iter()
                .filter(|message| message.local_id != root)
            {
                let continuation = previous
                    .is_some_and(|previous| self.thread_messages_grouped(previous, message));
                body = body.child(self.thread_message(message, continuation, cx));
                previous = Some(message);
            }
        }
        body
    }
    fn current_thread_details(&self) -> Option<&talk_core::ui::messaging::ThreadDetails> {
        let (context, root) = self.thread_parent?;
        self.snapshot
            .message_panel
            .as_ref()
            .filter(|panel| panel.context == context)
            .and_then(|panel| panel.thread_details.as_ref())
            .filter(|details| details.root == root)
    }
    fn thread_options(&self, cx: &Context<Self>) -> Div {
        let Some(details) = self.current_thread_details() else {
            return div();
        };
        let Some((context, root)) = self.thread_parent else {
            return div();
        };
        let can_change = self
            .snapshot
            .message_panel
            .as_ref()
            .is_some_and(|panel| panel.context == context && !panel.state.pending())
            && self.message_feature_enabled(talk_core::ui::messaging::MessageFeature::Threads);
        let mut body = div()
            .flex()
            .flex_col()
            .gap_2()
            .p_3()
            .border_b_1()
            .border_color(self.theme.border);
        let following = details.notification_level.is_some_and(|level| level != 3);
        body = body.child(
            self.button(
                "thread-follow",
                if following {
                    "Thread nicht mehr folgen"
                } else {
                    "Thread folgen"
                },
                ButtonStyle::Quiet,
                can_change && details.notification_level.is_some(),
                LocalAction::ThreadNotification(if following { 3 } else { 0 }),
                cx,
            )
            .justify_start(),
        );
        if following {
            body = body.child(
                div().flex().flex_wrap().gap_1().children(
                    [
                        ("thread-notify-all", "Alle Antworten", 1),
                        ("thread-notify-mentions", "Nur Erwähnungen", 2),
                    ]
                    .into_iter()
                    .map(|(id, label, level)| {
                        self.button(
                            id,
                            label,
                            ButtonStyle::Secondary,
                            can_change,
                            LocalAction::ThreadNotification(level),
                            cx,
                        )
                        .when(details.notification_level == Some(level), |button| {
                            button.bg(self.theme.selected)
                        })
                    }),
                ),
            );
        }
        let can_rename = self
            .snapshot
            .messages
            .as_slice()
            .iter()
            .chain(
                self.snapshot
                    .message_panel
                    .as_ref()
                    .into_iter()
                    .flat_map(|panel| {
                        panel
                            .thread_root_message
                            .iter()
                            .chain(panel.thread_messages.as_slice())
                    }),
            )
            .any(|message| {
                message.local_id == root
                    && message.metadata.edit == talk_core::ui::Decision::Allowed
            });
        if can_rename {
            body = if self.thread_renaming {
                body.child(
                    self.input_surface(&self.thread_title, cx)
                        .child(self.thread_title.clone()),
                )
                .child(self.button(
                    "thread-save-title",
                    "Speichern",
                    ButtonStyle::Secondary,
                    can_change && !self.thread_title.read(cx).content.trim().is_empty(),
                    LocalAction::SaveThreadRename,
                    cx,
                ))
            } else {
                body.child(
                    self.button(
                        "thread-rename",
                        "Thread umbenennen",
                        ButtonStyle::Quiet,
                        can_change,
                        LocalAction::BeginThreadRename,
                        cx,
                    )
                    .justify_start(),
                )
            };
        }
        body
    }
    fn search_body(&self, cx: &Context<Self>) -> Div {
        let t = self.theme;
        let q = self.search.read(cx).content.trim().to_lowercase();
        let mut body =
            div()
                .flex()
                .flex_col()
                .gap_4()
                .child(
                    self.input_surface(&self.search, cx)
                        .min_h(px(48.))
                        .child(self.scaled_icon("search", 20., t.muted))
                        .child(div().flex_1().min_w_0().child(self.search.clone()))
                        .child(
                            self.button(
                                "search-people",
                                "Personen suchen",
                                ButtonStyle::Secondary,
                                !q.is_empty()
                                    && self.snapshot.active_account.is_some()
                                    && self
                                        .snapshot
                                        .create_panel
                                        .as_ref()
                                        .is_none_or(|p| !p.state.pending()),
                                LocalAction::FindSearchPeople,
                                cx,
                            ),
                        ),
                )
                .child(self.hint(self.l(
                    "Geladene Inhalte sofort filtern oder Personen im Serververzeichnis suchen.",
                )))
                .child(
                    div().flex().flex_wrap().gap_3().children(
                        [
                            (SearchKind::All, "Alles"),
                            (SearchKind::Messages, "Nachrichten"),
                            (SearchKind::Files, "Dateien"),
                            (SearchKind::Conversations, "Gespräche"),
                            (SearchKind::People, "Personen"),
                        ]
                        .into_iter()
                        .enumerate()
                        .map(|(i, (k, l))| {
                            self.chip(
                                ("search-tab", i),
                                l,
                                (self.search_kind == k, i, 5),
                                LocalAction::SearchKind(k),
                                cx,
                            )
                        }),
                    ),
                );
        if q.is_empty() {
            return body.child(self.state_note(
                "search",
                "Gespräche und Personen suchen",
                "Gib einen Namen oder einen Suchbegriff ein.",
            ));
        }
        let mut count = 0;
        if matches!(self.search_kind, SearchKind::All | SearchKind::People) {
            let requested = self
                .directory_query
                .as_ref()
                .is_some_and(|(account, query)| {
                    Some(*account) == self.snapshot.active_account && query == &q
                });
            if requested {
                if let Some(panel) = self
                    .snapshot
                    .create_panel
                    .as_ref()
                    .filter(|p| Some(p.account) == self.snapshot.active_account)
                {
                    body = body.child(self.title(self.l("Personen")));
                    for person in panel.people.as_slice() {
                        count += 1;
                        body = body.child(
                            self.button(
                                gpui::SharedString::from(format!(
                                    "search-person-{}",
                                    person.id.local
                                )),
                                "",
                                ButtonStyle::Quiet,
                                !panel.state.pending(),
                                LocalAction::CreateDirect(person.id),
                                cx,
                            )
                            .w_full()
                            .justify_start()
                            .gap_3()
                            .child(self.content_avatar(
                                talk_core::ui::content::AssetTarget::PersonAvatar(person.id),
                                "?",
                                32.,
                            ))
                            .child(
                                div()
                                    .flex_1()
                                    .min_w_0()
                                    .child(person.name.as_str().to_owned())
                                    .child(self.hint(person.label.as_str())),
                            ),
                        );
                    }
                    if panel.state.pending() {
                        body = body.child(self.hint(self.l("Personen werden gesucht …")));
                    } else if matches!(
                        panel.state,
                        talk_core::ui::conversation::RemoteState::Failed(_, _)
                    ) {
                        body = body.child(
                            self.hint(self.l("Personensuche fehlgeschlagen. Erneut versuchen.")),
                        );
                    } else if panel.people_loaded && panel.people.as_slice().is_empty() {
                        body = body.child(self.hint(self.l("Keine passenden Personen gefunden.")));
                    }
                }
            } else {
                body = body.child(self.hint(
                    self.l("Mit „Personen suchen“ das freigegebene Serververzeichnis durchsuchen."),
                ));
            }
        }
        if matches!(
            self.search_kind,
            SearchKind::All | SearchKind::Conversations
        ) {
            for row in self
                .snapshot
                .conversations
                .as_slice()
                .iter()
                .filter(|r| r.title.as_str().to_lowercase().contains(&q))
            {
                count += 1;
                body = body.child(self.option_row(
                    scoped_id("search-room", row.context, 0, 0),
                    ("chat", row.title.as_str(), row.subtitle.as_str()),
                    true,
                    LocalAction::Navigate(row.context),
                    cx,
                ));
            }
        }
        if let Some(context) = self.snapshot.selected {
            if matches!(self.search_kind, SearchKind::All | SearchKind::Messages) {
                for m in self
                    .snapshot
                    .messages
                    .as_slice()
                    .iter()
                    .filter(|m| m.body.as_str().to_lowercase().contains(&q))
                {
                    count += 1;
                    body = body
                        .child(
                            self.button(
                                scoped_id("search-message", context, m.local_id, 0),
                                "",
                                ButtonStyle::Quiet,
                                true,
                                LocalAction::LocateMessage(context, m.local_id),
                                cx,
                            )
                            .child(m.body.as_str().to_owned())
                            .w_full()
                            .justify_start()
                            .text_size(self.sp(14.))
                            .py_3(),
                        )
                        .child(self.hint(&format!(
                            "{} · {}",
                            m.author.as_str(),
                            m.time_label.as_str()
                        )));
                }
            }
            if matches!(self.search_kind, SearchKind::All | SearchKind::Files) {
                for file in self
                    .snapshot
                    .files
                    .as_slice()
                    .iter()
                    .filter(|f| f.name.as_str().to_lowercase().contains(&q))
                {
                    count += 1;
                    body = body.child(self.option_row(
                        scoped_id("search-file", context, 0, file.local_id),
                        ("file", file.name.as_str(), file.detail.as_str()),
                        true,
                        LocalAction::MessageIn(context, super::messaging::Action::Files),
                        cx,
                    ));
                }
            }
        }
        if count == 0 && self.search_kind != SearchKind::People {
            body=body.child(self.state_note("search","Keine lokalen Treffer","In den geladenen Inhalten wurde nichts gefunden. Eine vollständige Serversuche ist hier noch nicht verfügbar."));
        }
        body
    }
    fn discover_body(&self, cx: &Context<Self>) -> Div {
        div().flex().flex_col().gap_5().child(self.hint(self.l("Finde Gespräche, die für dich im Verzeichnis freigegeben sind.")))
            .child(self.state_note("compass","Verzeichnis nicht verfügbar","Auffindbarkeit und Beitrittsrechte sind noch nicht bekannt. Ein Gastlink allein macht ein Gespräch nicht öffentlich auffindbar."))
            .child(self.button_with_icon("discover-refresh",("Verzeichnis laden","refresh"),ButtonStyle::Secondary,Availability::when(false,"Das Gesprächsverzeichnis ist in diesem Client noch nicht verfügbar."),LocalAction::Notice(""),cx))
            .child(self.button("discover-back","Zurück zu deinen Gesprächen",ButtonStyle::Quiet,true,LocalAction::Close,cx))
    }
}
