//! Stable, variable-height native history and deliberate message actions.
mod arrival;

use super::{
    ButtonStyle, Desktop, LocalAction, Overlay,
    controls::{Availability, scoped_id},
    input::TextInput,
};
use gpui::{
    Context, Div, Entity, FocusHandle, Focusable, FontWeight, ListAlignment, ListOffset, ListState,
    Window, div, list, prelude::*, px,
};
use std::{cell::RefCell, rc::Rc, time::Instant};
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
    ToggleSearch,
    CloseSearch,
    Locate(u128),
    Schedule,
    Remind(u128),
    SubmitTime,
    InsertMention(String),
}
pub(super) struct MessageUi {
    pub list: ListState,
    context: Option<ConversationRef>,
    observed: Vec<MessageView>,
    arrival: Rc<RefCell<arrival::Arrival>>,
    geometry: Rc<RefCell<MessageGeometry>>,
    pub editing: Option<u128>,
    pub(super) pending_edit: Option<RequestId>,
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
    search_return_focus: Option<FocusHandle>,
    search_pending_focus: bool,
    show_pins: bool,
    mention_candidate: Option<(ConversationRef, String, Instant)>,
    mention_request: Option<(RequestId, String)>,
    mention_ready: Option<String>,
}

#[derive(Default)]
struct MessageGeometry {
    // Only this page's painted rows, drained after prepaint. GPUI's Bottom list
    // exposes an end sentinel instead of the actual visible first-row offset.
    painted: Vec<(usize, u128, gpui::Bounds<gpui::Pixels>)>,
    anchor: Option<(u128, gpui::Pixels)>,
    header_inset: gpui::Pixels,
}

fn preserved_row_offset(
    offset: gpui::Pixels,
    was_first: bool,
    is_first: bool,
    header_inset: gpui::Pixels,
) -> gpui::Pixels {
    offset + header_inset * (f32::from(u8::from(is_first)) - f32::from(u8::from(was_first)))
}

impl MessageUi {
    fn visible_bounds(&self) -> gpui::Bounds<gpui::Pixels> {
        let bounds = self.list.viewport_bounds();
        let inset = self.geometry.borrow().header_inset.min(bounds.size.height);
        gpui::Bounds::from_corners(
            gpui::point(bounds.left(), bounds.top() + inset),
            bounds.bottom_right(),
        )
    }

    pub fn new(cx: &mut Context<Desktop>) -> Self {
        let theme = super::theme::Theme::resolve(
            super::theme::Palette::Blue,
            super::theme::Appearance::Light,
            gpui::WindowAppearance::Light,
        );
        let arrival = Rc::new(RefCell::new(arrival::Arrival::default()));
        let scroll_arrival = arrival.clone();
        let list = ListState::new(0, ListAlignment::Bottom, px(300.));
        list.set_scroll_handler(move |event, _, _| {
            if event.is_scrolled {
                scroll_arrival.borrow_mut().manual_scroll();
            }
        });
        Self {
            list,
            arrival,
            geometry: Rc::new(RefCell::new(MessageGeometry::default())),
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
            search_return_focus: None,
            search_pending_focus: false,
            show_pins: false,
            mention_candidate: None,
            mention_request: None,
            mention_ready: None,
        }
    }
}
impl Desktop {
    /// Reuse the composition owner's finite refresh tick for one debounced
    /// mention query. No per-keystroke task or unbounded request queue.
    pub(super) fn refresh_composer_mentions(&mut self, cx: &Context<Self>) {
        let current = self
            .snapshot
            .selected
            .filter(|_| {
                self.composer_active
                    && self.composer_available()
                    && self.message_feature_enabled(MessageFeature::Mentions)
            })
            .and_then(|context| {
                self.composer
                    .read(cx)
                    .mention_query()
                    .map(|q| (context, q.to_owned()))
            });
        let changed = self
            .message_ui
            .mention_candidate
            .as_ref()
            .map(|(c, q, _)| (*c, q.as_str()))
            != current.as_ref().map(|(c, q)| (*c, q.as_str()));
        if changed {
            self.message_ui.mention_candidate = current.map(|(c, q)| (c, q, Instant::now()));
            self.message_ui.mention_request = None;
            self.message_ui.mention_ready = None;
        }
        let Some((_, query, since)) = &self.message_ui.mention_candidate else {
            return;
        };
        if since.elapsed() < std::time::Duration::from_millis(200)
            || self.message_ui.mention_request.is_some()
            || self
                .snapshot
                .message_panel
                .as_ref()
                .is_none_or(|p| p.state.pending())
        {
            return;
        }
        let query = query.clone();
        if let Ok(bounded) = Text::new(&query)
            && let Some(request) = self.submit_message(MessageAction::Mentions(bounded))
        {
            self.message_ui.mention_request = Some((request, query));
        }
    }
    fn timeline_at_bottom(&self) -> bool {
        let count = self.message_ui.list.item_count();
        if count == 0 {
            return true;
        }
        if self.message_ui.list.logical_scroll_top().item_ix >= count {
            return true;
        }
        let viewport = self.message_ui.list.viewport_bounds();
        viewport.size.height > px(0.)
            && self
                .message_ui
                .list
                .bounds_for_item(count - 1)
                .is_some_and(|bounds| bounds.bottom() <= viewport.bottom() + px(1.))
    }

