use super::controls::{Availability, scoped_id};
use super::{ButtonStyle, Desktop, LocalAction, Overlay, SearchKind};
use gpui::{Context, Div, Focusable, FontWeight, Window, div, prelude::*, px};
use talk_core::ui::presentation::{ContentState, ConversationKind, ParticipantView, Presence};

impl Desktop {
    pub fn overlay_view(&self, window: &Window, cx: &Context<Self>) -> Div {
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
            Overlay::Files => ("Dateien", 700.),
            Overlay::Thread => ("Antworten", 400.),
            Overlay::CallParticipants => ("Im Anruf", 400.),
            Overlay::CallChat => ("Chat im Anruf", 440.),
            Overlay::Search => ("Suchen", 700.),
            Overlay::Create => ("Neues Gespräch", 716.),
            Overlay::Discover => ("Offene Gespräche", 760.),
            Overlay::Organize => ("Eigene Gruppen", 520.),
            _ => unreachable!("settings handled above"),
        };
        let body = match overlay {
            Overlay::RoomInfo | Overlay::Participants | Overlay::Files => {
                self.details_body(overlay, cx)
            }
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
            _ => unreachable!(),
        };
        let content = if overlay == Overlay::CallChat {
            div().flex_1().min_h_0().child(body).into_any_element()
        } else {
            self.scroll_area(super::navigation::Region::Overlay)
                .min_h_0()
                .flex_1()
                .p_6()
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
                div()
                    .w_full()
                    .max_w(px(width))
                    .max_h_full()
                    .min_h_0()
                    .when(side, |d| d.h_full())
                    .when(overlay == Overlay::Create, |d| {
                        d.h(px(
                            (f32::from(window.viewport_size().height) - 66.).min(632.)
                        ))
                    })
                    .when(!side, |d| {
                        d.max_h(px(
                            (f32::from(window.viewport_size().height) - 66.).min(632.)
                        ))
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
                            .child(
                                div()
                                    .min_w_0()
                                    .text_size(self.sp(if side { 18. } else { 24. }))
                                    .font_weight(FontWeight::BOLD)
                                    .child(self.l(title)),
                            )
                            .child(self.icon_button(
                                "close-overlay",
                                ("x", "Schließen"),
                                ButtonStyle::Quiet,
                                Availability::available(),
                                LocalAction::Close,
                                cx,
                            )),
                    )
                    .child(content)
                    .when(overlay == Overlay::Create, |d| {
                        d.child(self.creation_footer(cx))
                    })
                    .when(self.feedback.is_some_and(|f| f.context.is_none()), |d| {
                        d.child(
                            div()
                                .flex_none()
                                .px_4()
                                .pb_3()
                                .child(self.feedback_view(cx)),
                        )
                    }),
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
                            .font_weight(FontWeight::SEMIBOLD)
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
                    .font_weight(FontWeight::SEMIBOLD)
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
                    .font_weight(FontWeight::SEMIBOLD)
                    .child(self.l(title).to_owned()),
            )
            .child(self.hint(self.l(detail)))
    }
    fn thread_body(&self, cx: &Context<Self>) -> Div {
        let mut body = div().flex().flex_col().gap_4();
        let Some((context, root)) = self
            .thread_parent
            .filter(|(context, _)| Some(*context) == self.snapshot.selected)
        else {
            return body;
        };
        let Some(panel) = self
            .snapshot
            .message_panel
            .as_ref()
            .filter(|p| p.context == context && p.thread_root == Some(root))
        else {
            return body.child(self.hint(self.l("Nachrichten werden geladen …")));
        };
        body = body.child(
            self.hint(
                &self.count(
                    super::localization::Count::Replies,
                    panel
                        .thread_messages
                        .as_slice()
                        .iter()
                        .filter(|m| m.local_id != root)
                        .count() as u64,
                ),
            ),
        );
        for message in panel.thread_messages.as_slice() {
            body = body.child(
                div()
                    .flex()
                    .flex_col()
                    .gap_2()
                    .child(
                        div()
                            .font_weight(FontWeight::SEMIBOLD)
                            .child(message.author.as_str().to_owned()),
                    )
                    .child(if message.metadata.deleted {
                        self.l("Nachricht gelöscht.").to_owned()
                    } else {
                        message.body.as_str().to_owned()
                    }),
            );
        }
        body.child(
            self.button(
                "thread-reply",
                "Im Thread antworten",
                ButtonStyle::Secondary,
                Availability::when(
                    panel
                        .availability(talk_core::ui::messaging::MessageFeature::Threads)
                        .enabled(),
                    "Du hast dafür keine Berechtigung.",
                ),
                LocalAction::Message(super::messaging::Action::ThreadReply(root)),
                cx,
            ),
        )
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
                    div()
                        .relative()
                        .flex()
                        .items_center()
                        .gap_3()
                        .rounded(px(8.))
                        .bg(t.subtle)
                        .px_3()
                        .min_h(px(48.))
                        .child(self.field_ring(self.search.focus_handle(cx), 8.))
                        .child(self.scaled_icon("search", 20., t.muted))
                        .child(div().flex_1().min_w_0().child(self.search.clone())),
                )
                .child(self.hint(self.l(
                    "Nur geladene Gespräche sowie Nachrichten und Dateien des offenen Gesprächs.",
                )))
                .child(
                    div().flex().flex_wrap().gap_3().children(
                        [
                            (SearchKind::All, "Alles"),
                            (SearchKind::Messages, "Nachrichten"),
                            (SearchKind::Files, "Dateien"),
                            (SearchKind::Conversations, "Gespräche"),
                        ]
                        .into_iter()
                        .enumerate()
                        .map(|(i, (k, l))| {
                            self.chip(
                                ("search-tab", i),
                                l,
                                (self.search_kind == k, i, 4),
                                LocalAction::SearchKind(k),
                                cx,
                            )
                        }),
                    ),
                );
        if q.is_empty() {
            return body.child(self.state_note(
                "search",
                "In geladenen Inhalten suchen",
                "Gib einen Namen oder einen Suchbegriff ein.",
            ));
        }
        let mut count = 0;
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
                        LocalAction::Open(Overlay::Files),
                        cx,
                    ));
                }
            }
        }
        if count == 0 {
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
