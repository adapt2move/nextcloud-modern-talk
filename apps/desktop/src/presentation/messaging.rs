//! Stable, variable-height native history and deliberate message actions.
use super::{
    ButtonStyle, Desktop, LocalAction, Overlay,
    controls::{Availability, scoped_id},
    input::TextInput,
};
use gpui::{
    Context, Div, Entity, Focusable, FontWeight, ListAlignment, ListOffset, ListState, Window, div,
    list, prelude::*, px,
};
use talk_core::ui::messaging::{ComposeTarget, MessageAction, MessageFeature};
use talk_core::ui::presentation::{
    Completion, ContentState, ConversationRef, MessageView, RequestId, UiIntent,
};
use talk_core::ui::{Decision, Text};

#[derive(Clone)]
pub(crate) enum Action {
    Files,
    Silent,
    Pins,
    ShareUpload,
    ChooseForward(u128, ConversationRef),
    ThreadReply(u128),
    Menu(u128),
    Reply(u128),
    Edit(u128),
    CancelCompose,
    Command(MessageAction),
    ConfirmDelete(u128),
    Delete(u128),
    Forward(u128),
    ConfirmForward(u128, ConversationRef),
    Search,
    Schedule,
    Remind(u128),
    SubmitTime,
    InsertMention(String),
}
pub(super) struct MessageUi {
    pub list: ListState,
    context: Option<ConversationRef>,
    observed: Vec<MessageView>,
    pub editing: Option<u128>,
    pending_edit: Option<RequestId>,
    request: u64,
    menu: Option<u128>,
    confirm_delete: Option<u128>,
    forward: Option<u128>,
    forward_destination: Option<ConversationRef>,
    last_read: Option<(ConversationRef, u128)>,
    last_located: Option<RequestId>,
    time_target: Option<Option<u128>>,
    pub time_input: Entity<TextInput>,
    time_error: bool,
    pub search: Entity<TextInput>,
    show_search: bool,
    show_pins: bool,
}
impl MessageUi {
    pub fn new(cx: &mut Context<Desktop>) -> Self {
        let theme = super::theme::Theme::resolve(
            super::theme::Palette::Blue,
            super::theme::Appearance::Light,
            gpui::WindowAppearance::Light,
        );
        Self {
            list: ListState::new(0, ListAlignment::Bottom, px(300.)),
            context: None,
            observed: Vec::new(),
            editing: None,
            pending_edit: None,
            request: 0,
            menu: None,
            confirm_delete: None,
            forward: None,
            forward_destination: None,
            last_read: None,
            last_located: None,
            time_target: None,
            time_input: cx.new(|cx| TextInput::new("2026-10-21 09:00 +02:00", 32, theme, cx)),
            time_error: false,
            search: cx.new(|cx| TextInput::new("Im Gespräch suchen …", 256, theme, cx)),
            show_search: false,
            show_pins: false,
        }
    }
}
impl Desktop {
    pub(super) fn refresh_message_ui(&mut self, cx: &mut Context<Self>) {
        let rows = self.snapshot.messages.as_slice();
        if self.message_ui.context != self.snapshot.selected {
            self.message_ui.context = self.snapshot.selected;
            self.message_ui.list.reset(rows.len());
            self.message_ui.observed = rows.to_vec();
            self.message_ui.menu = None;
            self.message_ui.forward = None;
            self.message_ui.forward_destination = None;
            self.message_ui.confirm_delete = None;
            self.message_ui.editing = None;
            self.message_ui.pending_edit = None;
            self.message_ui.time_target = None;
            self.message_ui.show_search = false;
            self.message_ui.show_pins = false;
            self.message_ui.last_located = None;
        } else if self.message_ui.observed != rows {
            let anchor = self.message_ui.list.logical_scroll_top();
            let id = self
                .message_ui
                .observed
                .get(anchor.item_ix)
                .map(|m| m.local_id);
            self.message_ui
                .list
                .splice(0..self.message_ui.observed.len(), rows.len());
            if let Some(index) = id.and_then(|id| rows.iter().position(|m| m.local_id == id)) {
                self.message_ui.list.scroll_to(ListOffset {
                    item_ix: index,
                    offset_in_item: anchor.offset_in_item,
                });
            }
            self.message_ui.observed = rows.to_vec();
        }
        if let Some((request, id)) = self.snapshot.message_panel.as_ref().and_then(|p| p.located)
            && self.message_ui.last_located != Some(request)
            && let Some(index) = rows.iter().position(|m| m.local_id == id)
        {
            self.message_ui.list.scroll_to(ListOffset {
                item_ix: index,
                offset_in_item: px(0.),
            });
            self.message_ui.last_located = Some(request);
            self.highlighted_message = self.snapshot.selected.map(|context| (context, id));
        }
        if let Some((request, completion)) =
            self.snapshot.message_panel.as_ref().and_then(|p| p.result)
        {
            if self.message_ui.pending_edit == Some(request)
                && completion == Completion::ServerAccepted
            {
                self.message_ui.pending_edit = None;
                self.message_ui.editing = None;
                let body = self
                    .snapshot
                    .selected
                    .map_or("", |c| self.drafts.text(c))
                    .to_owned();
                self.composer.update(cx, |i, _| i.set_context_value(&body));
            }
            if let Some(context) = self.snapshot.selected {
                let _ = self
                    .port
                    .try_submit(UiIntent::AcknowledgeMessage { request, context });
            }
        }
    }
    pub(crate) fn observe_message_visibility(&mut self, window: &Window) {
        let Some(context) = self.visible_message_conversation(window) else {
            return;
        };
        self.observe_visible_message(context);
    }
    pub(crate) fn visible_message_conversation(&self, window: &Window) -> Option<ConversationRef> {
        if !window.is_window_active()
            || self.overlay.is_some()
            || self.prejoin.is_some()
            || self.incoming_visible()
            || self.snapshot.content != ContentState::Ready
        {
            return None;
        }
        self.snapshot.selected
    }
    fn observe_visible_message(&mut self, context: ConversationRef) {
        if !self
            .snapshot
            .message_panel
            .as_ref()
            .is_some_and(|p| p.availability(MessageFeature::ReadReceipts).enabled())
        {
            return;
        }
        let viewport = self.message_ui.list.viewport_bounds();
        let id = self
            .snapshot
            .messages
            .as_slice()
            .iter()
            .enumerate()
            .rev()
            .find_map(|(index, message)| {
                self.message_ui
                    .list
                    .bounds_for_item(index)
                    .filter(|bounds| {
                        bounds.intersects(&viewport) && bounds.bottom() <= viewport.bottom()
                    })
                    .map(|_| message.local_id)
            });
        if let Some(id) = id
            && self.message_ui.last_read != Some((context, id))
            && self.submit_message(MessageAction::MarkRead(id)).is_some()
        {
            self.message_ui.last_read = Some((context, id));
        }
    }
    fn message_available(&self, feature: MessageFeature) -> Availability {
        let value = self
            .snapshot
            .message_panel
            .as_ref()
            .map_or(talk_core::ui::conversation::Availability::UNKNOWN, |p| {
                p.availability(feature)
            });
        Availability::when(
            value.enabled(),
            super::conversation_dialog::unavailable(value),
        )
    }
    pub(super) fn submit_message(&mut self, action: MessageAction) -> Option<RequestId> {
        let context = self.snapshot.selected?;
        let request = RequestId(self.message_ui.request.checked_add(1)?);
        self.message_ui.request = request.0;
        self.submit(UiIntent::Message {
            request,
            context,
            action,
        })
        .then_some(request)
    }
    pub(super) fn message_action(
        &mut self,
        action: Action,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(context) = self.snapshot.selected else {
            return;
        };
        let anchor = self.message_ui.list.logical_scroll_top();
        match action {
            Action::Silent => {
                let mut target = self.drafts.target(context);
                target.silent = !target.silent;
                self.drafts.set_target(context, target);
            }
            Action::Pins => {
                self.message_ui.show_pins = !self.message_ui.show_pins;
                if self.message_ui.show_pins {
                    self.submit_message(MessageAction::Pins);
                }
            }
            Action::Files => {
                self.submit_message(MessageAction::Files);
                self.handle(LocalAction::Open(Overlay::Files), window, cx);
            }
            Action::ShareUpload => {
                if let Ok(caption) = Text::new(self.composer.read(cx).content.as_str()) {
                    self.submit_message(MessageAction::ShareUpload { caption });
                }
            }
            Action::ChooseForward(message, destination) => {
                self.message_ui.forward = Some(message);
                self.message_ui.forward_destination = Some(destination);
            }
            Action::ThreadReply(root) => {
                if self.save_draft(cx)
                    && self.drafts.set_target(
                        context,
                        ComposeTarget {
                            reply_to: None,
                            thread: Some(root),
                            silent: false,
                        },
                    )
                {
                    self.handle(LocalAction::Close, window, cx);
                    let value = self.drafts.text(context).to_owned();
                    self.composer.update(cx, |i, _| i.set_context_value(&value));
                    self.composer.focus_handle(cx).focus(window);
                }
            }
            Action::Menu(id) => {
                self.message_ui.menu = if self.message_ui.menu == Some(id) {
                    None
                } else {
                    Some(id)
                };
                self.message_ui.confirm_delete = None;
                self.message_ui.forward = None;
                self.message_ui.forward_destination = None;
            }
            Action::Reply(id) => {
                if self.save_draft(cx)
                    && self
                        .snapshot
                        .messages
                        .as_slice()
                        .iter()
                        .any(|m| m.local_id == id && m.metadata.replyable)
                {
                    if !self.drafts.set_target(
                        context,
                        ComposeTarget {
                            reply_to: Some(id),
                            thread: None,
                            silent: false,
                        },
                    ) {
                        self.feedback = Some(super::feedback::Feedback::from_error(
                            talk_core::ui::SubmitError::Busy,
                        ));
                        return;
                    }
                    let value = self.drafts.text(context).to_owned();
                    self.composer.update(cx, |i, _| i.set_context_value(&value));
                    self.message_ui.menu = None;
                    self.composer.focus_handle(cx).focus(window);
                }
            }
            Action::Edit(id) => {
                if self.save_draft(cx)
                    && let Some(message) = self.snapshot.messages.as_slice().iter().find(|m| {
                        m.local_id == id
                            && m.metadata.edit == Decision::Allowed
                            && !m.metadata.deleted
                    })
                {
                    let value = message.body.as_str().to_owned();
                    self.message_ui.editing = Some(id);
                    self.message_ui.menu = None;
                    self.composer.update(cx, |i, _| i.set_context_value(&value));
                    self.composer.focus_handle(cx).focus(window);
                }
            }
            Action::CancelCompose => {
                if self.message_ui.editing.take().is_none() {
                    self.save_draft(cx);
                    self.drafts.set_target(context, ComposeTarget::default());
                }
                let value = self.drafts.text(context).to_owned();
                self.composer.update(cx, |i, _| i.set_context_value(&value));
                self.message_ui.time_target = None;
            }
            Action::Command(command) => {
                self.submit_message(command);
            }
            Action::ConfirmDelete(id) => self.message_ui.confirm_delete = Some(id),
            Action::Delete(id) => {
                self.submit_message(MessageAction::Delete(id));
                self.message_ui.confirm_delete = None;
            }
            Action::Forward(id) => {
                self.message_ui.forward = Some(id);
                self.message_ui.forward_destination = None;
            }
            Action::ConfirmForward(message, destination) => {
                self.submit_message(MessageAction::Forward {
                    message,
                    destination,
                });
                self.message_ui.forward = None;
                self.message_ui.forward_destination = None;
            }
            Action::Search => {
                self.message_ui.show_search = true;
                if let Ok(query) = Text::new(self.message_ui.search.read(cx).content.as_str())
                    && !query.as_str().trim().is_empty()
                {
                    self.submit_message(MessageAction::Search(query));
                }
            }
            Action::Schedule => self.message_ui.time_target = Some(None),
            Action::Remind(id) => {
                self.message_ui.time_target = Some(Some(id));
                self.message_ui.menu = None;
            }
            Action::SubmitTime => {
                let timestamp = parse_time(self.message_ui.time_input.read(cx).content.as_str());
                self.message_ui.time_error = timestamp.is_none();
                if let Some(timestamp) = timestamp
                    && let Some(target) = self.message_ui.time_target
                {
                    let command = if let Some(message) = target {
                        Some(MessageAction::Reminder {
                            message,
                            timestamp: Some(timestamp),
                        })
                    } else {
                        Text::new(self.composer.read(cx).content.as_str())
                            .ok()
                            .map(|body| MessageAction::Schedule {
                                body,
                                timestamp,
                                target: self.drafts.target(context),
                            })
                    };
                    if let Some(command) = command
                        && self.submit_message(command).is_some()
                    {
                        self.message_ui.time_target = None;
                    }
                }
            }
            Action::InsertMention(identifier) => {
                let current = self.composer.read(cx).content.to_string();
                self.composer
                    .update(cx, |i, _| i.set_value(&format!("{current}@{identifier} ")));
            }
        }
        self.message_ui.list.splice(
            0..self.snapshot.messages.as_slice().len(),
            self.snapshot.messages.as_slice().len(),
        );
        self.message_ui.list.scroll_to(anchor);
        cx.notify();
    }
    pub(super) fn send_edit(&mut self, cx: &mut Context<Self>) -> bool {
        let Some(message) = self.message_ui.editing else {
            return false;
        };
        if let Ok(body) = Text::new(self.composer.read(cx).content.as_str()) {
            self.message_ui.pending_edit =
                self.submit_message(MessageAction::Edit { message, body });
        }
        true
    }
    pub(super) fn message_timeline(&self, constrained: bool, cx: &Context<Self>) -> Div {
        let t = self.theme;
        let mut body = div()
            .flex_1()
            .min_h_0()
            .flex()
            .flex_col()
            .when(constrained, |d| d.flex_none().h(px(200.)));
        let Some(row) = self.selected() else {
            return body;
        };
        let toolbar = div()
            .flex_none()
            .flex()
            .items_center()
            .flex_wrap()
            .gap_2()
            .px_6()
            .py_2()
            .child(self.icon_button(
                "history-search",
                ("search", "Suchen"),
                ButtonStyle::Quiet,
                self.message_available(MessageFeature::Search),
                LocalAction::Message(Action::Search),
                cx,
            ))
            .child(self.icon_button(
                "history-files",
                ("file", "Dateien"),
                ButtonStyle::Quiet,
                self.message_available(MessageFeature::Files),
                LocalAction::Message(Action::Files),
                cx,
            ))
            .child(self.icon_button(
                "history-pins",
                ("pin", "Angeheftete Nachrichten"),
                ButtonStyle::Quiet,
                self.message_available(MessageFeature::Pins),
                LocalAction::Message(Action::Pins),
                cx,
            ))
            .child(self.icon_button(
                "history-unread",
                ("inbox", "Als ungelesen markieren"),
                ButtonStyle::Quiet,
                Availability::available(),
                LocalAction::Message(Action::Command(MessageAction::MarkUnread)),
                cx,
            ));
        body = body.child(toolbar);
        if self.message_ui.show_pins {
            let mut pins = div()
                .id("pinned-message-list")
                .flex_none()
                .max_h(px(200.))
                .overflow_y_scroll()
                .px_6()
                .py_2();
            if let Some(panel) = &self.snapshot.message_panel {
                for message in panel.pinned_messages.as_slice() {
                    pins = pins.child(
                        div()
                            .flex()
                            .gap_2()
                            .items_center()
                            .child(
                                self.button(
                                    ("pinned-message", message.local_id as usize),
                                    message.body.as_str().to_owned(),
                                    ButtonStyle::Quiet,
                                    true,
                                    LocalAction::Message(Action::Command(MessageAction::Context(
                                        message.local_id,
                                    ))),
                                    cx,
                                )
                                .flex_1(),
                            )
                            .child(self.button(
                                ("unpin-message", message.local_id as usize),
                                "Lösen",
                                ButtonStyle::Quiet,
                                self.message_available(MessageFeature::Pins),
                                LocalAction::Message(Action::Command(MessageAction::Unpin(
                                    message.local_id,
                                ))),
                                cx,
                            )),
                    );
                }
                if panel.pinned_messages.as_slice().is_empty() {
                    pins = pins.child(self.hint(self.l("Keine angehefteten Nachrichten geladen.")));
                }
            }
            body = body.child(pins);
        }
        if self.message_ui.show_search {
            body = body.child(
                div()
                    .px_6()
                    .py_2()
                    .flex()
                    .gap_2()
                    .child(self.message_ui.search.clone())
                    .child(self.button(
                        "history-search-submit",
                        "Suchen",
                        ButtonStyle::Secondary,
                        self.message_available(MessageFeature::Search),
                        LocalAction::Message(Action::Search),
                        cx,
                    )),
            );
            if let Some(panel) = &self.snapshot.message_panel {
                let mut results = div()
                    .flex_none()
                    .max_h(px(180.))
                    .id("message-search-results")
                    .overflow_y_scroll()
                    .px_6();
                for hit in panel.search.as_slice() {
                    results = results.child(
                        self.control(
                            ("message-search", hit.id as usize),
                            ButtonStyle::Quiet,
                            true,
                            LocalAction::Message(Action::Command(MessageAction::Context(hit.id))),
                            cx,
                        )
                        .w_full()
                        .justify_start()
                        .child(
                            div()
                                .flex()
                                .flex_col()
                                .items_start()
                                .child(hit.title.as_str().to_owned())
                                .child(self.hint(hit.excerpt.as_str())),
                        ),
                    );
                }
                if panel.search_loaded && panel.search.as_slice().is_empty() {
                    results = results.child(self.hint(self.l("Keine Nachrichten gefunden.")));
                }
                if panel.search_more {
                    results = results.child(self.button(
                        "search-next",
                        "Weitere Treffer",
                        ButtonStyle::Secondary,
                        self.message_available(MessageFeature::Search),
                        LocalAction::Message(Action::Command(MessageAction::SearchNext)),
                        cx,
                    ));
                }
                body = body.child(results);
            }
        }
        if self
            .snapshot
            .message_panel
            .as_ref()
            .is_some_and(|p| p.older.is_some())
        {
            body = body.child(self.button(
                "load-older",
                "Ältere Nachrichten laden",
                ButtonStyle::Quiet,
                true,
                LocalAction::Message(Action::Command(MessageAction::Older)),
                cx,
            ));
        }
        match self.snapshot.content {
            ContentState::Loading => {
                body = body.child(self.state_note(
                    "chat",
                    "Nachrichten werden geladen …",
                    "Du kannst währenddessen ein anderes Gespräch öffnen.",
                ))
            }
            ContentState::Denied => {
                body = body.child(self.state_note(
                    "lock",
                    "Kein Zugriff auf dieses Gespräch.",
                    "Deine Berechtigungen können sich geändert haben.",
                ))
            }
            ContentState::Failed | ContentState::Unconfigured => {
                body = body
                    .child(self.state_note(
                        "chat",
                        "Nachrichten konnten nicht aktualisiert werden.",
                        "Dein Entwurf bleibt erhalten.",
                    ))
                    .child(self.button(
                        "retry-content",
                        "Erneut laden",
                        ButtonStyle::Secondary,
                        true,
                        LocalAction::Message(Action::Command(MessageAction::Refresh)),
                        cx,
                    ))
            }
            _ => {
                if self.snapshot.messages.as_slice().is_empty() {
                    body = body.child(div().flex_1().child(self.state_note(
                        "chat",
                        "Noch keine Nachrichten.",
                        "Hier erscheinen die Nachrichten dieses Gesprächs.",
                    )));
                } else {
                    body = body.child(
                        div()
                            .flex_1()
                            .min_h_0()
                            .track_focus(&self.navigation.history_focus())
                            .child(
                                list(
                                    self.message_ui.list.clone(),
                                    cx.processor(|this, index, _, cx| {
                                        this.snapshot
                                            .messages
                                            .as_slice()
                                            .get(index)
                                            .map(|m| this.message_row(m, cx))
                                            .unwrap_or_else(div)
                                            .into_any_element()
                                    }),
                                )
                                .size_full(),
                            ),
                    );
                }
            }
        }
        if let Some(panel) = &self.snapshot.message_panel {
            if let talk_core::ui::conversation::RemoteState::Failed(_, reason) = panel.state {
                body = body.child(div().px_6().py_2().text_color(t.danger).child(self.l(
                    match reason {
                        talk_core::ui::UnavailableReason::Unsupported => {
                            "Der Server unterstützt diese Funktion nicht."
                        }
                        talk_core::ui::UnavailableReason::Unconfigured => {
                            "In diesem Client noch nicht verfügbar."
                        }
                        talk_core::ui::UnavailableReason::PermissionDenied => {
                            "Du hast dafür keine Berechtigung."
                        }
                        _ => {
                            "Die Anfrage ist fehlgeschlagen. Der bestätigte Stand bleibt erhalten."
                        }
                    },
                )));
            }
            if matches!(
                panel.state,
                talk_core::ui::conversation::RemoteState::Uncertain(_)
            ) {
                body = body
                    .child(self.hint(self.l("Ergebnis unklar. Vor erneutem Ändern neu laden.")));
            }
        }
        let _ = row;
        body
    }
    fn message_row(&self, m: &MessageView, cx: &Context<Self>) -> Div {
        let t = self.theme;
        let context = self.snapshot.selected.expect("message context");
        if m.metadata.system {
            return div().child(
                div()
                    .id(scoped_id("message", context, m.local_id, 0))
                    .px_6()
                    .py_2()
                    .flex()
                    .justify_center()
                    .gap_2()
                    .text_color(t.muted)
                    .text_size(self.sp(12.))
                    .child(m.body.as_str().to_owned())
                    .child(self.hint(m.time_label.as_str())),
            );
        }
        let mut content = div()
            .flex_1()
            .min_w_0()
            .flex()
            .flex_col()
            .gap_2()
            .child(
                div()
                    .flex()
                    .items_baseline()
                    .gap_2()
                    .child(
                        div()
                            .text_size(self.sp(13.))
                            .font_weight(FontWeight::BOLD)
                            .child(m.author.as_str().to_owned()),
                    )
                    .when(m.own, |d| d.child(self.hint(self.l("Du"))))
                    .child(self.hint(m.time_label.as_str()))
                    .when(m.metadata.edited, |d| {
                        d.child(self.hint(self.l("Bearbeitet")))
                    }),
            )
            .when_some(m.metadata.parent.as_ref(), |d, parent| {
                d.child(
                    div()
                        .border_l_2()
                        .border_color(t.accent)
                        .pl_3()
                        .child(self.hint(if parent.unavailable {
                            self.l("Ursprüngliche Nachricht nicht mehr verfügbar.")
                        } else {
                            parent.author.as_str()
                        }))
                        .when(!parent.unavailable, |d| {
                            d.child(self.hint(parent.text.as_str()))
                        }),
                )
            });
        if m.metadata.deleted {
            content = content.child(self.hint(self.l("Nachricht gelöscht.")));
        } else {
            content = content.child(self.rich_text(m.body.as_str()));
        }
        for object in m.metadata.rich.as_slice().iter().filter(|r| {
            !matches!(
                r.kind.as_str(),
                "user" | "guest" | "group" | "email" | "call"
            )
        }) {
            content = content.child(
                div()
                    .p_3()
                    .rounded_lg()
                    .bg(t.subtle)
                    .child(object.name.as_str().to_owned())
                    .child(self.hint(self.l(match object.kind.as_str() {
                        "file" => "Datei",
                        "poll" => "Umfrage",
                        _ => "Anhang",
                    }))),
            );
        }
        if !m.metadata.reactions.as_slice().is_empty() {
            content = content.child(
                div().flex().flex_wrap().gap_2().children(
                    m.metadata
                        .reactions
                        .as_slice()
                        .iter()
                        .enumerate()
                        .map(|(i, r)| {
                            self.button(
                                scoped_id("reaction", context, m.local_id, i as u128),
                                format!("{} {}", r.emoji.as_str(), r.count),
                                if r.own {
                                    ButtonStyle::Primary
                                } else {
                                    ButtonStyle::Secondary
                                },
                                self.message_available(MessageFeature::React),
                                LocalAction::Message(Action::Command(MessageAction::React {
                                    message: m.local_id,
                                    emoji: r.emoji.clone(),
                                    add: !r.own,
                                })),
                                cx,
                            )
                        }),
                ),
            );
        }
        if m.metadata.truncated {
            content = content.child(self.hint(
                self.l("Lange Nachricht gekürzt angezeigt. Bearbeiten ist hier nicht verfügbar."),
            ));
        }
        if self.message_ui.menu == Some(m.local_id) && !m.metadata.deleted {
            let mut actions = div()
                .flex()
                .flex_wrap()
                .gap_2()
                .child(self.button(
                    scoped_id("reply", context, m.local_id, 0),
                    "Antworten",
                    ButtonStyle::Quiet,
                    Availability::when(
                        m.metadata.replyable,
                        "Auf diese Nachricht kann nicht geantwortet werden.",
                    ),
                    LocalAction::Message(Action::Reply(m.local_id)),
                    cx,
                ))
                .child(self.button(
                    scoped_id("react", context, m.local_id, 0),
                    "👍",
                    ButtonStyle::Quiet,
                    self.message_available(MessageFeature::React),
                    LocalAction::Message(Action::Command(MessageAction::React {
                        message: m.local_id,
                        emoji: Text::new("👍").expect("emoji"),
                        add: true,
                    })),
                    cx,
                ))
                .child(self.button(
                    scoped_id("edit", context, m.local_id, 0),
                    "Bearbeiten",
                    ButtonStyle::Quiet,
                    Availability::when(
                        m.metadata.edit == Decision::Allowed,
                        "Diese Nachricht kann nicht bearbeitet werden.",
                    ),
                    LocalAction::Message(Action::Edit(m.local_id)),
                    cx,
                ))
                .child(self.button(
                    scoped_id("delete", context, m.local_id, 0),
                    "Löschen",
                    ButtonStyle::Quiet,
                    Availability::when(
                        m.metadata.delete == Decision::Allowed,
                        "Diese Nachricht kann nicht gelöscht werden.",
                    ),
                    LocalAction::Message(Action::ConfirmDelete(m.local_id)),
                    cx,
                ))
                .child(self.button(
                    scoped_id("pin", context, m.local_id, 0),
                    "Anheften",
                    ButtonStyle::Quiet,
                    self.message_available(MessageFeature::Pins),
                    LocalAction::Message(Action::Command(MessageAction::Pin(m.local_id))),
                    cx,
                ))
                .child(self.button(
                    scoped_id("remind", context, m.local_id, 0),
                    "Später erinnern",
                    ButtonStyle::Quiet,
                    self.message_available(MessageFeature::Reminders),
                    LocalAction::Message(Action::Remind(m.local_id)),
                    cx,
                ))
                .child(self.button(
                    scoped_id("forward", context, m.local_id, 0),
                    "Weiterleiten",
                    ButtonStyle::Quiet,
                    self.message_available(MessageFeature::Forward),
                    LocalAction::Message(Action::Forward(m.local_id)),
                    cx,
                ));
            if m.metadata.replyable {
                actions = actions.child(self.button(
                    scoped_id("thread-open", context, m.local_id, 0),
                    "Thread öffnen",
                    ButtonStyle::Quiet,
                    self.message_available(MessageFeature::Threads),
                    LocalAction::Thread(context, m.metadata.thread.unwrap_or(m.local_id)),
                    cx,
                ));
            }
            if self.message_ui.confirm_delete == Some(m.local_id) {
                actions =
                    actions
                        .child(self.hint(self.l(
                            "Diese Nachricht für alle löschen? Geteilte Dateien bleiben erhalten.",
                        )))
                        .child(self.button(
                            "confirm-message-delete",
                            "Löschen bestätigen",
                            ButtonStyle::Danger,
                            true,
                            LocalAction::Message(Action::Delete(m.local_id)),
                            cx,
                        ));
            }
            content = content.child(actions);
            if self.message_ui.forward == Some(m.local_id) {
                let mut destinations = div()
                    .flex()
                    .flex_col()
                    .gap_2()
                    .p_3()
                    .bg(t.subtle)
                    .child(self.room_forward_hint(m));
                for room in self
                    .snapshot
                    .conversations
                    .as_slice()
                    .iter()
                    .filter(|r| {
                        r.context != context
                            && r.context.account == context.account
                            && r.send == Decision::Allowed
                    })
                    .take(20)
                {
                    destinations = destinations.child(self.button(
                        scoped_id("forward-to", room.context, m.local_id, 0),
                        room.title.as_str().to_owned(),
                        ButtonStyle::Secondary,
                        true,
                        LocalAction::Message(Action::ChooseForward(m.local_id, room.context)),
                        cx,
                    ));
                }
                if let Some(destination) = self.message_ui.forward_destination
                    && let Some(room) = self
                        .snapshot
                        .conversations
                        .as_slice()
                        .iter()
                        .find(|r| r.context == destination)
                {
                    destinations =
                        destinations
                            .child(self.hint(room.title.as_str()))
                            .child(self.button(
                                "confirm-forward",
                                "An dieses Gespräch senden",
                                ButtonStyle::Primary,
                                true,
                                LocalAction::Message(Action::ConfirmForward(
                                    m.local_id,
                                    destination,
                                )),
                                cx,
                            ));
                }
                content = content.child(destinations);
            }
        }
        div().child(
            div()
                .id(scoped_id("message", context, m.local_id, 0))
                .px_6()
                .py_3()
                .flex()
                .gap_3()
                .when(
                    self.highlighted_message == Some((context, m.local_id)),
                    |d| d.bg(t.selected),
                )
                .child(self.avatar(m.initials.as_str(), false))
                .child(content)
                .child(self.icon_button(
                    scoped_id("message-menu", context, m.local_id, 0),
                    ("more", "Nachrichtenaktionen"),
                    ButtonStyle::Quiet,
                    Availability::available(),
                    LocalAction::Message(Action::Menu(m.local_id)),
                    cx,
                )),
        )
    }
    pub(super) fn shared_files(&self, cx: &Context<Self>) -> Div {
        let mut body = div().flex().flex_col().gap_3();
        if let Some(panel) = &self.snapshot.message_panel {
            body = body.child(self.hint(&format!(
                "{} {} MiB",
                self.l("Maximale Dateigröße:"),
                panel.file_limit / 1024 / 1024
            )));
            for message in panel.shared_files.as_slice() {
                for object in message
                    .metadata
                    .rich
                    .as_slice()
                    .iter()
                    .filter(|r| r.kind.as_str() == "file")
                {
                    body = body.child(
                        div()
                            .flex()
                            .gap_3()
                            .items_center()
                            .child(div().flex_1().child(object.name.as_str().to_owned()))
                            .child(self.button(
                                ("download-shared", message.local_id as usize),
                                "Herunterladen",
                                ButtonStyle::Secondary,
                                self.message_available(MessageFeature::Files),
                                LocalAction::Message(Action::Command(MessageAction::Download {
                                    message: message.local_id,
                                    placeholder: object.placeholder.clone(),
                                    name: Text::new(object.name.as_str()).unwrap_or_else(|_| {
                                        Text::new("download").expect("constant")
                                    }),
                                })),
                                cx,
                            )),
                    );
                }
            }
            if panel.shared_files.as_slice().is_empty() {
                body = body.child(self.hint(self.l("Keine Dateien geladen")));
            }
        }
        body
    }
    fn room_forward_hint(&self, m: &MessageView) -> Div {
        div()
            .child(self.hint(self.l("Weiterleiten als neue Nachricht. Ziel auswählen:")))
            .child(self.hint(m.body.as_str()))
    }
    fn rich_text(&self, text: &str) -> Div {
        let mut body = div().flex().flex_col().gap_1().line_height(self.sp(22.));
        let mut code = false;
        // Bounded by message bytes; no HTML interpretation, remote embedding, or automatic link open.
        for line in text.split('\n').take(256) {
            if line.starts_with("```") {
                code = !code;
                continue;
            }
            let quote = line.strip_prefix("> ");
            body = body.child(
                div()
                    .when(code, |d| {
                        d.font_family("monospace").bg(self.theme.subtle).px_2()
                    })
                    .when(quote.is_some(), |d| {
                        d.pl_3().border_l_2().border_color(self.theme.border)
                    })
                    .child(quote.unwrap_or(line).to_owned()),
            );
        }
        body
    }
    pub(super) fn composer_context(&self, cx: &Context<Self>) -> Div {
        let mut body = div().flex().flex_col().gap_2();
        let Some(context) = self.snapshot.selected else {
            return body;
        };
        let target = self.drafts.target(context);
        if self.message_ui.editing.is_some() || target.reply_to.is_some() || target.thread.is_some()
        {
            let label = if self.message_ui.editing.is_some() {
                "Nachricht bearbeiten"
            } else if target.thread.is_some() {
                "Antwort im Thread"
            } else {
                "Antwort auf eine Nachricht"
            };
            body = body.child(
                div()
                    .flex()
                    .items_center()
                    .gap_2()
                    .child(self.hint(self.l(label)))
                    .child(self.button(
                        "cancel-compose-mode",
                        "Abbrechen",
                        ButtonStyle::Quiet,
                        true,
                        LocalAction::Message(Action::CancelCompose),
                        cx,
                    )),
            );
        }
        body = body.child(
            div()
                .flex()
                .flex_wrap()
                .gap_2()
                .child(self.icon_button(
                    "composer-mentions",
                    ("at", "@"),
                    ButtonStyle::Quiet,
                    self.message_available(MessageFeature::Mentions),
                    LocalAction::Message(Action::Command(MessageAction::Mentions(
                        Text::new("").expect("empty query"),
                    ))),
                    cx,
                ))
                .child(self.icon_button(
                    "composer-schedule",
                    ("clock", "Nachricht planen"),
                    ButtonStyle::Quiet,
                    self.message_available(MessageFeature::Schedules),
                    LocalAction::Message(Action::Schedule),
                    cx,
                ))
                .child(self.icon_button(
                    "composer-scheduled",
                    ("inbox", "Geplante Nachrichten"),
                    ButtonStyle::Quiet,
                    self.message_available(MessageFeature::Schedules),
                    LocalAction::Message(Action::Command(MessageAction::Scheduled)),
                    cx,
                ))
                .child(self.icon_button(
                    "composer-silent",
                    ("moon", "Leise senden"),
                    if target.silent {
                        ButtonStyle::Primary
                    } else {
                        ButtonStyle::Quiet
                    },
                    self.message_available(MessageFeature::Silent),
                    LocalAction::Message(Action::Silent),
                    cx,
                ))
                .child(self.icon_button(
                    "composer-reminders",
                    ("bell", "Erinnerungen"),
                    ButtonStyle::Quiet,
                    self.message_available(MessageFeature::Reminders),
                    LocalAction::Message(Action::Command(MessageAction::Reminders)),
                    cx,
                )),
        );
        if let Some(panel) = &self.snapshot.message_panel {
            if let Some(file) = &panel.uploaded {
                body = body.child(
                    div()
                        .p_3()
                        .rounded_lg()
                        .bg(self.theme.subtle)
                        .child(file.name.as_str().to_owned())
                        .child(self.hint(self.l(
                            "Hochgeladen. Erst beim Teilen wird die Datei im Gespräch sichtbar.",
                        )))
                        .child(self.button(
                            "share-upload",
                            "Datei mit diesem Gespräch teilen",
                            ButtonStyle::Primary,
                            self.message_available(MessageFeature::Files),
                            LocalAction::Message(Action::ShareUpload),
                            cx,
                        ))
                        .child(self.button(
                            "dismiss-upload",
                            "Vorschau schließen",
                            ButtonStyle::Quiet,
                            true,
                            LocalAction::Message(Action::Command(MessageAction::DismissUpload)),
                            cx,
                        )),
                );
            }
            if panel.file_saved.is_some() {
                body = body.child(self.hint(self.l("Datei gespeichert.")));
            }
            for (index, mention) in panel.mentions.as_slice().iter().take(8).enumerate() {
                body = body.child(self.button(
                    ("mention-choice", index),
                    mention.label.as_str().to_owned(),
                    ButtonStyle::Quiet,
                    true,
                    LocalAction::Message(Action::InsertMention(
                        mention.identifier.as_str().to_owned(),
                    )),
                    cx,
                ));
            }
            for scheduled in panel.scheduled.as_slice().iter().take(8) {
                body = body.child(
                    div()
                        .flex()
                        .items_center()
                        .gap_2()
                        .child(self.hint(scheduled.body.as_str()))
                        .child(self.button(
                            ("schedule-cancel", scheduled.id as usize),
                            "Planung abbrechen",
                            ButtonStyle::Quiet,
                            self.message_available(MessageFeature::Schedules),
                            LocalAction::Message(Action::Command(MessageAction::CancelScheduled(
                                scheduled.id,
                            ))),
                            cx,
                        )),
                );
            }
            for reminder in panel.reminders.as_slice().iter().take(8) {
                body = body.child(
                    div()
                        .flex()
                        .items_center()
                        .gap_2()
                        .child(self.hint(self.l("Erinnerung auf dem Server bestätigt.")))
                        .child(self.button(
                            ("reminder-cancel", reminder.message as usize),
                            "Erinnerung löschen",
                            ButtonStyle::Quiet,
                            self.message_available(MessageFeature::Reminders),
                            LocalAction::Message(Action::Command(MessageAction::Reminder {
                                message: reminder.message,
                                timestamp: None,
                            })),
                            cx,
                        )),
                );
            }
        }
        if self.message_ui.time_target.is_some() {
            body = body
                .child(self.hint(self.l("Datum, Uhrzeit und UTC-Versatz: JJJJ-MM-TT HH:MM +HH:MM")))
                .child(self.message_ui.time_input.clone())
                .child(self.button(
                    "submit-message-time",
                    "Auf dem Server speichern",
                    ButtonStyle::Secondary,
                    true,
                    LocalAction::Message(Action::SubmitTime),
                    cx,
                ))
                .when(self.message_ui.time_error, |d| {
                    d.child(self.hint(self.l("Datum und Zeitzonenversatz sind ungültig.")))
                });
        }
        body
    }
}
/// Explicit UTC offset removes ambiguous daylight-saving interpretation. No local timer sends content.
fn parse_time(value: &str) -> Option<u64> {
    if value.len() != 23 {
        return None;
    }
    let bytes = value.as_bytes();
    if bytes[4] != b'-'
        || bytes[7] != b'-'
        || bytes[10] != b' '
        || bytes[13] != b':'
        || bytes[16] != b' '
        || bytes[20] != b':'
    {
        return None;
    }
    let num = |range: std::ops::Range<usize>| value.get(range)?.parse::<i64>().ok();
    let y = num(0..4)?;
    let m = num(5..7)?;
    let d = num(8..10)?;
    let h = num(11..13)?;
    let min = num(14..16)?;
    let oh = num(18..20)?;
    let om = num(21..23)?;
    let leap = y % 4 == 0 && (y % 100 != 0 || y % 400 == 0);
    let days = match m {
        1 | 3 | 5 | 7 | 8 | 10 | 12 => 31,
        4 | 6 | 9 | 11 => 30,
        2 => {
            if leap {
                29
            } else {
                28
            }
        }
        _ => return None,
    };
    if !(1970..=2100).contains(&y) || d < 1 || d > days || h > 23 || min > 59 || oh > 14 || om > 59
    {
        return None;
    }
    let sign = match bytes[17] {
        b'+' => 1,
        b'-' => -1,
        _ => return None,
    };
    let yr = y - i64::from(m <= 2);
    let era = yr.div_euclid(400);
    let yoe = yr - era * 400;
    let mp = m + if m > 2 { -3 } else { 9 };
    let doy = (153 * mp + 2) / 5 + d - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    let epoch =
        (era * 146097 + doe - 719468) * 86400 + h * 3600 + min * 60 - sign * (oh * 3600 + om * 60);
    u64::try_from(epoch).ok()
}
#[cfg(test)]
mod tests {
    use super::parse_time;
    #[test]
    fn explicit_offset_resolves_daylight_saving_and_rejects_bad_dates() {
        assert_eq!(
            parse_time("2026-10-25 02:30 +02:00").unwrap() + 3600,
            parse_time("2026-10-25 02:30 +01:00").unwrap()
        );
        assert!(parse_time("2026-02-29 09:00 +01:00").is_none());
        assert!(parse_time("2028-02-29 09:00 +01:00").is_some());
        assert!(parse_time("2026-10-25 02:30").is_none());
    }
}