    pub(super) fn prepare_message_motion(&self, window: &Window) {
        let call_host = self.snapshot.call.as_ref().filter(|call| {
            call.stage != super::CallStage::Incoming
                && !(matches!(
                    call.stage,
                    super::CallStage::Ended | super::CallStage::Failed
                ) && self.dismissed_terminal_call == Some(call.scope))
        });
        let visible = !self.update_restart_pending
            && !self.projection_blocked
            && window.is_window_active()
            && self.snapshot.selected.is_some()
            && self.snapshot.content == ContentState::Ready
            && self.prejoin.is_none()
            && !self.incoming_visible()
            && !self.onboarding_open
            && !self.navigation_open
            && matches!(self.overlay, None | Some(Overlay::CallChat))
            && if self.overlay == Some(Overlay::CallChat) {
                true
            } else if let Some(call) = call_host {
                !matches!(
                    call.stage,
                    super::CallStage::Ended | super::CallStage::Failed
                ) && super::docked_call_chat(
                    window.viewport_size().width >= px(1000.) && self.scale < 150,
                    self.call_chat_open,
                    self.overlay,
                )
            } else {
                self.destination != super::SidebarDestination::Inbox
            };
        self.message_ui
            .arrival
            .borrow_mut()
            .visibility(visible, self.reduced_motion);
    }

    pub(super) fn trace_message_motion(&self) {
        println!(
            "UI_MESSAGE_MOTION reduced={} following={} anchor={:?} at_bottom={} counters={:?} animation_frames={}",
            self.reduced_motion,
            self.message_ui.arrival.borrow().following(),
            self.message_ui.list.logical_scroll_top(),
            self.timeline_at_bottom(),
            self.message_ui.arrival.borrow().diagnostic(),
            self.motion_frames.get(),
        );
    }

    pub(super) fn refresh_message_ui(&mut self, cx: &mut Context<Self>) {
        let rows = self.snapshot.messages.as_slice();
        if self.message_ui.context != self.snapshot.selected {
            self.message_ui.arrival.borrow_mut().reset();
            *self.message_ui.geometry.borrow_mut() = MessageGeometry::default();
        }
        // Explicit history/context loads establish a baseline, even when the
        // previously loaded conversation remains visible during the request.
        let live = self.snapshot.content == ContentState::Ready
            && self.snapshot.message_panel.as_ref().is_none_or(|panel| {
                panel.history == talk_core::ui::conversation::RemoteState::Idle
                    && panel
                        .located
                        .is_none_or(|(request, _)| self.message_ui.last_located == Some(request))
            });
        self.message_ui.arrival.borrow_mut().observe(
            &self.message_ui.observed,
            rows,
            live,
            self.timeline_at_bottom(),
            self.reduced_motion,
            Instant::now(),
        );
        if self.message_ui.context != self.snapshot.selected {
            if self.message_ui.context.is_some() && self.overlay == Some(Overlay::Files) {
                self.overlay = None;
                self.return_focus = None;
                self.pending_modal_focus = false;
            }
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
            self.message_ui.search_return_focus = None;
            self.message_ui.search_pending_focus = false;
            self.message_ui.search.update(cx, |i, _| i.reset());
            self.message_ui.show_pins = false;
            self.message_ui.last_located = None;
        } else if self.message_ui.observed != rows {
            let anchor = self.message_ui.list.logical_scroll_top();
            let (id, offset) = self
                .message_ui
                .observed
                .get(anchor.item_ix)
                .map(|m| (Some(m.local_id), anchor.offset_in_item))
                .or_else(|| {
                    self.message_ui
                        .geometry
                        .borrow()
                        .anchor
                        .map(|(id, offset)| (Some(id), offset))
                })
                .unwrap_or((None, anchor.offset_in_item));
            self.message_ui
                .list
                .splice(0..self.message_ui.observed.len(), rows.len());
            if let Some(index) = id.and_then(|id| rows.iter().position(|m| m.local_id == id)) {
                let was_first = self
                    .message_ui
                    .observed
                    .first()
                    .is_some_and(|row| Some(row.local_id) == id);
                self.message_ui.list.scroll_to(ListOffset {
                    item_ix: index,
                    offset_in_item: preserved_row_offset(
                        offset,
                        was_first,
                        index == 0,
                        self.message_ui.geometry.borrow().header_inset,
                    ),
                });
            }
            self.message_ui.observed = rows.to_vec();
        }
        if let Some((request, id)) = self
            .snapshot
            .message_panel
            .as_ref()
            .filter(|p| Some(p.context) == self.snapshot.selected)
            .and_then(|p| p.located)
            && self.message_ui.last_located != Some(request)
            && let Some(index) = rows.iter().position(|m| m.local_id == id)
        {
            self.message_ui.arrival.borrow_mut().manual_scroll();
            self.message_ui.list.scroll_to(ListOffset {
                item_ix: index,
                offset_in_item: if index == 0 {
                    px(0.)
                } else {
                    -self.message_ui.geometry.borrow().header_inset
                },
            });
            self.message_ui.last_located = Some(request);
            self.highlighted_message = self.snapshot.selected.map(|context| (context, id));
        }
        if let Some((request, completion)) = self
            .snapshot
            .message_panel
            .as_ref()
            .filter(|p| Some(p.context) == self.snapshot.selected)
            .and_then(|p| p.result)
        {
            if let Some((pending, query)) = &self.message_ui.mention_request
                && *pending == request
                && completion == Completion::ServerAccepted
            {
                self.message_ui.mention_ready = Some(query.clone());
            }
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
        if self.update_restart_pending
            || !window.is_window_active()
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
        if !self.snapshot.message_panel.as_ref().is_some_and(|p| {
            p.context == context && p.availability(MessageFeature::ReadReceipts).enabled()
        }) {
            return;
        }
        let viewport = self.message_ui.visible_bounds();
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
    pub(super) fn message_available(&self, feature: MessageFeature) -> Availability {
        let value = self
            .snapshot
            .message_panel
            .as_ref()
            .filter(|p| Some(p.context) == self.snapshot.selected)
            .map_or(talk_core::ui::conversation::Availability::UNKNOWN, |p| {
                p.availability(feature)
            });
        Availability::when(
            value.enabled() && self.snapshot.content == ContentState::Ready,
            if self.snapshot.content != ContentState::Ready {
                "Die Verbindung ist derzeit nicht bereit."
            } else {
                super::conversation_dialog::unavailable(value)
            },
        )
    }
    fn message_feature_enabled(&self, feature: MessageFeature) -> bool {
        self.snapshot.content == ContentState::Ready
            && self.snapshot.message_panel.as_ref().is_some_and(|panel| {
                Some(panel.context) == self.snapshot.selected
                    && panel.availability(feature).enabled()
            })
    }
    pub(super) fn conversation_search_open(&self) -> bool {
        self.message_ui.show_search && self.message_ui.context == self.snapshot.selected
    }
    pub(super) fn trace_chat_ui(&self, window: &Window, cx: &Context<Self>) {
        println!(
            "UI_CHAT_HEADER inset={:?} list={:?} visible={:?}",
            self.message_ui.geometry.borrow().header_inset,
            self.message_ui.list.viewport_bounds(),
            self.message_ui.visible_bounds(),
        );
        println!(
            "UI_CHAT_STATE search_open={} search_focused={} search_chars={} search_bounds={:?} composer_bounds={:?} reply={:?} menu={:?} reduced_motion={}",
            self.conversation_search_open(),
            self.message_ui.search.focus_handle(cx).is_focused(window),
            self.message_ui.search.read(cx).content.len(),
            self.message_ui.search.read(cx).geometry(),
            self.composer.read(cx).geometry(),
            self.snapshot
                .selected
                .and_then(|c| self.drafts.target(c).reply_to),
            self.message_ui.menu,
            self.reduced_motion,
        );
    }
    pub(super) fn finish_search_focus(&mut self, window: &mut Window, cx: &Context<Self>) {
        if self.message_ui.search_pending_focus
            && self.conversation_search_open()
            && self.composer_available()
        {
            self.message_ui.search_pending_focus = false;
            self.message_ui.search.focus_handle(cx).focus(window);
            self.navigation.reveal.request();
        }
    }
    fn toggle_conversation_search(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if !self.composer_available()
            || !self.message_feature_enabled(MessageFeature::Search)
            || self.input_composing(cx)
        {
            return;
        }
        if self.conversation_search_open()
            && self.message_ui.search.focus_handle(cx).is_focused(window)
        {
            self.close_conversation_search(window, cx);
            return;
        }
        if !self.conversation_search_open() {
            self.message_ui.search_return_focus = window.focused(cx);
        }
        // The call remains connected. Reveal its existing chat surface when a
        // current-room search is invoked during a call.
        if let Some(call) = self.snapshot.call.as_ref().filter(|call| {
            call.stage != super::CallStage::Incoming
                && self.dismissed_terminal_call != Some(call.scope)
        }) {
            if matches!(
                call.stage,
                super::CallStage::Ended | super::CallStage::Failed
            ) || !self.snapshot.selected.is_some_and(|context| {
                context.account == call.scope.account
                    && context.conversation == call.scope.conversation
            }) {
                return;
            }
            if window.viewport_size().width < px(1000.) || self.scale >= 150 {
                if self.overlay.is_none() {
                    self.handle(LocalAction::Open(Overlay::CallChat), window, cx);
                }
            } else {
                self.call_chat_open = true;
            }
        }
        self.message_ui.show_search = true;
        self.message_ui.search_pending_focus = true;
        self.message_ui.search.focus_handle(cx).focus(window);
    }
    pub(super) fn close_conversation_search(&mut self, window: &mut Window, cx: &Context<Self>) {
        self.message_ui.show_search = false;
        self.message_ui.search_pending_focus = false;
        if let Some(focus) = self
            .message_ui
            .search_return_focus
            .take()
            .filter(|focus| self.root_focus.contains(focus, window))
        {
            focus.focus(window);
        } else if let Some(focus) = self.control_focus.current(&"history-search".into()) {
            focus.focus(window);
        } else if self.composer_available() {
            self.composer.focus_handle(cx).focus(window);
        } else {
            self.root_focus.focus(window);
        }
    }
    fn can_reply_to(&self, message: &MessageView) -> bool {
        reply_allowed(
            message,
            self.snapshot.content,
            self.selected().map_or(Decision::Unknown, |row| row.send),
            self.message_feature_enabled(MessageFeature::Reply),
            self.snapshot
                .selected
                .is_some_and(|c| self.drafts.pending(c))
                || self.message_ui.pending_edit.is_some(),
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
        let prior_compose = self.compose_motion_id(context);
        match action {
            Action::Silent => {
                let mut target = self.drafts.target(context);
                target.silent = !target.silent;
                self.drafts.set_target(context, target);
            }
            Action::Pins => {
                self.message_ui.show_pins = true;
                if self.message_feature_enabled(MessageFeature::Pins) {
                    self.submit_message(MessageAction::Pins);
                }
            }
            Action::Files => {
                self.message_ui.show_pins = false;
                if self.message_feature_enabled(MessageFeature::Files) {
                    self.submit_message(MessageAction::Files);
                }
                if self.overlay != Some(Overlay::Files) {
                    self.handle(LocalAction::Open(Overlay::Files), window, cx);
                }
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
                if !self
                    .snapshot
                    .messages
                    .as_slice()
                    .iter()
                    .any(|m| m.local_id == id && message_actions_allowed(m))
                {
                    return;
                }
                self.message_ui.menu = if self.message_ui.menu == Some(id) {
                    None
                } else {
                    Some(id)
                };
                if self.message_ui.menu.is_some() {
                    self.begin_surface_reveal(scoped_id("menu-reveal", context, id, 0));
                }
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
                        .any(|m| m.local_id == id && self.can_reply_to(m))
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
                if self.conversation_search_open()
                    && self.message_feature_enabled(MessageFeature::Search)
                    && !self.message_ui.search.read(cx).composing()
                    && let Ok(query) = Text::new(self.message_ui.search.read(cx).content.as_str())
                    && !query.as_str().trim().is_empty()
                {
                    self.submit_message(MessageAction::Search(query));
                }
            }
            Action::ToggleSearch => self.toggle_conversation_search(window, cx),
            Action::CloseSearch => self.close_conversation_search(window, cx),
            Action::Locate(id) => {
                if self.submit_message(MessageAction::Context(id)).is_some()
                    && self.overlay == Some(Overlay::Files)
                {
                    self.handle(LocalAction::Close, window, cx);
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
                self.composer
                    .update(cx, |i, cx| i.insert_mention(&identifier, cx));
                self.composer.focus_handle(cx).focus(window);
            }
        }
        if let Some(id) = self.compose_motion_id(context)
            && Some(&id) != prior_compose.as_ref()
        {
            self.begin_surface_reveal(id);
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
    pub(super) fn conversation_search(&self, cx: &Context<Self>) -> Div {
        let Some(context) = self
            .snapshot
            .selected
            .filter(|_| self.conversation_search_open())
        else {
            return div();
        };
        let t = self.theme;
        let field = div()
            .flex_none()
            .flex()
            .items_center()
            .gap_2()
            .px_4()
            .py_2()
            .child(self.scaled_icon("search", 18., t.muted))
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .child(self.message_ui.search.clone()),
            )
            .child(self.button(
                "history-search-submit",
                "Suchen",
                ButtonStyle::Secondary,
                self.message_available(MessageFeature::Search),
                LocalAction::MessageIn(context, Action::Search),
                cx,
            ))
            .child(self.icon_button(
                "history-search-close",
                ("x", "Gesprächssuche schließen"),
                ButtonStyle::Quiet,
                Availability::available(),
                LocalAction::MessageIn(context, Action::CloseSearch),
                cx,
            ));
        let mut body = div()
            .id(scoped_id("conversation-search", context, 0, 0))
            .flex_none()
            .border_b_1()
            .border_color(t.border)
            .on_key_down(
                cx.listener(move |this, event: &gpui::KeyDownEvent, window, cx| {
                    if Some(context) == this.snapshot.selected
                        && event.keystroke.key == "enter"
                        && !event.keystroke.modifiers.modified()
                        && this.message_ui.search.focus_handle(cx).is_focused(window)
                        && !this.message_ui.search.read(cx).composing()
                    {
                        this.handle(LocalAction::MessageIn(context, Action::Search), window, cx);
                        cx.stop_propagation();
                    }
                }),
            )
            .child(self.reveal("search-reveal", field));
        if let Some(panel) = self
            .snapshot
            .message_panel
            .as_ref()
            .filter(|p| p.context == context)
        {
            let mut results = div()
                .flex_none()
                .max_h(self.sp(140.).min(px(240.)))
                .id(scoped_id("message-search-results", context, 0, 0))
                .overflow_y_scroll()
                .px_4();
            for hit in panel.search.as_slice() {
                results = results.child(
                    self.control(
                        scoped_id("message-search", context, hit.id, 0),
                        ButtonStyle::Quiet,
                        true,
                        LocalAction::MessageIn(
                            context,
                            Action::Command(MessageAction::Context(hit.id)),
                        ),
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
                    LocalAction::MessageIn(context, Action::Command(MessageAction::SearchNext)),
                    cx,
                ));
            }
            body = body.child(results);
        }
        div().flex_none().child(body)
    }
    pub(super) fn message_timeline(&self, constrained: bool, cx: &Context<Self>) -> Div {
        self.message_timeline_with_header(constrained, None, cx)
    }
    pub(super) fn conversation_timeline(
        &self,
        header: Div,
        constrained: bool,
        cx: &Context<Self>,
    ) -> Div {
        if matches!(
            self.snapshot.content,
            ContentState::Ready | ContentState::Offline
        ) && !self.snapshot.messages.as_slice().is_empty()
        {
            self.message_timeline_with_header(constrained, Some(header), cx)
        } else {
            div()
                .flex_1()
                .min_h_0()
                .flex()
                .flex_col()
                .when(constrained, |d| d.flex_none())
                .child(header)
                .child(self.message_timeline(constrained, cx))
        }
    }
    fn message_timeline_with_header(
        &self,
        constrained: bool,
        mut header: Option<Div>,
        cx: &Context<Self>,
    ) -> Div {
        let t = self.theme;
        let mut body = div()
            .flex_1()
            .min_h_0()
            .flex()
            .flex_col()
            .when(constrained, |d| d.flex_none())
            .when(constrained && header.is_none(), |d| d.h(px(200.)));
        if self.selected().is_none() {
            return body;
        }
        if self
            .snapshot
            .message_panel
            .as_ref()
            .is_some_and(|p| p.older.is_some())
        {
            let older = self.button(
                "load-older",
                "Ältere Nachrichten laden",
                ButtonStyle::Quiet,
                true,
                LocalAction::Message(Action::Command(MessageAction::Older)),
                cx,
            );
            if let Some(top) = header.take() {
                header = Some(top.child(older.bg(t.surface)));
            } else {
                body = body.child(older);
            }
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
                    body = body.child(if let Some(header) = header.take() {
                        super::shell::chat_header::scroll_under_header(
                            header,
                            constrained,
                            cx.processor(|this, inset, _, cx| this.message_history(inset, cx)),
                        )
                        .into_any_element()
                    } else {
                        self.message_history(px(0.), cx).into_any_element()
                    });
                }
            }
        }
        if let Some(panel) = self
            .snapshot
            .message_panel
            .as_ref()
            .filter(|p| Some(p.context) == self.snapshot.selected)
        {
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
        body
    }
    fn message_history(&self, header_inset: gpui::Pixels, cx: &Context<Self>) -> Div {
        let previous = self.message_ui.geometry.borrow().header_inset;
        if previous != header_inset {
            let mut anchor = self.message_ui.list.logical_scroll_top();
            self.message_ui.geometry.borrow_mut().header_inset = header_inset;
            // The first real row owns the top clearance, so it scrolls away.
            // List padding in GPUI 0.2.2 is a fixed gutter and omits preceding
            // rows there; it also disagrees with bounds_for_item.
            if self.message_ui.list.item_count() > 0 {
                self.message_ui.list.splice(0..1, 1);
                if anchor.item_ix > 0 && anchor.item_ix < self.message_ui.list.item_count() {
                    anchor.offset_in_item -= header_inset - previous;
                }
                self.message_ui.list.scroll_to(anchor);
            }
        }
        let layout_arrival = self.message_ui.arrival.clone();
        let wheel_arrival = self.message_ui.arrival.clone();
        let keyboard_arrival = self.message_ui.arrival.clone();
        let layout_list = self.message_ui.list.clone();
        let keyboard_list = self.message_ui.list.clone();
        let keyboard_focus = self.navigation.history_focus();
        let layout_focus = keyboard_focus.clone();
        let reveal = self.navigation.reveal.clone();
        let reduced = self.reduced_motion;
        let layout_geometry = self.message_ui.geometry.clone();
        self.message_ui.geometry.borrow_mut().painted.clear();
        div()
            .flex_1()
            .min_h_0()
            .track_focus(&keyboard_focus.clone().tab_stop(!self.background_inert))
            .on_scroll_wheel(move |event, _, _| {
                if event.delta.pixel_delta(px(20.)).y > px(0.) {
                    wheel_arrival.borrow_mut().manual_scroll();
                }
            })
            .on_key_down(move |event, window, cx| {
                if !keyboard_focus.is_focused(window) {
                    return;
                }
                let page =
                    (keyboard_list.viewport_bounds().size.height - header_inset).max(px(0.)) * 0.85;
                match event.keystroke.key.as_str() {
                    "pageup" => keyboard_list.scroll_by(-page),
                    "pagedown" => keyboard_list.scroll_by(page),
                    "home" => keyboard_list.scroll_to(ListOffset {
                        item_ix: 0,
                        offset_in_item: px(0.),
                    }),
                    "end" => keyboard_list.scroll_to(ListOffset {
                        item_ix: keyboard_list.item_count(),
                        offset_in_item: px(0.),
                    }),
                    _ => return,
                }
                keyboard_arrival.borrow_mut().manual_scroll();
                cx.stop_propagation();
                window.refresh();
            })
            .on_children_prepainted(move |_, window, cx| {
                let viewport = layout_list.viewport_bounds();
                let mut geometry = layout_geometry.borrow_mut();
                let painted_anchor = geometry
                    .painted
                    .iter()
                    .find(|(_, _, bounds)| bounds.bottom() > viewport.top())
                    .map(|(index, id, bounds)| (*index, *id, viewport.top() - bounds.top()));
                geometry.anchor = painted_anchor.map(|(_, id, offset)| (id, offset));
                geometry.painted.clear();
                drop(geometry);
                let mut arrival = layout_arrival.borrow_mut();
                if layout_focus.contains_focused(window, cx)
                    && reveal.reveal_history(
                        &layout_list,
                        header_inset,
                        painted_anchor.map(|(index, _, offset)| ListOffset {
                            item_ix: index,
                            offset_in_item: offset,
                        }),
                        window,
                    )
                {
                    arrival.manual_scroll();
                }
                let now = Instant::now();
                let followed = arrival.after_layout(now, reduced);
                if followed {
                    layout_list.scroll_to(ListOffset {
                        item_ix: layout_list.item_count(),
                        offset_in_item: px(0.),
                    });
                }
                if followed || arrival.following() || arrival.animate_next_frame(now) {
                    window.request_animation_frame();
                }
            })
            .child(
                list(
                    self.message_ui.list.clone(),
                    cx.processor(move |this, index, _, cx| {
                        this.snapshot
                            .messages
                            .as_slice()
                            .get(index)
                            .map(|m: &MessageView| {
                                let geometry = this.message_ui.geometry.clone();
                                let id = m.local_id;
                                let row = this
                                    .message_row(m, cx)
                                    .when(index == 0, |d| d.pt(header_inset))
                                    .on_children_prepainted(move |bounds, _, _| {
                                        let mut geometry = geometry.borrow_mut();
                                        if geometry.painted.len() < 100
                                            && let Some(mut bounds) = bounds.first().copied()
                                        {
                                            if index == 0 {
                                                bounds.origin.y -= header_inset;
                                                bounds.size.height += header_inset;
                                            }
                                            geometry.painted.push((index, id, bounds));
                                        }
                                    });
                                let progress = this
                                    .message_ui
                                    .arrival
                                    .borrow()
                                    .progress(m.local_id, Instant::now());
                                if let Some(progress) = progress {
                                    let frames = this.motion_frame_counter();
                                    super::motion::count_frame(&frames);
                                    row.opacity(0.6 + 0.4 * gpui::ease_out_quint()(progress))
                                        .into_any_element()
                                } else {
                                    row.into_any_element()
                                }
                            })
                            .unwrap_or_else(|| div().into_any_element())
                    }),
                )
                .size_full(),
            )
    }

    fn message_row(&self, m: &MessageView, cx: &Context<Self>) -> Div {
        let t = self.theme;
        let context = self.snapshot.selected.expect("message context");
        let action_group: gpui::SharedString = scoped_id("message-actions", context, m.local_id, 0)
            .to_string()
            .into();
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
                    .flex_wrap()
                    .items_baseline()
                    .gap_2()
                    .pr(px(76.))
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
            content = if object.kind.as_str() == "file" {
                content.child(self.attachment_card(
                    talk_core::ui::content::AttachmentRef {
                        context,
                        message: m.local_id,
                        placeholder: object.placeholder.clone(),
                    },
                    object,
                    cx,
                ))
            } else if matches!(object.kind.as_str(), "poll" | "talk-poll")
                && let Some(id) = object.id.as_ref().and_then(|id| id.as_str().parse().ok())
            {
                content.child(self.poll_card(id, cx))
            } else {
                content.child(
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
                )
            };
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
                                LocalAction::MessageIn(
                                    context,
                                    Action::Command(MessageAction::React {
                                        message: m.local_id,
                                        emoji: r.emoji.clone(),
                                        add: !r.own,
                                    }),
                                ),
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
                        self.can_reply_to(m),
                        "Auf diese Nachricht kann nicht geantwortet werden.",
                    ),
                    LocalAction::MessageIn(context, Action::Reply(m.local_id)),
                    cx,
                ))
                .child(self.button(
                    scoped_id("react", context, m.local_id, 0),
                    "👍",
                    ButtonStyle::Quiet,
                    self.message_available(MessageFeature::React),
                    LocalAction::MessageIn(
                        context,
                        Action::Command(MessageAction::React {
                            message: m.local_id,
                            emoji: Text::new("👍").expect("emoji"),
                            add: true,
                        }),
                    ),
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
                    LocalAction::MessageIn(context, Action::Edit(m.local_id)),
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
                    LocalAction::MessageIn(context, Action::ConfirmDelete(m.local_id)),
                    cx,
                ))
                .child(self.button(
                    scoped_id("pin", context, m.local_id, 0),
                    "Anheften",
                    ButtonStyle::Quiet,
                    self.message_available(MessageFeature::Pins),
                    LocalAction::MessageIn(
                        context,
                        Action::Command(MessageAction::Pin(m.local_id)),
                    ),
                    cx,
                ))
                .child(self.button(
                    scoped_id("remind", context, m.local_id, 0),
                    "Später erinnern",
                    ButtonStyle::Quiet,
                    self.message_available(MessageFeature::Reminders),
                    LocalAction::MessageIn(context, Action::Remind(m.local_id)),
                    cx,
                ))
                .child(self.button(
                    scoped_id("forward", context, m.local_id, 0),
                    "Weiterleiten",
                    ButtonStyle::Quiet,
                    self.message_available(MessageFeature::Forward),
                    LocalAction::MessageIn(context, Action::Forward(m.local_id)),
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
                            LocalAction::MessageIn(context, Action::Delete(m.local_id)),
                            cx,
                        ));
            }
            content = content.child(
                self.reveal_from_action(scoped_id("menu-reveal", context, m.local_id, 0), actions),
            );
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
                        LocalAction::MessageIn(
                            context,
                            Action::ChooseForward(m.local_id, room.context),
                        ),
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
                                LocalAction::MessageIn(
                                    context,
                                    Action::ConfirmForward(m.local_id, destination),
                                ),
                                cx,
                            ));
                }
                content = content.child(destinations);
            }
        }
        div().child(
            div()
                .id(scoped_id("message", context, m.local_id, 0))
                .group(action_group.clone())
                .relative()
                .px_6()
                .py_3()
                .flex()
                .gap_3()
                .when(
                    self.highlighted_message == Some((context, m.local_id)),
                    |d| d.bg(t.selected),
                )
                .child(self.content_avatar(
                    talk_core::ui::content::AssetTarget::MessageAvatar {
                        context,
                        message: m.local_id,
                    },
                    m.initials.as_str(),
                    36.,
                ))
                .child(content)
                .child(
                    div()
                        .absolute()
                        .top(px(8.))
                        .right(px(10.))
                        .flex()
                        .gap_1()
                        .child(
                            self.icon_button(
                                scoped_id("message-reply", context, m.local_id, 0),
                                ("reply", "Antworten"),
                                ButtonStyle::Quiet,
                                Availability::when(
                                    self.can_reply_to(m),
                                    "Auf diese Nachricht kann nicht geantwortet werden.",
                                ),
                                LocalAction::MessageIn(context, Action::Reply(m.local_id)),
                                cx,
                            )
                            .opacity(0.)
                            .group_hover(action_group.clone(), |s| s.opacity(1.))
                            .focus(|s| s.opacity(1.)),
                        )
                        .child(
                            self.icon_button(
                                scoped_id("message-menu", context, m.local_id, 0),
                                ("more", "Nachrichtenaktionen"),
                                ButtonStyle::Quiet,
                                Availability::when(
                                    message_actions_allowed(m),
                                    "Für diese Nachricht sind keine Aktionen verfügbar.",
                                ),
                                LocalAction::MessageIn(context, Action::Menu(m.local_id)),
                                cx,
                            )
                            .opacity(0.)
                            .group_hover(action_group, |s| s.opacity(1.))
                            .focus(|s| s.opacity(1.)),
                        ),
                ),
        )
    }
    pub(super) fn shared_contents(&self, cx: &Context<Self>) -> Div {
        let Some(row) = self.selected() else {
            return div().child(self.hint(self.l("Das Gespräch ist nicht mehr verfügbar.")));
        };
        let context = row.context;
        let pins = self.message_ui.show_pins;
        let mut body = div()
            .flex()
            .flex_col()
            .gap_4()
            .child(
                div()
                    .font_weight(FontWeight::MEDIUM)
                    .child(row.title.as_str().to_owned()),
            )
            .child(
                div()
                    .flex()
                    .flex_wrap()
                    .gap_2()
                    .child(self.chip(
                        "shared-files-tab",
                        "Dateien und Medien",
                        (!pins, 0, 2),
                        LocalAction::MessageIn(context, Action::Files),
                        cx,
                    ))
                    .child(self.chip(
                        "shared-pins-tab",
                        "Angeheftete Nachrichten",
                        (pins, 1, 2),
                        LocalAction::MessageIn(context, Action::Pins),
                        cx,
                    )),
            );
        let Some(panel) = self
            .snapshot
            .message_panel
            .as_ref()
            .filter(|p| p.context == context)
        else {
            return body.child(self.hint(self.l("Geteilte Inhalte sind derzeit nicht verfügbar.")));
        };
        let feature = if pins {
            MessageFeature::Pins
        } else {
            MessageFeature::Files
        };
        let available = panel.availability(feature);
        if !available.enabled() {
            return body
                .child(self.hint(self.l(super::conversation_dialog::unavailable(available))));
        }
        if pins {
            for message in panel.pinned_messages.as_slice() {
                body = body.child(
                    div()
                        .flex()
                        .items_start()
                        .gap_2()
                        .child(
                            self.button(
                                scoped_id("pinned-message", context, message.local_id, 0),
                                message.body.as_str().to_owned(),
                                ButtonStyle::Quiet,
                                true,
                                LocalAction::MessageIn(context, Action::Locate(message.local_id)),
                                cx,
                            )
                            .flex_1()
                            .min_w_0()
                            .justify_start(),
                        )
                        .child(self.icon_button(
                            scoped_id("unpin-message", context, message.local_id, 0),
                            ("pin", "Angeheftete Nachricht lösen"),
                            ButtonStyle::Quiet,
                            self.message_available(MessageFeature::Pins),
                            LocalAction::MessageIn(
                                context,
                                Action::Command(MessageAction::Unpin(message.local_id)),
                            ),
                            cx,
                        )),
                );
            }
            if panel.pinned_messages.as_slice().is_empty()
                && !matches!(
                    panel.state,
                    talk_core::ui::conversation::RemoteState::Loading(_)
                )
            {
                body = body.child(self.hint(self.l("Keine angehefteten Nachrichten geladen.")));
            }
        } else {
            body = body.child(self.shared_files(cx));
        }
        match panel.state {
            talk_core::ui::conversation::RemoteState::Loading(_) => {
                body = body.child(self.hint(self.l("Geteilte Inhalte werden geladen …")));
            }
            talk_core::ui::conversation::RemoteState::Failed(_, _) => {
                body =
                    body.child(self.hint(self.l("Geteilte Inhalte konnten nicht geladen werden.")));
            }
            talk_core::ui::conversation::RemoteState::Uncertain(_) => {
                body = body
                    .child(self.hint(self.l("Ergebnis unklar. Vor erneutem Ändern neu laden.")));
            }
            _ => {}
        }
        body.child(self.button(
            "reload-shared",
            "Erneut laden",
            ButtonStyle::Quiet,
            self.message_available(feature),
            LocalAction::MessageIn(context, if pins { Action::Pins } else { Action::Files }),
            cx,
        ))
    }
    pub(super) fn shared_files(&self, cx: &Context<Self>) -> Div {
        let mut body = div().flex().flex_col().gap_3();
        if let Some(panel) = self
            .snapshot
            .message_panel
            .as_ref()
            .filter(|p| Some(p.context) == self.snapshot.selected)
        {
            let context = panel.context;
            body = body.child(self.hint(&format!(
                "{} {} MiB",
                self.l("Maximale Dateigröße:"),
                panel.file_limit / 1024 / 1024
            )));
            for message in panel.shared_files.as_slice() {
                for (index, object) in message
                    .metadata
                    .rich
                    .as_slice()
                    .iter()
                    .filter(|r| r.kind.as_str() == "file")
                    .enumerate()
                {
                    body = body.child(
                        div()
                            .flex()
                            .gap_3()
                            .items_center()
                            .child(div().flex_1().child(self.attachment_card(
                                talk_core::ui::content::AttachmentRef {
                                    context,
                                    message: message.local_id,
                                    placeholder: object.placeholder.clone(),
                                },
                                object,
                                cx,
                            )))
                            .child(self.button(
                                scoped_id(
                                    "download-shared",
                                    context,
                                    message.local_id,
                                    index as u128,
                                ),
                                "Herunterladen",
                                ButtonStyle::Secondary,
                                self.message_available(MessageFeature::Files),
                                LocalAction::MessageIn(
                                    context,
                                    Action::Command(MessageAction::Download {
                                        message: message.local_id,
                                        placeholder: object.placeholder.clone(),
                                        name: Text::new(object.name.as_str()).unwrap_or_else(
                                            |_| Text::new("download").expect("constant"),
                                        ),
                                    }),
                                ),
                                cx,
                            )),
                    );
                }
            }
            if panel.shared_files.as_slice().is_empty()
                && !matches!(
                    panel.state,
                    talk_core::ui::conversation::RemoteState::Loading(_)
                )
            {
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
    fn compose_motion_id(&self, context: ConversationRef) -> Option<gpui::ElementId> {
        let target = self.drafts.target(context);
        self.message_ui
            .editing
            .map(|id| scoped_id("compose-mode", context, 0, id))
            .or_else(|| {
                target
                    .thread
                    .map(|id| scoped_id("compose-mode", context, 1, id))
            })
            .or_else(|| {
                target
                    .reply_to
                    .map(|id| scoped_id("compose-mode", context, 2, id))
            })
    }
    pub(super) fn composer_context(&self, cx: &Context<Self>) -> Div {
        let mut body = div()
            .flex()
            .flex_col()
            .gap_2()
            .child(self.download_status(cx));
        let Some(context) = self.snapshot.selected else {
            return body;
        };
        let target = self.drafts.target(context);
        if let Some(motion_id) = self.compose_motion_id(context) {
            let label = if self.message_ui.editing.is_some() {
                "Nachricht bearbeiten"
            } else if target.thread.is_some() {
                "Antwort im Thread"
            } else {
                "Antwort auf eine Nachricht"
            };
            body = body.child(
                self.reveal_from_action(
                    motion_id,
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
                ),
            );
        }
        body = body.when(self.composer_active, |d| {
            d.child(
                div()
                    .flex()
                    .flex_wrap()
                    .gap_2()
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
            )
        });
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
            let show_mentions = self.composer_active
                && self
                    .message_ui
                    .mention_ready
                    .as_deref()
                    .is_some_and(|q| self.composer.read(cx).mention_query() == Some(q));
            for (index, mention) in panel
                .mentions
                .as_slice()
                .iter()
                .take(if show_mentions { 8 } else { 0 })
                .enumerate()
            {
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
            for scheduled in panel
                .scheduled
                .as_slice()
                .iter()
                .take(if self.composer_active { 8 } else { 0 })
            {
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
            for reminder in panel
                .reminders
                .as_slice()
                .iter()
                .take(if self.composer_active { 8 } else { 0 })
            {
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
fn message_actions_allowed(message: &MessageView) -> bool {
    !message.metadata.deleted
        && !message.metadata.system
        && matches!(
            message.delivery,
            talk_core::ui::presentation::DeliveryState::Received
                | talk_core::ui::presentation::DeliveryState::ServerAccepted
        )
}
fn reply_allowed(
    message: &MessageView,
    content: ContentState,
    send: Decision,
    reply_feature: bool,
    pending: bool,
) -> bool {
    message_actions_allowed(message)
        && message.metadata.replyable
        && content == ContentState::Ready
        && send == Decision::Allowed
        && reply_feature
        && !pending
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
    use super::*;
    #[test]
    fn prepend_and_eviction_preserve_the_visible_row_when_header_clearance_moves() {
        for inset in [px(0.), px(74.), px(158.)] {
            for was_first in [false, true] {
                for is_first in [false, true] {
                    for offset in [px(-30.), px(0.), px(17.)] {
                        let next = preserved_row_offset(offset, was_first, is_first, inset);
                        let before = -offset + if was_first { inset } else { px(0.) };
                        let after = -next + if is_first { inset } else { px(0.) };
                        assert_eq!(before, after);
                    }
                }
            }
        }
    }

    #[test]
    fn quick_reply_revalidates_canonical_message_and_current_send_state() {
        let mut message = MessageView {
            local_id: 7,
            author: Text::new("Test").unwrap(),
            initials: Text::new("T").unwrap(),
            time_label: Text::new("09:00").unwrap(),
            body: Text::new("Synthetic message").unwrap(),
            own: false,
            delivery: talk_core::ui::presentation::DeliveryState::Received,
            attachments: talk_core::ui::Page::empty(),
            reply_count: 0,
            metadata: talk_core::ui::messaging::MessageMetadata {
                replyable: true,
                ..Default::default()
            },
        };
        let allowed =
            |m: &MessageView| reply_allowed(m, ContentState::Ready, Decision::Allowed, true, false);
        assert!(allowed(&message));
        message.metadata.deleted = true;
        assert!(!allowed(&message));
        message.metadata.deleted = false;
        message.metadata.system = true;
        assert!(!allowed(&message));
        message.metadata.system = false;
        for delivery in [
            talk_core::ui::presentation::DeliveryState::Pending,
            talk_core::ui::presentation::DeliveryState::Failed,
            talk_core::ui::presentation::DeliveryState::Uncertain,
        ] {
            message.delivery = delivery;
            assert!(!allowed(&message));
        }
        message.delivery = talk_core::ui::presentation::DeliveryState::ServerAccepted;
        assert!(allowed(&message));
        assert!(!reply_allowed(
            &message,
            ContentState::Offline,
            Decision::Allowed,
            true,
            false
        ));
        for send in [Decision::Denied, Decision::Unknown, Decision::Stale] {
            assert!(!reply_allowed(
                &message,
                ContentState::Ready,
                send,
                true,
                false
            ));
        }
        assert!(!reply_allowed(
            &message,
            ContentState::Ready,
            Decision::Allowed,
            false,
            false
        ));
        assert!(!reply_allowed(
            &message,
            ContentState::Ready,
            Decision::Allowed,
            true,
            true
        ));
    }
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
