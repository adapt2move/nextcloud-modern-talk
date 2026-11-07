//! Stable, variable-height native history and deliberate message actions.
mod arrival;
pub(super) mod composer_menu;
mod date_picker;
mod deadline;
pub(super) mod forward;
mod history;
mod menu;
mod pins;
mod private_reply;
mod reactions;
mod recovery;
pub(super) mod row;
mod summary;
mod thread_actions;
mod translation;

use super::{
    ButtonStyle, Desktop, LocalAction, Overlay,
    collection::CollectionState,
    controls::{Availability, Skeleton, scoped_id},
    input::TextInput,
};
use gpui::{
    Context, Div, Entity, FocusHandle, Focusable, FontWeight, ListAlignment, ListOffset, ListState,
    Window, div, list, prelude::*, px,
};
use std::{cell::RefCell, rc::Rc, time::Instant};
use talk_core::ui::messaging::{ComposeTarget, ContextLoad, MessageAction, MessageFeature};
use talk_core::ui::presentation::{
    Completion, ContentState, ConversationRef, MessageView, RequestId, UiIntent,
};
use talk_core::ui::{Decision, Text};

#[derive(Clone)]
pub(crate) enum Action {
    Files,
    Silent,
    Pins,
    Menu(u128),
    ThreadMenu(u128),
    ReactionPalette(u128, menu::Source),
    React(u128, Text<64>),
    ReactionAuthors(u128, Text<64>),
    Recovery(recovery::RecoveryAction),
    MenuChoice(menu::Target, menu::Choice),
    Forward(forward::Action),
    RetryHistory,
    RetryContext { request: RequestId, target: u128 },
    Reply(u128),
    PrivateReply(u128),
    ThreadReply(u128),
    ThreadEdit(u128),
    ThreadCancel,
    Edit(u128),
    CancelCompose,
    Command(MessageAction),
    Search,
    ToggleSearch,
    CloseSearch,
    Locate(u128),
    JumpUnread,
    OlderThread(u128),
    OpenLocation(u128, Text<128>, Text<1024>),
    Translate(u128),
    HideTranslation(u128),
    CyclePin { current: u128, forward: bool },
    TogglePinStrip,
    Schedule,
    EditScheduled(u128),
    ToggleScheduled,
    CancelTime,
    Remind { message: u128, thread: Option<u128> },
    SubmitTime,
    SubmitRepeatedTime(talk_core::preferences::time::RepeatedTime),
    ChooseTime(talk_core::preferences::time::DeadlineChoice),
    DatePicker(date_picker::PickerAction),
    InsertMention(String),
    InsertThreadMention(String),
}
pub(super) struct MessageUi {
    pub thread_load: Option<ThreadLoad>,
    thread_info_retry: Option<(ConversationRef, u128, Instant)>,
    thread_root_retry: Option<(ConversationRef, u128, Instant)>,
    private_reply_pending: Option<private_reply::Pending>,
    pub(super) private_reply_focus: Option<ConversationRef>,
    thread_edit: Option<thread_actions::Edit>,
    thread_edit_generation: u64,
    pub list: ListState,
    context: Option<ConversationRef>,
    observed: Vec<MessageView>,
    observed_pins: Vec<u128>,
    observed_translations: Vec<talk_core::ui::messaging::Translation>,
    observed_translation_status: (Option<u128>, Option<u128>),
    hidden_translations: Vec<u128>,
    arrival: Rc<RefCell<arrival::Arrival>>,
    geometry: Rc<RefCell<MessageGeometry>>,
    pub editing: Option<u128>,
    pub(super) composer_load_failed: Option<ConversationRef>,
    edit_generation: u64,
    pub(super) pending_edit: Option<super::state::PendingEdit>,
    request: u64,
    menu: menu::Menu,
    pub(super) forward: forward::Forward,
    pins: pins::Pins,
    reaction_request: Option<RequestId>,
    reaction_authors_visible: Option<(ConversationRef, u128, Text<64>, bool)>,
    pending_action: Option<(RequestId, Option<&'static str>)>,
    failed_selection: Option<(ConversationRef, u128)>,
    history: Rc<RefCell<history::History>>,
    last_read: Option<(ConversationRef, u128)>,
    unread_hold: Option<(ConversationRef, RequestId)>,
    unread_prior: Option<(RequestId, Option<(ConversationRef, u128)>)>,
    unread_marker: Option<(ConversationRef, u128)>,
    first_unread: Option<(ConversationRef, u128)>,
    unread_jump: Option<(ConversationRef, RequestId)>,
    last_located: Option<RequestId>,
    time_target: Option<Option<u128>>,
    time_thread: Option<u128>,
    picker: Option<talk_core::preferences::time::calendar::Calendar>,
    scheduled_edit: Option<(u128, bool)>,
    show_scheduled: bool,
    fixture_menu_pending: bool,
    pub scheduled_input: Entity<TextInput>,
    time_error: Option<talk_core::preferences::time::LocalTimeError>,
    pub search: Entity<TextInput>,
    show_search: bool,
    search_return_focus: Option<FocusHandle>,
    search_pending_focus: bool,
    show_pins: bool,
    mention_candidate: Option<(ConversationRef, bool, String, Instant)>,
    mention_request: Option<(RequestId, bool, String)>,
    mention_ready: Option<(bool, String)>,
}

pub(super) struct ThreadLoad {
    pub context: ConversationRef,
    pub root: u128,
    pub request: Option<RequestId>,
}

struct ContextStatus {
    label: &'static str,
    retry: Option<RequestId>,
}
fn context_status(load: &ContextLoad) -> Option<ContextStatus> {
    use talk_core::ui::{UnavailableReason, conversation::RemoteState};
    let (label, retry) = match load.state {
        RemoteState::Idle => return None,
        RemoteState::Loading(_) | RemoteState::Saving(_) => return None,
        RemoteState::Failed(_, UnavailableReason::Unsupported) => {
            ("Der Server unterstützt diese Funktion nicht.", None)
        }
        RemoteState::Failed(_, UnavailableReason::Unconfigured) => {
            ("In diesem Client noch nicht verfügbar.", None)
        }
        RemoteState::Failed(request, UnavailableReason::PermissionDenied) => (
            "Auf diese Nachricht kannst du derzeit nicht zugreifen. Der Gesprächsverlauf bleibt erhalten.",
            Some(request),
        ),
        RemoteState::Failed(request, _) | RemoteState::Uncertain(request) => (
            "Die Nachricht konnte nicht geöffnet werden. Der Gesprächsverlauf bleibt erhalten.",
            Some(request),
        ),
    };
    Some(ContextStatus { label, retry })
}

fn current_context_retry(load: &ContextLoad, request: RequestId, target: u128) -> bool {
    load.target == target
        && context_status(load).is_some_and(|status| status.retry == Some(request))
}

impl ThreadLoad {
    pub fn content_state(
        &self,
        panel: Option<&talk_core::ui::messaging::MessagePanel>,
    ) -> ContentState {
        use talk_core::ui::{UnavailableReason, conversation::RemoteState};
        let Some(request) = self.request else {
            return ContentState::Failed;
        };
        let Some(panel) =
            panel.filter(|p| p.context == self.context && p.thread_root == Some(self.root))
        else {
            return ContentState::Loading;
        };
        match panel.thread_state {
            RemoteState::Idle => ContentState::Ready,
            RemoteState::Failed(id, reason) if id == request => match reason {
                UnavailableReason::PermissionDenied => ContentState::Denied,
                UnavailableReason::Connection(_) => ContentState::Offline,
                UnavailableReason::Unconfigured if !panel.thread_messages.as_slice().is_empty() => {
                    ContentState::Offline
                }
                UnavailableReason::Unsupported | UnavailableReason::Unconfigured => {
                    ContentState::Unconfigured
                }
                _ => ContentState::Failed,
            },
            RemoteState::Uncertain(id) if id == request => ContentState::Failed,
            _ => ContentState::Loading,
        }
    }
}

#[derive(Default)]
struct MessageGeometry {
    // Only this page's painted rows, drained after prepaint. GPUI's Bottom list
    // exposes an end sentinel instead of the actual visible first-row offset.
    painted: Vec<(usize, u128, gpui::Bounds<gpui::Pixels>, bool)>,
    visible_read: Option<(ConversationRef, u128)>,
    unread_visible: Option<bool>,
    anchor: Option<(u128, gpui::Pixels)>,
    header_inset: gpui::Pixels,
    boundary_inset: gpui::Pixels,
    compact: bool,
}

/// Painting an older row is navigation, never an instruction to rewind reading.
fn read_advances(
    previous: Option<(ConversationRef, u128)>,
    context: ConversationRef,
    id: u128,
) -> bool {
    previous.is_none_or(|(room, read)| room != context || id > read)
}

fn visible_read(
    painted: &[(usize, u128, gpui::Bounds<gpui::Pixels>, bool)],
    viewport: gpui::Bounds<gpui::Pixels>,
) -> Option<u128> {
    if viewport.size.height <= px(0.) {
        return None;
    }
    painted
        .iter()
        .filter(|(_, _, bounds, canonical)| {
            *canonical && bounds.intersects(&viewport) && bounds.bottom() <= viewport.bottom()
        })
        .max_by_key(|(index, _, _, _)| *index)
        .map(|(_, id, _, _)| *id)
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
        let pins = pins::Pins::default();
        let cancel_pins = pins.cancellation();
        let list = ListState::new(0, ListAlignment::Bottom, px(300.));
        list.set_scroll_handler(move |event, _, _| {
            cancel_pins();
            if event.is_scrolled {
                scroll_arrival.borrow_mut().manual_scroll();
            }
        });
        Self {
            thread_load: None,
            thread_info_retry: None,
            thread_root_retry: None,
            private_reply_pending: None,
            private_reply_focus: None,
            thread_edit: None,
            thread_edit_generation: 0,
            list,
            arrival,
            geometry: Rc::new(RefCell::new(MessageGeometry::default())),
            context: None,
            observed: Vec::new(),
            observed_pins: Vec::new(),
            observed_translations: Vec::new(),
            observed_translation_status: (None, None),
            hidden_translations: Vec::new(),
            editing: None,
            composer_load_failed: None,
            edit_generation: 0,
            pending_edit: None,
            request: 0,
            menu: menu::Menu::new(cx),
            forward: forward::Forward::new(theme, cx),
            pins,
            reaction_request: None,
            reaction_authors_visible: None,
            pending_action: None,
            failed_selection: None,
            history: Rc::new(RefCell::new(history::History::default())),
            last_read: None,
            unread_hold: None,
            unread_prior: None,
            unread_marker: None,
            first_unread: None,
            unread_jump: None,
            last_located: None,
            time_target: None,
            time_thread: None,
            picker: None,
            scheduled_edit: None,
            show_scheduled: false,
            fixture_menu_pending: false,
            scheduled_input: cx.new(|cx| TextInput::new("Nachricht bearbeiten", 4096, theme, cx)),
            time_error: None,
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
    pub(super) fn open_pending_thread(&mut self, cx: &mut Context<Self>) {
        let Some((context, root)) = self.pending_thread_open else {
            return;
        };
        if self.snapshot.active_account != Some(context.account)
            || !self
                .snapshot
                .threads
                .entries
                .as_slice()
                .iter()
                .any(|item| item.context == context && item.root == root)
        {
            self.pending_thread_open = None;
            return;
        }
        if self.snapshot.selected != Some(context)
            || !self.message_feature_enabled(MessageFeature::Threads)
        {
            return;
        }
        if !self.prepare_thread_draft(context, root, cx) {
            self.pending_thread_open = None;
            return;
        }
        if let Some(request) = self.submit_message(MessageAction::Thread(root)) {
            self.submit_message(MessageAction::ThreadInfo(root));
            self.thread_options_open = false;
            self.thread_renaming = false;
            self.thread_parent = Some((context, root));
            self.message_ui.thread_load = Some(ThreadLoad {
                context,
                root,
                request: Some(request),
            });
            self.pending_thread_open = None;
            self.destination = super::SidebarDestination::Conversations;
            self.overlay = Some(Overlay::Thread);
            self.pending_modal_focus = true;
            cx.notify();
        }
    }
    pub(super) fn save_thread_draft(&mut self, cx: &gpui::App) -> bool {
        let Some((context, root)) = self.thread_parent else {
            return true;
        };
        let input = self.thread_composer.read(cx);
        if input.composing() {
            return false;
        }
        if let Some(edit) = self.message_ui.thread_edit.as_mut()
            && edit.context == context
            && edit.root == root
        {
            let Ok(body) = Text::new(input.committed_value()) else {
                return false;
            };
            edit.body = body;
            return true;
        }
        self.thread_drafts
            .save(context, input.committed_value())
            .is_ok()
    }

    pub(super) fn prepare_thread_draft(
        &mut self,
        context: ConversationRef,
        root: u128,
        cx: &mut Context<Self>,
    ) -> bool {
        if self.thread_parent == Some((context, root)) {
            return true;
        }
        if !self.save_thread_draft(cx)
            || !self.thread_drafts.set_target(
                context,
                ComposeTarget {
                    thread: Some(root),
                    ..ComposeTarget::default()
                },
            )
        {
            return false;
        }
        if self
            .message_ui
            .time_thread
            .is_some_and(|scope| scope != root)
        {
            self.message_ui.time_target = None;
            self.message_ui.time_thread = None;
        }
        let text = self
            .message_ui
            .thread_edit
            .as_ref()
            .filter(|edit| edit.context == context && edit.root == root)
            .map_or_else(
                || self.thread_drafts.text(context).to_owned(),
                |edit| edit.body.as_str().to_owned(),
            );
        self.thread_composer.update(cx, |input, cx| {
            let loaded = input.set_context_value(&text);
            cx.notify();
            loaded
        })
    }

    pub(super) fn send_thread_draft(&mut self, cx: &mut Context<Self>) {
        self.stop_local_typing();
        let Some((context, root)) = self.thread_parent else {
            return;
        };
        if self.thread_edit_active(context, root) {
            self.submit_thread_edit(context, root, cx);
            return;
        }
        if Some(context) != self.snapshot.selected
            || self.overlay != Some(Overlay::Thread)
            || self.thread_drafts.target(context).thread != Some(root)
            || !self.thread_send_available(cx)
            || !self.save_thread_draft(cx)
        {
            return;
        }
        let Some(row) = self.selected() else { return };
        let gate = super::state::composer_send_decision(&self.snapshot, row);
        let result = self.thread_drafts.send(context, gate, self.port.as_mut());
        if let Ok(request) = result {
            self.thread_drafts.capture_editor(
                context,
                request,
                self.thread_composer.read(cx).edit_generation(),
            );
            self.feedback = None;
        } else if let Err(error) = result {
            self.report_draft_error(error, context);
        }
        cx.notify();
    }

    pub(super) fn thread_send_available(&self, cx: &gpui::App) -> bool {
        let Some((context, root)) = self.thread_parent else {
            return false;
        };
        if self.thread_edit_active(context, root) {
            return self.thread_edit_available(context, root, cx);
        }
        self.overlay == Some(Overlay::Thread)
            && Some(context) == self.snapshot.selected
            && self.thread_drafts.target(context).thread == Some(root)
            && !self.thread_drafts.pending(context)
            && !self.thread_composer.read(cx).composing()
            && !self.thread_composer.read(cx).content.trim().is_empty()
            && self.snapshot.message_panel.as_ref().is_some_and(|panel| {
                panel.context == context
                    && thread_reply_feature_allowed(
                        self.snapshot.content,
                        panel.availability(MessageFeature::Threads),
                        panel.queue_available,
                        panel.thread_root == Some(root),
                    )
            })
            && self.selected().is_some_and(|row| {
                super::state::composer_send_decision(&self.snapshot, row) == Decision::Allowed
            })
    }
    pub(crate) fn fixture_message_tools(
        &mut self,
        surface: &str,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if !self.development {
            return;
        }
        match surface {
            "chat-reminder-time" => self.open_message_deadline(Some(45), None, window, cx),
            "chat-reminder-menu" => self.message_ui.fixture_menu_pending = true,
            "chat-schedule-time" => {
                self.composer.update(cx, |input, cx| {
                    input.set_context_value("Notizen morgen teilen.");
                    cx.notify();
                });
                self.open_message_deadline(None, None, window, cx);
                if self.message_ui.picker.is_none() {
                    self.message_ui.picker =
                        talk_core::preferences::time::calendar::Calendar::parse("2027-01-16 09:00");
                }
            }
            "chat-schedule-edit" => self.open_scheduled_edit(41, window, cx),
            _ => self.message_ui.show_scheduled = true,
        }
    }
    pub(super) fn prepare_message_tool_fixture(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if !self.development || !self.message_ui.fixture_menu_pending || !window.is_window_active()
        {
            return;
        }
        self.message_ui.fixture_menu_pending = false;
        let view = cx.weak_entity();
        window.on_next_frame(move |window, cx| {
            let _ = view.update(cx, |this, cx| {
                this.message_action(Action::Menu(45), window, cx)
            });
        });
    }
    /// Reuse the composition owner's finite refresh tick for one debounced
    /// mention query. No per-keystroke task or unbounded request queue.
    pub(super) fn refresh_composer_mentions(&mut self, cx: &Context<Self>) {
        let current = self
            .snapshot
            .selected
            .filter(|_| {
                (self.composer_active || self.thread_composer_active)
                    && self.composer_available()
                    && self.message_feature_enabled(MessageFeature::Mentions)
            })
            .and_then(|context| {
                let thread = self.thread_composer_active;
                let editor = if thread {
                    &self.thread_composer
                } else {
                    &self.composer
                };
                editor
                    .read(cx)
                    .mention_query()
                    .map(|q| (context, thread, q.to_owned()))
            });
        let changed = self
            .message_ui
            .mention_candidate
            .as_ref()
            .map(|(c, thread, q, _)| (*c, *thread, q.as_str()))
            != current
                .as_ref()
                .map(|(c, thread, q)| (*c, *thread, q.as_str()));
        if changed {
            self.message_ui.mention_candidate =
                current.map(|(c, thread, q)| (c, thread, q, Instant::now()));
            self.message_ui.mention_request = None;
            self.message_ui.mention_ready = None;
        }
        let Some((_, thread, query, since)) = &self.message_ui.mention_candidate else {
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
        let thread = *thread;
        if let Ok(bounded) = Text::new(&query)
            && let Some(request) = self.submit_message(MessageAction::Mentions(bounded))
        {
            self.message_ui.mention_request = Some((request, thread, query));
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
        // Only a history mounted in this frame may refresh its read candidate.
        self.message_ui.geometry.borrow_mut().visible_read = None;
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
            && matches!(
                self.snapshot.content,
                ContentState::Ready | ContentState::Offline
            )
            && self.prejoin.is_none()
            && !self.incoming_visible()
            && !self.onboarding_open
            && !self.navigation_open
            && matches!(
                self.overlay,
                None | Some(Overlay::CallChat | Overlay::Thread)
            )
            && if self.overlay == Some(Overlay::CallChat) {
                true
            } else if let Some(call) = call_host {
                !matches!(
                    call.stage,
                    super::CallStage::Ended | super::CallStage::Failed
                ) && super::docked_call_chat(
                    window.viewport_size().width >= px(900.) && self.scale < 150,
                    self.call_chat_open,
                    self.overlay,
                )
            } else {
                self.destination != super::SidebarDestination::Inbox
            };
        self.settle_pin_scroll_for_reduced_motion();
        self.message_ui
            .pins
            .visibility(visible, self.reduced_motion);
        self.message_ui
            .arrival
            .borrow_mut()
            .visibility(visible, self.reduced_motion);
        self.message_ui.history.borrow_mut().visibility(visible);
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
        self.refresh_message_ui_after_rebind(false, cx);
    }
    pub(super) fn refresh_message_ui_after_rebind(
        &mut self,
        preserve_edit: bool,
        cx: &mut Context<Self>,
    ) {
        if self.overlay == Some(Overlay::Thread)
            && let Some((context, root)) = self.thread_parent
            && Some(context) == self.snapshot.selected
            && !self
                .snapshot
                .messages
                .as_slice()
                .iter()
                .any(|row| row.local_id == root)
            && self.snapshot.message_panel.as_ref().is_some_and(|panel| {
                use talk_core::ui::{UnavailableReason, conversation::RemoteState};
                panel.context == context
                    && panel.thread_root == Some(root)
                    && panel.thread_root_message.is_none()
                    && !panel
                        .thread_messages
                        .as_slice()
                        .iter()
                        .any(|row| row.local_id == root)
                    && !panel.thread_state.pending()
                    && matches!(
                        panel.thread_root_state,
                        RemoteState::Idle
                            | RemoteState::Failed(_, UnavailableReason::Connection(_))
                    )
            })
            && self
                .message_ui
                .thread_root_retry
                .is_none_or(|(room, id, at)| room != context || id != root || Instant::now() >= at)
        {
            self.message_ui.thread_root_retry = Some((
                context,
                root,
                Instant::now() + std::time::Duration::from_secs(5),
            ));
            self.submit_message(MessageAction::ThreadRoot(root));
        }
        if self.overlay == Some(Overlay::Thread)
            && let Some((context, root)) = self.thread_parent
            && Some(context) == self.snapshot.selected
            && self.snapshot.message_panel.as_ref().is_some_and(|panel| {
                panel.context == context
                    && panel.thread_root == Some(root)
                    && panel
                        .thread_details
                        .as_ref()
                        .is_none_or(|details| details.root != root)
                    && panel.thread_details_request.is_none()
            })
            && self.message_feature_enabled(MessageFeature::Threads)
            && self
                .message_ui
                .thread_info_retry
                .is_none_or(|(room, id, at)| room != context || id != root || Instant::now() >= at)
        {
            self.message_ui.thread_info_retry = Some((
                context,
                root,
                Instant::now() + std::time::Duration::from_secs(5),
            ));
            self.submit_message(MessageAction::ThreadInfo(root));
        }
        self.refresh_pin_banner();
        let rows = self.snapshot.messages.as_slice();
        let pins: Vec<_> = self
            .snapshot
            .message_panel
            .as_ref()
            .filter(|panel| Some(panel.context) == self.snapshot.selected)
            .into_iter()
            .flat_map(|panel| panel.pinned_messages.as_slice())
            .map(|message| message.local_id)
            .collect();
        let (translations, translation_status) = self
            .snapshot
            .message_panel
            .as_ref()
            .filter(|panel| Some(panel.context) == self.snapshot.selected)
            .map_or_else(
                || (Vec::new(), (None, None)),
                |panel| {
                    (
                        panel.translations.as_slice().to_vec(),
                        (panel.translation_pending, panel.translation_failed),
                    )
                },
            );
        // Pin markers change row height without changing canonical message text.
        // Invalidate the same bounded list geometry and visibility observation.
        let rows_changed = self.message_ui.observed != rows
            || self.message_ui.observed_pins != pins
            || self.message_ui.observed_translations != translations
            || self.message_ui.observed_translation_status != translation_status;
        if rows_changed {
            self.message_ui.geometry.borrow_mut().visible_read = None;
        }
        if self.message_ui.context != self.snapshot.selected {
            self.message_ui.unread_hold = None;
            self.message_ui.unread_prior = None;
            self.message_ui.arrival.borrow_mut().reset();
            *self.message_ui.geometry.borrow_mut() = MessageGeometry::default();
            *self.message_ui.history.borrow_mut() = history::History::default();
        }
        // Explicit history/context loads establish a baseline, even when the
        // previously loaded conversation remains visible during the request.
        let live = matches!(
            self.snapshot.content,
            ContentState::Ready | ContentState::Offline
        ) && self.snapshot.message_panel.as_ref().is_none_or(|panel| {
            !panel.history.pending()
                && panel
                    .located
                    .is_none_or(|(request, _)| self.message_ui.last_located == Some(request))
        });
        let at_bottom = self.timeline_at_bottom();
        self.message_ui.arrival.borrow_mut().observe(
            &self.message_ui.observed,
            rows,
            live,
            at_bottom,
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
            self.message_ui.unread_marker = self.snapshot.selected.and_then(|context| {
                self.snapshot
                    .conversations
                    .as_slice()
                    .iter()
                    .find(|row| row.context == context && row.unread > 0)
                    .and_then(|row| row.last_read_message.map(|marker| (context, marker)))
            });
            self.message_ui.first_unread = None;
            self.message_ui.unread_jump = None;
            self.message_ui.list.reset(rows.len());
            self.message_ui.observed = rows.to_vec();
            self.message_ui.menu.clear();
            self.message_ui.reaction_request = None;
            self.message_ui.reaction_authors_visible = None;
            self.message_ui.pending_action = None;
            if !preserve_edit {
                self.message_ui.editing = None;
                self.message_ui.pending_edit = None;
            }
            self.message_ui.time_target = None;
            self.message_ui.time_thread = None;
            self.message_ui.picker = None;
            self.message_ui.scheduled_edit = None;
            self.message_ui.show_scheduled = false;
            self.message_ui.hidden_translations.clear();
            self.message_ui.show_search = false;
            self.message_ui.search_return_focus = None;
            self.message_ui.search_pending_focus = false;
            self.message_ui.search.update(cx, |i, _| i.reset());
            self.message_ui.show_pins = false;
            self.message_ui.last_located = None;
        } else if rows_changed
            && at_bottom
            && !self.message_ui.arrival.borrow().following()
            && !self
                .message_ui
                .observed
                .first()
                .zip(rows.first())
                .is_some_and(|(previous, current)| current.local_id < previous.local_id)
        {
            // An edit, deletion or height change at the end keeps the reader
            // there, including expiry of the first retained row. An older page
            // instead preserves its visible anchor below. Whole-range splice
            // preserves the list's bottom sentinel;
            // only an admitted arrival pins the old view for its delayed follow.
            self.message_ui
                .list
                .splice(0..self.message_ui.observed.len(), rows.len());
            self.message_ui.list.scroll_to(ListOffset {
                item_ix: rows.len(),
                offset_in_item: px(0.),
            });
            self.message_ui.observed = rows.to_vec();
        } else if rows_changed {
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
                    offset_in_item: preserved_row_offset(offset, was_first, index == 0, {
                        let geometry = self.message_ui.geometry.borrow();
                        geometry.header_inset + geometry.boundary_inset
                    }),
                });
            }
            self.message_ui.observed = rows.to_vec();
        }
        if self.message_ui.first_unread.is_none()
            && let Some((context, marker)) = self.message_ui.unread_marker
            && Some(context) == self.snapshot.selected
        {
            let first = rows
                .iter()
                .find(|row| row.metadata.unread_after == Some(marker))
                .or_else(|| {
                    (marker == 0
                        && self.snapshot.message_panel.as_ref().is_some_and(|panel| {
                            panel.context == context
                                && panel.older.is_none()
                                && !panel.history.pending()
                                && self.snapshot.content == ContentState::Ready
                        }))
                    .then(|| rows.first())
                    .flatten()
                });
            if let Some(first) = first {
                self.message_ui.first_unread = Some((context, first.local_id));
            }
        }
        self.message_ui.observed_pins = pins;
        self.message_ui.observed_translations = translations;
        self.message_ui.observed_translation_status = translation_status;
        if let Some(panel) = self
            .snapshot
            .message_panel
            .as_ref()
            .filter(|p| Some(p.context) == self.snapshot.selected)
        {
            self.message_ui.history.borrow_mut().reconcile(
                self.snapshot.revision,
                panel.history,
                panel.older,
            );
        }
        if let Some((request, id)) = self
            .snapshot
            .message_panel
            .as_ref()
            .filter(|p| Some(p.context) == self.snapshot.selected)
            .and_then(|p| p.located)
            && self.message_ui.last_located != Some(request)
            && rows.iter().any(|m| m.local_id == id)
        {
            if let Some(context) = self.snapshot.selected {
                if self.message_ui.unread_jump == Some((context, request)) {
                    self.message_ui.first_unread = Some((context, id));
                    self.message_ui.unread_jump = None;
                }
                self.locate_message_smooth(context, id);
            }
            self.message_ui.last_located = Some(request);
        }
        if let Some((request, completion)) = self
            .snapshot
            .message_panel
            .as_ref()
            .filter(|p| Some(p.context) == self.snapshot.selected)
            .and_then(|p| p.result)
        {
            if self
                .message_ui
                .unread_hold
                .is_some_and(|(context, pending)| {
                    Some(context) == self.snapshot.selected && pending == request
                })
                && matches!(completion, Completion::Failed(_))
            {
                self.message_ui.unread_hold = None;
                if let Some((_, prior)) = self
                    .message_ui
                    .unread_prior
                    .take()
                    .filter(|(pending, _)| *pending == request)
                {
                    self.message_ui.first_unread = prior;
                }
            } else if completion == Completion::ServerAccepted
                && self
                    .message_ui
                    .unread_prior
                    .is_some_and(|(pending, _)| pending == request)
            {
                self.message_ui.unread_prior = None;
            }
            if self.message_ui.reaction_request == Some(request) {
                self.message_ui.reaction_request = None;
            }
            if let Some((pending, success)) = self.message_ui.pending_action
                && pending == request
            {
                self.message_ui.pending_action = None;
                let notice = match completion {
                    Completion::Queued => None,
                    Completion::ServerAccepted => success.map(super::feedback::Feedback::notice),
                    Completion::Failed(reason) => super::feedback::Feedback::from_error(
                        talk_core::ui::SubmitError::Unavailable(reason),
                    ),
                    Completion::Uncertain => Some(super::feedback::Feedback::warning(
                        "Ergebnis unklar. Vor erneutem Ändern neu laden.",
                    )),
                };
                if let Some(notice) = notice
                    && let Some(context) = self.snapshot.selected
                {
                    self.feedback = Some(notice.in_conversation(context));
                }
            }
            self.complete_person_mention(request, completion, cx);
            if let Some((pending, thread, query)) = &self.message_ui.mention_request
                && *pending == request
                && completion == Completion::ServerAccepted
            {
                self.message_ui.mention_ready = Some((*thread, query.clone()));
            }
            self.complete_thread_edit(request, completion, cx);
            if let Some(restore) = super::state::PendingEdit::complete(
                &mut self.message_ui.pending_edit,
                request,
                completion,
                self.message_ui
                    .editing
                    .map(|id| (id, self.message_ui.edit_generation)),
                self.composer.read(cx).committed_value(),
                (!self.composer.read(cx).composing())
                    .then(|| self.composer.read(cx).edit_generation()),
            ) {
                if restore {
                    let body = self
                        .snapshot
                        .selected
                        .map_or("", |c| self.drafts.text(c))
                        .to_owned();
                    if self.load_composer_value(&body, cx) {
                        self.message_ui.editing = None;
                    }
                } else if !matches!(completion, Completion::Queued | Completion::ServerAccepted)
                    && let Some(context) = self.snapshot.selected
                {
                    self.feedback = Some(
                        super::feedback::Feedback::error(
                            "Die Änderung konnte nicht bestätigt werden. Bitte neu laden.",
                        )
                        .in_conversation(context),
                    );
                }
            }
            if let Some(context) = self.snapshot.selected {
                let _ = self
                    .port
                    .try_submit(UiIntent::AcknowledgeMessage { request, context });
            }
        }
    }
    pub(super) fn clear_message_visibility(&mut self) {
        let mut geometry = self.message_ui.geometry.borrow_mut();
        geometry.visible_read = None;
        geometry.painted.clear();
    }
    pub(crate) fn observe_message_visibility(&mut self, window: &mut Window) {
        if window.is_window_active() && self.request_older_history(false) {
            window.refresh();
        }
        let Some(context) = self.visible_message_conversation(window) else {
            return;
        };
        self.observe_visible_message(context);
    }
    pub(crate) fn visible_message_conversation(&self, window: &Window) -> Option<ConversationRef> {
        if self.update_restart_pending
            || self.projection_blocked
            || self.onboarding_open
            || self.navigation_open
            || !window.is_window_active()
            || (self.overlay.is_some() && !self.thread_is_docked(window))
            || self.prejoin.is_some()
            || self.incoming_visible()
            || self.snapshot.content != ContentState::Ready
        {
            return None;
        }
        self.snapshot.selected
    }
    fn observe_visible_message(&mut self, context: ConversationRef) {
        if self.message_ui.unread_jump.is_some_and(|(room, request)| {
            room == context
                && self.snapshot.message_panel.as_ref().is_some_and(|panel| {
                    panel.context == context
                        && panel.context_load.as_ref().is_some_and(|load| {
                            load.state == talk_core::ui::conversation::RemoteState::Loading(request)
                        })
                })
        }) {
            return;
        }
        if self
            .snapshot
            .message_panel
            .as_ref()
            .is_some_and(|panel| panel.context == context && panel.unread_queued)
        {
            return;
        }
        if self
            .message_ui
            .unread_hold
            .is_some_and(|(room, _)| room == context)
        {
            return;
        }
        if !self.snapshot.message_panel.as_ref().is_some_and(|p| {
            p.context == context && p.availability(MessageFeature::ReadReceipts).enabled()
        }) {
            return;
        }
        // GPUI's Bottom list uses an end sentinel for logical_scroll_top.
        // bounds_for_item then returns None even for painted rows. Use the
        // finite, scoped paint observation instead, never an unpainted ID.
        let id = self
            .message_ui
            .geometry
            .borrow()
            .visible_read
            .filter(|(painted_context, id)| {
                *painted_context == context
                    && self
                        .snapshot
                        .messages
                        .as_slice()
                        .iter()
                        .any(|row| row.local_id == *id)
            })
            .and_then(|(_, id)| {
                self.snapshot
                    .messages
                    .as_slice()
                    .iter()
                    .find(|message| message.local_id == id)
                    .map(|message| (id, message.metadata.read_through.unwrap_or(id)))
            });
        if let Some((id, effective)) = id
            && read_advances(self.message_ui.last_read, context, effective)
            && self.submit_message(MessageAction::MarkRead(id)).is_some()
        {
            self.message_ui.last_read = Some((context, effective));
        }
    }
    pub(super) fn release_unread_hold(&mut self, context: ConversationRef) {
        if self
            .message_ui
            .unread_hold
            .is_some_and(|(room, _)| room == context)
        {
            self.message_ui.unread_hold = None;
            self.message_ui.unread_prior = None;
            self.message_ui.last_read = None;
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
            feature_allows_intent(
                self.snapshot.content,
                value,
                feature,
                self.local_message_intents(),
            ),
            if !value.implemented
                || value.allowed == Decision::Denied
                || value.server == talk_core::ui::presentation::FeatureSupport::Unsupported
            {
                super::conversation_dialog::unavailable(value)
            } else if self.snapshot.content != ContentState::Ready {
                "Die Verbindung ist derzeit nicht bereit."
            } else {
                super::conversation_dialog::unavailable(value)
            },
        )
    }
    pub(super) fn message_feature_enabled(&self, feature: MessageFeature) -> bool {
        self.message_available(feature).is_enabled()
    }
    pub(super) fn can_start_thread(&self) -> bool {
        use talk_core::ui::presentation::FeatureSupport;
        self.message_feature_enabled(MessageFeature::Threads)
            || matches!(
                self.snapshot.content,
                ContentState::Ready | ContentState::Offline
            ) && self.local_message_intents()
                && self.snapshot.message_panel.as_ref().is_some_and(|panel| {
                    Some(panel.context) == self.snapshot.selected
                        && panel.availability(MessageFeature::Threads).implemented
                        && panel.availability(MessageFeature::Threads).server
                            == FeatureSupport::Supported
                        && panel.availability(MessageFeature::Threads).allowed != Decision::Denied
                })
    }
    /// Previously observed thread details permit local staging, never a server
    /// permission grant. The outbox revalidates after reconnecting.
    pub(super) fn can_change_thread(&self, context: ConversationRef) -> bool {
        use talk_core::ui::presentation::FeatureSupport;
        self.message_feature_enabled(MessageFeature::Threads)
            || matches!(
                self.snapshot.content,
                ContentState::Ready | ContentState::Offline
            ) && self.local_message_intents()
                && self.snapshot.message_panel.as_ref().is_some_and(|panel| {
                    panel.context == context
                        && panel.thread_details.is_some()
                        && panel.availability(MessageFeature::Threads).server
                            != FeatureSupport::Unsupported
                        && panel.availability(MessageFeature::Threads).allowed != Decision::Denied
                })
    }
    /// Offline thread content is a read-only cached observation. A previously
    /// observed root may be opened while fresh server authority is unavailable.
    pub(super) fn can_open_thread(&self, context: ConversationRef, root: u128) -> bool {
        use talk_core::ui::presentation::FeatureSupport;
        self.message_feature_enabled(MessageFeature::Threads)
            || matches!(
                self.snapshot.content,
                ContentState::Ready | ContentState::Offline
            ) && self.snapshot.selected == Some(context)
                && self.snapshot.message_panel.as_ref().is_some_and(|panel| {
                    panel.context == context
                        && panel.availability(MessageFeature::Threads).server
                            != FeatureSupport::Unsupported
                })
                && (self.thread_parent == Some((context, root))
                    || self.snapshot.messages.as_slice().iter().any(|message| {
                        (message.local_id == root
                            && (message.reply_count > 0 || message.metadata.thread == Some(root)))
                            || message.metadata.thread == Some(root)
                    }))
    }
    pub(super) fn local_message_intents(&self) -> bool {
        matches!(
            self.snapshot.content,
            ContentState::Ready | ContentState::Offline
        ) && self
            .snapshot
            .message_panel
            .as_ref()
            .is_some_and(|p| Some(p.context) == self.snapshot.selected && p.queue_available)
    }
    pub(super) fn conversation_search_open(&self) -> bool {
        self.message_ui.show_search && self.message_ui.context == self.snapshot.selected
    }
    pub(super) fn trace_chat_ui(&self, window: &Window, cx: &Context<Self>) {
        self.trace_message_menu(window);
        println!(
            "UI_SIDEBAR narrow={} visible={}",
            self.narrow_navigation(window),
            self.sidebar_visible(window),
        );
        println!(
            "UI_HISTORY_PAGING visible_top_demand_pending_attempts_failed={:?}",
            self.message_ui.history.borrow().diagnostic()
        );
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
            self.message_ui.menu.message(),
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
            self.local_message_intents(),
            self.snapshot
                .selected
                .is_some_and(|c| self.drafts.pending(c))
                || self.message_ui.pending_edit.is_some()
                || self.message_ui.editing.is_some(),
        )
    }
    pub(super) fn submit_message(&mut self, action: MessageAction) -> Option<RequestId> {
        let context = self.snapshot.selected?;
        let unread_from = if let MessageAction::MarkUnreadFrom(message) = &action {
            Some(*message)
        } else {
            None
        };
        let mark_unread = matches!(
            action,
            MessageAction::MarkUnread | MessageAction::MarkUnreadFrom(_)
        );
        let pin_intent = match &action {
            MessageAction::Pin(id) => Some((*id, true)),
            MessageAction::Unpin(id) => Some((*id, false)),
            _ => None,
        };
        let notice = action_notice(&action);
        let request = RequestId(self.message_ui.request.checked_add(1)?);
        self.message_ui.request = request.0;
        if self.submit(UiIntent::Message {
            request,
            context,
            action,
        }) {
            if mark_unread {
                self.message_ui.unread_hold = Some((context, request));
                self.message_ui.unread_prior = Some((request, self.message_ui.first_unread));
            }
            if let Some(message) = unread_from {
                self.message_ui.first_unread = Some((context, message));
                self.message_ui.geometry.borrow_mut().unread_visible = None;
            }
            if let Some((source, pin)) = pin_intent {
                self.admit_pin_intent(context, request, source, pin);
            }
            if let Some(notice) = notice {
                self.message_ui.pending_action = Some((request, notice));
            }
            Some(request)
        } else {
            None
        }
    }

    pub(super) fn submit_sidebar_unread(&mut self, context: ConversationRef) {
        if Some(context) == self.snapshot.selected {
            self.submit_message(MessageAction::MarkUnread);
            return;
        }
        let Some(next) = self.message_ui.request.checked_add(1) else {
            return;
        };
        self.message_ui.request = next;
        self.submit(UiIntent::MarkRoomUnread {
            request: RequestId(next),
            context,
        });
    }

    pub(super) fn begin_new_thread(
        &mut self,
        context: ConversationRef,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if Some(context) != self.snapshot.selected
            || !self.can_start_thread()
            || self.message_ui.editing.is_some()
            || self.drafts.pending(context)
            || self.has_staged_attachments()
            || !self.save_draft(cx)
        {
            return;
        }
        let previous = self.drafts.target(context);
        let target = ComposeTarget {
            new_thread: true,
            ..Default::default()
        };
        if !self.drafts.set_target(context, target) {
            self.report_draft_error(talk_core::ui::SubmitError::Busy, context);
            return;
        }
        let body = self.drafts.text(context).to_owned();
        if !self.load_composer_value(&body, cx) {
            self.drafts.set_target(context, previous);
            return;
        }
        self.composer.focus_handle(cx).focus(window);
    }
    pub(super) fn show_reaction_authors(
        &mut self,
        context: ConversationRef,
        message: u128,
        emoji: Text<64>,
        sticky: bool,
    ) {
        if Some(context) != self.snapshot.selected
            || !self.reaction_available(message).is_enabled()
            || !self.reaction_message(message).is_some_and(|row| {
                row.metadata
                    .reactions
                    .as_slice()
                    .iter()
                    .any(|reaction| reaction.emoji == emoji)
            })
        {
            return;
        }
        let cached = self.snapshot.message_panel.as_ref().is_some_and(|panel| {
            panel.context == context
                && panel
                    .reaction_authors
                    .as_slice()
                    .iter()
                    .any(|entry| entry.message == message && entry.emoji == emoji)
        });
        let busy = self
            .snapshot
            .message_panel
            .as_ref()
            .is_some_and(|panel| panel.reaction_authors_load.is_some());
        let sticky = sticky
            || self
                .message_ui
                .reaction_authors_visible
                .as_ref()
                .is_some_and(|(room, id, current, retained)| {
                    *room == context && *id == message && current == &emoji && *retained
                });
        self.message_ui.reaction_authors_visible = Some((context, message, emoji.clone(), sticky));
        if !cached && !busy {
            self.submit_message(MessageAction::ReactionAuthors { message, emoji });
        }
    }

    pub(super) fn hide_hovered_reaction_authors(
        &mut self,
        context: ConversationRef,
        message: u128,
        emoji: &Text<64>,
    ) {
        if self
            .message_ui
            .reaction_authors_visible
            .as_ref()
            .is_some_and(|(room, id, current, sticky)| {
                *room == context && *id == message && current == emoji && !sticky
            })
        {
            self.message_ui.reaction_authors_visible = None;
        }
    }
    pub(super) fn submit_open_reminder(&mut self, context: ConversationRef, message: u128) -> bool {
        let Some(next) = self.message_ui.request.checked_add(1) else {
            return false;
        };
        self.message_ui.request = next;
        self.submit(UiIntent::OpenReminder {
            request: RequestId(next),
            context,
            message,
        })
    }
    pub(super) fn submit_notification_message(
        &mut self,
        context: ConversationRef,
        message: u128,
    ) -> bool {
        let Some(next) = self.message_ui.request.checked_add(1) else {
            return false;
        };
        self.message_ui.request = next;
        self.submit(UiIntent::OpenNotificationMessage {
            request: RequestId(next),
            context,
            message,
        })
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
        if !may_replace_composer(
            &action,
            self.message_ui.editing,
            self.message_ui.pending_edit.is_some(),
        ) {
            self.feedback = Some(
                super::feedback::Feedback::info("Beende zuerst die aktuelle Bearbeitung.")
                    .in_conversation(context),
            );
            return;
        }
        let anchor = self.message_ui.list.logical_scroll_top();
        let prior_compose = self.compose_motion_id(context);
        match action {
            Action::Forward(action) => {
                self.forward_action(action, window, cx);
                cx.notify();
                return;
            }
            Action::Recovery(action) => self.recover_failed_draft(context, action, window, cx),
            Action::Silent => {
                self.content_ui.menu = false;
                let mut target = self.drafts.target(context);
                target.silent = !target.silent;
                self.drafts.set_target(context, target);
            }
            Action::Pins => {
                self.message_ui.show_pins = true;
                if self.snapshot.content == ContentState::Ready
                    && self.message_feature_enabled(MessageFeature::Pins)
                {
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
            Action::Menu(id) => self.toggle_message_menu(context, id, window, cx),
            Action::ThreadMenu(id) => self.toggle_thread_message_menu(context, id, window, cx),
            Action::ReactionPalette(id, source) => {
                self.toggle_reaction_palette(context, id, source, window, cx)
            }
            Action::React(message, emoji) => {
                self.toggle_reaction(message, emoji);
            }
            Action::ReactionAuthors(message, emoji) => {
                if self
                    .message_ui
                    .reaction_authors_visible
                    .as_ref()
                    .is_some_and(|(room, id, current, sticky)| {
                        *room == context && *id == message && *current == emoji && *sticky
                    })
                {
                    self.message_ui.reaction_authors_visible = None;
                } else {
                    self.show_reaction_authors(context, message, emoji, true);
                }
            }
            Action::MenuChoice(target, choice) => {
                self.choose_message_menu(target, choice, window, cx)
            }
            Action::RetryHistory => {
                self.request_older_history(true);
            }
            Action::RetryContext { request, target } => {
                if self
                    .snapshot
                    .message_panel
                    .as_ref()
                    .filter(|panel| panel.context == context)
                    .and_then(|panel| panel.context_load.as_ref())
                    .is_some_and(|load| current_context_retry(load, request, target))
                {
                    if self.message_ui.unread_jump == Some((context, request)) {
                        if let Some(next) =
                            self.submit_message(MessageAction::LocateUnreadAfter(target))
                        {
                            self.message_ui.unread_jump = Some((context, next));
                        }
                    } else {
                        self.submit_message(MessageAction::Context(target));
                    }
                }
            }
            Action::Reply(id) => {
                let previous = self.drafts.target(context);
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
                            new_thread: false,
                            silent: false,
                            private_reply: None,
                        },
                    ) {
                        self.report_draft_error(talk_core::ui::SubmitError::Busy, context);
                        return;
                    }
                    let value = self.drafts.text(context).to_owned();
                    if !self.load_composer_value(&value, cx) {
                        self.drafts.set_target(context, previous);
                        return;
                    }
                    self.message_ui.menu.clear();
                    self.composer.focus_handle(cx).focus(window);
                }
            }
            Action::PrivateReply(id) => self.begin_private_reply(context, id, cx),
            Action::ThreadReply(id) => self.reply_in_thread(context, id, window, cx),
            Action::ThreadEdit(id) => self.edit_in_thread(context, id, window, cx),
            Action::ThreadCancel => self.cancel_thread_compose(context, window, cx),
            Action::Edit(id) => {
                if self.save_draft(cx)
                    && let Some(message) = self.snapshot.messages.as_slice().iter().find(|m| {
                        m.local_id == id
                            && message_mutation_allowed(
                                m,
                                MessageFeature::Edit,
                                self.message_feature_enabled(MessageFeature::Edit),
                                self.local_message_intents(),
                            )
                    })
                {
                    let value = message.body.as_str().to_owned();
                    let Some(generation) = self.message_ui.edit_generation.checked_add(1) else {
                        return;
                    };
                    if !self.load_composer_value(&value, cx) {
                        return;
                    }
                    self.message_ui.edit_generation = generation;
                    self.message_ui.editing = Some(id);
                    self.message_ui.menu.clear();
                    self.composer.focus_handle(cx).focus(window);
                }
            }
            Action::CancelCompose => {
                let previous = self.drafts.target(context);
                if self.message_ui.editing.is_none() {
                    if !self.save_draft(cx) {
                        return;
                    }
                    if !self.drafts.set_target(context, ComposeTarget::default()) {
                        return;
                    }
                }
                let value = self.drafts.text(context).to_owned();
                if !self.load_composer_value(&value, cx) {
                    self.drafts.set_target(context, previous);
                    return;
                }
                self.message_ui.editing = None;
                self.message_ui.time_target = None;
                self.message_ui.time_thread = None;
                self.message_ui.scheduled_edit = None;
            }
            Action::Command(command) => {
                if matches!(command, MessageAction::Scheduled | MessageAction::Reminders) {
                    self.content_ui.menu = false;
                }
                if matches!(command, MessageAction::Scheduled) {
                    self.message_ui.show_scheduled = true;
                }
                self.submit_message(command);
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
            Action::CyclePin { current, forward } => self.cycle_pin(context, current, forward),
            Action::TogglePinStrip => self.toggle_pin_strip(context),
            Action::Locate(id) => {
                self.message_ui.unread_jump = None;
                if self.locate_message_smooth(context, id) {
                    if self.overlay == Some(Overlay::Files) {
                        self.handle(LocalAction::Close, window, cx);
                    }
                    cx.notify();
                    return;
                }
                if self.submit_message(MessageAction::Context(id)).is_some()
                    && self.overlay == Some(Overlay::Files)
                {
                    self.handle(LocalAction::Close, window, cx);
                }
            }
            Action::JumpUnread => {
                if let Some((_, first)) = self
                    .message_ui
                    .first_unread
                    .filter(|(room, _)| *room == context)
                    && self.locate_message_smooth(context, first)
                {
                    cx.notify();
                    return;
                }
                if let Some((_, marker)) = self
                    .message_ui
                    .unread_marker
                    .filter(|(room, _)| *room == context)
                    && self.snapshot.content == ContentState::Ready
                    && let Some(request) =
                        self.submit_message(MessageAction::LocateUnreadAfter(marker))
                {
                    self.message_ui.unread_jump = Some((context, request));
                }
            }
            Action::OlderThread(root) => {
                if self.thread_parent == Some((context, root)) {
                    let request = self.submit_message(MessageAction::ThreadOlder(root));
                    self.message_ui.thread_load = Some(ThreadLoad {
                        context,
                        root,
                        request,
                    });
                }
            }
            Action::OpenLocation(message, placeholder, id) => {
                self.submit(UiIntent::OpenLocation {
                    context,
                    message,
                    placeholder,
                    id,
                });
            }
            Action::Translate(message) => self.translate_message(context, message),
            Action::HideTranslation(message) => {
                if !self.message_ui.hidden_translations.contains(&message)
                    && self.message_ui.hidden_translations.len() < 100
                {
                    self.message_ui.hidden_translations.push(message);
                }
            }
            Action::Schedule => {
                self.message_ui.scheduled_edit = None;
                self.open_message_deadline(None, None, window, cx);
            }
            Action::EditScheduled(id) => {
                self.open_scheduled_edit(id, window, cx);
            }
            Action::ToggleScheduled => {
                self.message_ui.show_scheduled = !self.message_ui.show_scheduled;
                if self.message_ui.show_scheduled {
                    self.submit_message(MessageAction::Scheduled);
                }
            }
            Action::CancelTime => {
                let thread = self.message_ui.time_thread;
                self.message_ui.time_target = None;
                self.message_ui.time_thread = None;
                self.message_ui.picker = None;
                self.message_ui.scheduled_edit = None;
                self.message_ui.time_error = None;
                self.restore_deadline_focus(thread, window, cx);
            }
            Action::Remind { message, thread } => {
                self.message_ui.scheduled_edit = None;
                self.open_message_deadline(Some(message), thread, window, cx);
            }
            Action::SubmitTime => {
                self.submit_custom_deadline(
                    talk_core::preferences::time::RepeatedTime::Ask,
                    window,
                    cx,
                );
            }
            Action::SubmitRepeatedTime(repeated) => {
                self.submit_custom_deadline(repeated, window, cx)
            }
            Action::DatePicker(action) => self.date_picker_action(context, action, window, cx),
            Action::ChooseTime(choice) => self.submit_message_deadline(choice, window, cx),
            Action::InsertMention(identifier) => {
                self.composer
                    .update(cx, |i, cx| i.insert_mention(&identifier, cx));
                self.composer.focus_handle(cx).focus(window);
            }
            Action::InsertThreadMention(identifier) => {
                self.thread_composer
                    .update(cx, |input, cx| input.insert_mention(&identifier, cx));
                self.thread_composer.focus_handle(cx).focus(window);
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
        if self.message_ui.pending_edit.is_some() {
            return true;
        }
        if let Some(body) = self.composer_submission(cx)
            && let Some(request) = self.submit_message(MessageAction::Edit {
                message,
                body: body.clone(),
            })
        {
            self.message_ui.pending_edit = Some(super::state::PendingEdit {
                request,
                message,
                generation: self.message_ui.edit_generation,
                editor_generation: self.composer.read(cx).edit_generation(),
                body,
            });
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
            .child(
                self.input_surface(&self.message_ui.search, cx)
                    .flex_1()
                    .min_w_0()
                    .child(self.scaled_icon("search", 18., t.muted))
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .child(self.message_ui.search.clone()),
                    ),
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
        self.message_timeline_with_header(constrained, None, false, cx)
    }
    pub(super) fn call_message_timeline(&self, cx: &Context<Self>) -> Div {
        self.message_timeline_with_header(false, None, true, cx)
    }
    pub(super) fn conversation_timeline(
        &self,
        header: Div,
        constrained: bool,
        cx: &Context<Self>,
    ) -> Div {
        if CollectionState::new(
            self.snapshot.content,
            !self.snapshot.messages.as_slice().is_empty(),
        )
        .shows_content()
        {
            self.message_timeline_with_header(constrained, Some(header), false, cx)
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
        compact: bool,
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
        let state = CollectionState::new(
            self.snapshot.content,
            !self.snapshot.messages.as_slice().is_empty(),
        );
        let context_pending = self
            .snapshot
            .message_panel
            .as_ref()
            .filter(|panel| Some(panel.context) == self.snapshot.selected)
            .and_then(|panel| panel.context_load.as_ref())
            .is_some_and(|load| {
                matches!(
                    load.state,
                    talk_core::ui::conversation::RemoteState::Loading(_)
                        | talk_core::ui::conversation::RemoteState::Saving(_)
                )
            });
        if state.shows_content() {
            // Keep the same virtualized list and scroll anchor during refresh
            // and temporary failures; retained rows confer no mutation rights.
            body = body.child(if let Some(header) = header.take() {
                let inert = self.background_inert;
                super::shell::chat_header::scroll_under_header(
                    header,
                    constrained,
                    !self.theme.reduced_transparency && !cfg!(target_os = "windows"),
                    cx.processor(move |this, inset, _, cx| {
                        this.in_surface(inert, |this| this.message_history(inset, compact, cx))
                    }),
                )
                .into_any_element()
            } else {
                self.message_history(if compact { self.sp(8.) } else { px(0.) }, compact, cx)
                    .into_any_element()
            });
        } else if state.confirmed_empty() && !context_pending {
            let has_older = self.snapshot.message_panel.as_ref().is_some_and(|panel| {
                Some(panel.context) == self.snapshot.selected && panel.older.is_some()
            });
            body = if has_older {
                body.child(
                    div()
                        .flex_1()
                        .flex()
                        .items_center()
                        .justify_center()
                        .child(self.history_boundary(cx)),
                )
            } else {
                body.child(
                    div().flex_1().min_h_0().child(
                        self.state_note(
                            "chat",
                            "Noch keine Nachrichten.",
                            "Hier erscheinen die Nachrichten dieses Gesprächs.",
                        )
                        .size_full(),
                    ),
                )
            };
        } else if state.initial_loading() || context_pending {
            body = body.child(self.collection_skeleton(Skeleton::Messages, "messages"));
        } else {
            body = body.child(div().flex_1());
        }
        if !matches!(
            self.snapshot.content,
            ContentState::Ready | ContentState::Loading | ContentState::Offline
        ) && let Some(context) = self.snapshot.selected
        {
            body = body.child(div().px_3().py_2().child(self.collection_feedback(
                state,
                "retry-content",
                LocalAction::Intent(UiIntent::Refresh(context)),
                cx,
            )));
        }
        if let Some(panel) = self
            .snapshot
            .message_panel
            .as_ref()
            .filter(|p| Some(p.context) == self.snapshot.selected)
        {
            if let Some(load) = &panel.context_load
                && let Some(status) = context_status(load)
            {
                body = body.child(
                    div()
                        .px_6()
                        .py_2()
                        .flex()
                        .flex_wrap()
                        .items_center()
                        .gap_3()
                        .child(
                            div()
                                .flex_1()
                                .min_w_0()
                                .child(self.hint(self.l(status.label))),
                        )
                        .when_some(status.retry, |d, request| {
                            d.child(self.button(
                                "retry-context",
                                "Erneut versuchen",
                                ButtonStyle::Secondary,
                                true,
                                LocalAction::MessageIn(
                                    panel.context,
                                    Action::RetryContext {
                                        request,
                                        target: load.target,
                                    },
                                ),
                                cx,
                            ))
                        }),
                );
            }
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
    fn message_history(
        &self,
        header_inset: gpui::Pixels,
        compact: bool,
        cx: &Context<Self>,
    ) -> Div {
        if self.message_ui.geometry.borrow().compact != compact {
            let anchor = self.message_ui.list.logical_scroll_top();
            let count = self.message_ui.list.item_count();
            self.message_ui.geometry.borrow_mut().compact = compact;
            if count > 0 {
                // Compact call rows have different heights. Invalidate cached
                // geometry while retaining the current logical scroll anchor.
                self.message_ui.list.splice(0..count, count);
                self.message_ui.list.scroll_to(anchor);
            }
        }
        let previous = self.message_ui.geometry.borrow().header_inset;
        let boundary_inset = self.sp(32.);
        if previous != header_inset
            || self.message_ui.geometry.borrow().boundary_inset != boundary_inset
        {
            let mut anchor = self.message_ui.list.logical_scroll_top();
            self.message_ui.geometry.borrow_mut().header_inset = header_inset;
            self.message_ui.geometry.borrow_mut().boundary_inset = boundary_inset;
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
        let group_ends: Vec<_> = self
            .media_spans()
            .iter()
            .filter_map(|span| {
                Some((
                    span.start,
                    span.end,
                    self.snapshot.messages.as_slice().get(span.start)?.local_id,
                    self.snapshot
                        .messages
                        .as_slice()
                        .get(span.end.checked_sub(1)?)?
                        .local_id,
                ))
            })
            .collect();
        let layout_arrival = self.message_ui.arrival.clone();
        let layout_pins = self.pin_locate_layout();
        let wheel_arrival = self.message_ui.arrival.clone();
        let wheel_pins = self.message_ui.pins.cancellation();
        let keyboard_pins = self.message_ui.pins.cancellation();
        let wheel_history = self.message_ui.history.clone();
        let keyboard_history = self.message_ui.history.clone();
        let layout_history = self.message_ui.history.clone();
        let keyboard_arrival = self.message_ui.arrival.clone();
        let layout_list = self.message_ui.list.clone();
        let keyboard_list = self.message_ui.list.clone();
        let keyboard_focus = self.navigation.history_focus();
        let layout_focus = keyboard_focus.clone();
        let reveal = self.navigation.reveal.clone();
        let reduced = self.reduced_motion;
        let layout_geometry = self.message_ui.geometry.clone();
        let context = self.snapshot.selected;
        let unread_target = self
            .message_ui
            .first_unread
            .filter(|(room, _)| Some(*room) == context);
        let unread_marker = self
            .message_ui
            .unread_marker
            .filter(|(room, _)| Some(*room) == context);
        let show_unread_jump = unread_target.is_some()
            && self.message_ui.geometry.borrow().unread_visible == Some(false)
            || unread_target.is_none() && unread_marker.is_some();
        self.message_ui.geometry.borrow_mut().painted.clear();
        let inert = self.background_inert;
        div()
            .relative()
            .flex_1()
            .min_h_0()
            .track_focus(&keyboard_focus.clone().tab_stop(!self.background_inert))
            .on_scroll_wheel(move |event, _, _| {
                wheel_pins();
                if event.delta.pixel_delta(px(20.)).y > px(0.) {
                    wheel_arrival.borrow_mut().manual_scroll();
                    wheel_history.borrow_mut().upward();
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
                if matches!(event.keystroke.key.as_str(), "pageup" | "home") {
                    keyboard_history.borrow_mut().upward();
                }
                keyboard_pins();
                keyboard_arrival.borrow_mut().manual_scroll();
                cx.stop_propagation();
                window.refresh();
            })
            .on_children_prepainted(move |_, window, cx| {
                let viewport = layout_list.viewport_bounds();
                let mut geometry = layout_geometry.borrow_mut();
                let inset = header_inset.min(viewport.size.height);
                let visible = gpui::Bounds::from_corners(
                    gpui::point(viewport.left(), viewport.top() + inset),
                    viewport.bottom_right(),
                );
                geometry.visible_read = context.and_then(|context| {
                    visible_read(&geometry.painted, visible).map(|id| (context, group_ends.iter().find(|(_,_,head,_)|*head==id).map_or(id,|(_,_,_,last)|*last)))
                });
                let unread_visible = unread_target.is_some_and(|(_, target)| {
                    geometry.painted.iter().any(|(_, id, bounds, _)| {
                        *id == target && bounds.bottom() > visible.top() && bounds.top() < visible.bottom()
                    })
                });
                if unread_target.is_some() && geometry.unread_visible != Some(unread_visible) {
                    geometry.unread_visible = Some(unread_visible);
                    window.refresh();
                }
                let at_bottom = viewport.size.height > px(0.)
                    && geometry.painted.iter().any(|(index, _, bounds, _)| {
                        group_ends.iter().find(|(start,_,_,_)|start==index).map_or_else(||index.checked_add(1),|(_,end,_,_)|Some(*end)) == Some(layout_list.item_count())
                            && bounds.bottom() <= viewport.bottom() + px(1.)
                    });
                let painted_anchor = geometry
                    .painted
                    .iter()
                    .find(|(_, _, bounds,_)| bounds.bottom() > viewport.top())
                    .map(|(index, id, bounds,_)| (*index, *id, viewport.top() - bounds.top()));
                geometry.anchor = painted_anchor.map(|(_, id, offset)| (id, offset));
                layout_history.borrow_mut().layout(
                    viewport.size.height > px(0.)
                        && geometry.painted.iter().any(|(index, _, bounds,_)| {
                            *index == 0 && bounds.top() >= viewport.top() - px(1.)
                        }),
                );
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
                    layout_history.borrow_mut().layout(false);
                }
                let now = Instant::now();
                arrival.after_scroll_layout(now, at_bottom);
                let followed = arrival.after_layout(now, reduced);
                if followed {
                    layout_history.borrow_mut().layout(false);
                    layout_list.scroll_to(ListOffset {
                        item_ix: layout_list.item_count(),
                        offset_in_item: px(0.),
                    });
                }
                layout_pins(painted_anchor, window);
                if followed || arrival.following() || arrival.animate_next_frame(now) {
                    window.request_animation_frame();
                }
            })
            .child(
                list(
                    self.message_ui.list.clone(),
                    cx.processor(move |this, index: usize, _, cx| {
                        this.in_surface(inert, |this| {
                            if this.media_spans().iter().any(|span|span.start<index&&index<span.end){return div().h(px(0.)).into_any_element();}
                            this.snapshot
                                .messages
                                .as_slice()
                                .get(index)
                                .map(|m: &MessageView| {
                                    let geometry = this.message_ui.geometry.clone();
                                    let id = m.local_id;
                                    let canonical=matches!(m.delivery,talk_core::ui::presentation::DeliveryState::Received|talk_core::ui::presentation::DeliveryState::ServerAccepted);
                                    let progress = this
                                        .message_ui
                                        .arrival
                                        .borrow()
                                        .progress(m.local_id, Instant::now())
                                        .map(gpui::ease_out_quint());
                                    let continuation =
                                        index.checked_sub(1).is_some_and(|previous| {
                                            this.snapshot
                                                .messages
                                                .as_slice()
                                                .get(previous)
                                                .is_some_and(|previous| {
                                                    row::grouped_message(
                                                        previous,
                                                        m,
                                                        this.settings_port.same_local_day(
                                                            previous.metadata.timestamp,
                                                            m.metadata.timestamp,
                                                        ),
                                                    )
                                                })
                                        });
                                    let mut bubble = this.message_row(m, continuation, compact, cx);
                                    if let Some(progress) = progress {
                                        // Paint-only rise: a relative offset inside the
                                        // measured wrapper leaves row height, list
                                        // measurement and the recorded anchor bounds intact.
                                        // Own sends rise from the composer a little further.
                                        let rise = this.sp(if m.own { 14. } else { 8. });
                                        bubble = bubble
                                            .relative()
                                            .top(rise * (1. - progress))
                                            .opacity(0.2 + 0.8 * progress);
                                    }
                                    let row = div()
                                        .child(
                                            div()
                                                .when(index == 0, |d| {
                                                    d.child(this.history_boundary(cx))
                                                })
                                                .child(bubble),
                                        )
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
                                                geometry.painted.push((index, id, bounds,canonical));
                                            }
                                        });
                                    if progress.is_some() {
                                        let frames = this.motion_frame_counter();
                                        super::motion::count_frame(&frames);
                                    }
                                    row.into_any_element()
                                })
                                .unwrap_or_else(|| div().into_any_element())
                        })
                    }),
                )
                .size_full(),
            )
            .when(show_unread_jump, |area| {
                let Some(context) = context else { return area };
                let loading = self.message_ui.unread_jump.is_some_and(|(room, request)| {
                    room == context && self.snapshot.message_panel.as_ref().is_some_and(|panel| {
                        panel.context_load.as_ref().is_some_and(|load| {
                            load.state == talk_core::ui::conversation::RemoteState::Loading(request)
                        })
                    })
                });
                area.child(
                    div().absolute().right_4().bottom_4().child(
                        self.button(
                            "jump-unread",
                            "Zu ungelesenen springen",
                            ButtonStyle::Secondary,
                            if loading {
                                Availability::pending()
                            } else {
                                Availability::when(unread_target.is_some() || self.snapshot.content == ContentState::Ready, "Die Verbindung ist derzeit nicht bereit.")
                            },
                            LocalAction::MessageIn(context, Action::JumpUnread),
                            cx,
                        )
                        .rounded_full()
                        .border_1()
                        .border_color(self.theme.border)
                        .shadow(self.theme.elevation(super::theme::Elevation::Popover)),
                    ),
                )
            })
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
        if !pins && self.snapshot.active_account.is_some() {
            body = body.child(self.button(
                "browse-cloud-files",
                "Alle Dateien anzeigen",
                ButtonStyle::Secondary,
                Availability::when(
                    self.snapshot.content_panel.file_browser_supported != Some(false),
                    "Diese Dateioption ist derzeit nicht verfügbar.",
                ),
                LocalAction::Content(super::content::Action::OpenCloudBrowser),
                cx,
            ));
        }
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
        let can_read = if pins {
            self.message_feature_enabled(MessageFeature::Pins)
        } else {
            available.enabled()
        };
        if !can_read {
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
                            Availability::when(
                                self.message_feature_enabled(MessageFeature::Pins)
                                    && decision_allows_intent(
                                        panel.pin_change,
                                        self.local_message_intents(),
                                    )
                                    && (!panel.state.pending() || self.local_message_intents()),
                                "Anheften ist derzeit nicht verfügbar.",
                            ),
                            LocalAction::MessageIn(
                                context,
                                Action::Command(MessageAction::Unpin(message.local_id)),
                            ),
                            cx,
                        )),
                );
            }
            if panel.pinned_messages.as_slice().is_empty()
                && panel.pins_loaded
                && !panel.pins_state.pending()
            {
                body = body.child(self.hint(self.l("Keine angehefteten Nachrichten geladen.")));
            }
        } else {
            body = body.child(self.shared_files(cx));
        }
        match if pins { panel.pins_state } else { panel.state } {
            talk_core::ui::conversation::RemoteState::Loading(_) => {
                let has_content = if pins {
                    !panel.pinned_messages.as_slice().is_empty()
                } else {
                    !panel.shared_files.as_slice().is_empty()
                };
                if !has_content {
                    body = body.child(self.collection_skeleton(Skeleton::Rows, "shared-content"));
                }
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
            Availability::when(
                self.snapshot.content == ContentState::Ready && available.enabled(),
                if self.snapshot.content == ContentState::Ready {
                    super::conversation_dialog::unavailable(available)
                } else {
                    "Die Verbindung ist derzeit nicht bereit."
                },
            ),
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
                "{} {}",
                self.l("Maximale Dateigröße:"),
                super::localization::file_size(self.locale, Some(panel.file_limit))
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
                                self.download_available(),
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
    fn message_text(
        &self,
        message: &MessageView,
        thread: bool,
        cx: &Context<Self>,
    ) -> super::selectable::SelectableText {
        let context = self.snapshot.selected.expect("message context");
        // The current room privacy projection also gates read-only export.
        // Unknown policy, cached-only content and obsolete scopes cannot copy.
        let permitted = !self.background_inert
            && !self.projection_blocked
            && !self.update_restart_pending
            && self.snapshot.content == ContentState::Ready
            && self
                .selected()
                .is_some_and(|room| room.draft_clipboard == Decision::Allowed)
            && !message.metadata.deleted;
        let text = super::selectable::SelectableText::new(
            row::surface_id("message-text", context, message.local_id, 0, thread),
            message.body.as_str(),
            self.text_selection.clone(),
            self.theme,
            permitted,
        );
        if compact_system_message(message) {
            text
        } else {
            let handles = if permitted {
                (0..self
                    .text_selection
                    .read(cx)
                    .link_count(message.body.as_str()))
                    .map(|ordinal| {
                        // A timeline row and a rendered thread copy must never
                        // share focus ownership, even for the same message.
                        let id = row::surface_id(
                            "message-link",
                            context,
                            message.local_id,
                            ordinal as u128,
                            thread,
                        );
                        self.control_focus.get(id, cx)
                    })
                    .collect()
            } else {
                Vec::new()
            };
            text.rich(message.body.as_str(), f32::from(self.scale) / 100.)
                .links(context, message.local_id, message.body.clone(), handles)
        }
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
            .or_else(|| {
                target
                    .new_thread
                    .then(|| scoped_id("compose-mode", context, 4, 0))
            })
            .or_else(|| {
                target
                    .private_reply
                    .as_ref()
                    .map(|private| scoped_id("compose-mode", context, 3, private.message))
            })
    }
    pub(super) fn composer_context(&self, cx: &Context<Self>) -> Vec<gpui::AnyElement> {
        // The composer owns row spacing; absent context must not leave empty flex items.
        let mut rows = Vec::new();
        let Some(context) = self.snapshot.selected else {
            return rows;
        };
        if let Some(card) = self.summary_card(context, cx) {
            rows.push(card.into_any_element());
        }
        let target = self.drafts.target(context);
        // Explicit reply/edit context and silent sending are the only persistent
        // composer states; each is one slim strip with its own dismissal.
        if let Some(motion_id) = self.compose_motion_id(context) {
            let strip = if target.new_thread {
                self.composer_strip(
                    Some("chat"),
                    "Neuer Thread · Titel aus der Nachricht",
                    "cancel-compose-mode",
                    LocalAction::Message(Action::CancelCompose),
                    cx,
                )
            } else if let Some(private) = target.private_reply.as_ref() {
                self.private_reply_preview(context, private, cx)
            } else if self.message_ui.editing.is_none()
                && target.thread.is_none()
                && let Some(reply) = target.reply_to
            {
                self.composer_reply_preview(context, reply, cx)
            } else {
                let (icon, label) = if self.message_ui.editing.is_some() {
                    (None, "Nachricht bearbeiten")
                } else {
                    (Some("chat"), "Antwort im Thread")
                };
                self.composer_strip(
                    icon,
                    label,
                    "cancel-compose-mode",
                    LocalAction::Message(Action::CancelCompose),
                    cx,
                )
            };
            rows.push(self.reveal_from_action(motion_id, strip).into_any_element());
        }
        if target.silent {
            rows.push(
                self.composer_strip(
                    Some("moon"),
                    "Wird leise gesendet",
                    "composer-silent-off",
                    LocalAction::Message(Action::Silent),
                    cx,
                )
                .into_any_element(),
            );
        }
        if let Some(panel) = &self.snapshot.message_panel {
            if !panel.uploads.as_slice().is_empty() {
                rows.push(self.upload_strip(cx).into_any_element());
            }
            if panel.file_error == Some(talk_core::ui::messaging::UploadFailure::TooMany) {
                rows.push(
                    div()
                        .flex()
                        .items_center()
                        .gap_2()
                        .child(
                            self.hint(self.l("Du kannst höchstens 8 Dateien auf einmal anhängen.")),
                        )
                        .child(self.icon_button(
                            "dismiss-file-error",
                            ("x", "Schließen"),
                            ButtonStyle::Quiet,
                            Availability::available(),
                            LocalAction::Message(Action::Command(MessageAction::DismissUpload)),
                            cx,
                        ))
                        .into_any_element(),
                );
            }
            let show_mentions = self.composer_active
                && self
                    .message_ui
                    .mention_ready
                    .as_ref()
                    .is_some_and(|(thread, q)| {
                        !thread && self.composer.read(cx).mention_query() == Some(q.as_str())
                    });
            for (index, mention) in panel
                .mentions
                .as_slice()
                .iter()
                .take(if show_mentions { 8 } else { 0 })
                .enumerate()
            {
                rows.push(
                    self.button(
                        ("mention-choice", index),
                        mention.label.as_str().to_owned(),
                        ButtonStyle::Quiet,
                        true,
                        LocalAction::Message(Action::InsertMention(
                            mention.identifier.as_str().to_owned(),
                        )),
                        cx,
                    )
                    .into_any_element(),
                );
            }
            // Retained counts are immediate; expanding asks the owner to refresh.
            if !panel.scheduled.as_slice().is_empty() {
                rows.push(
                    self.button(
                        "scheduled-bar",
                        self.lf(
                            "Geplant ({n})",
                            &[("n", &panel.scheduled.as_slice().len().to_string())],
                        ),
                        ButtonStyle::Quiet,
                        self.message_available(MessageFeature::Schedules),
                        LocalAction::MessageIn(context, Action::ToggleScheduled),
                        cx,
                    )
                    .into_any_element(),
                );
            }
            // Server lists requested from the + menu stay grouped in one card each.
            let scheduled: Vec<_> = panel
                .scheduled
                .as_slice()
                .iter()
                .take(if self.message_ui.show_scheduled { 8 } else { 0 })
                .map(|scheduled| {
                    let deadline = if scheduled.timestamp == 0 {
                        scheduled.failed_original_timestamp
                    } else {
                        Some(scheduled.timestamp)
                    };
                    let label = deadline
                        .and_then(|timestamp| self.settings_port.local_datetime(timestamp))
                        .and_then(|time| {
                            super::localization::local_datetime(self.locale, time.as_str())
                        })
                        .map(|time| format!("{} · {}", time, scheduled.body.as_str()))
                        .unwrap_or_else(|| scheduled.body.as_str().to_owned());
                    let label = if scheduled.timestamp == 0 {
                        format!("{} · {}", self.l("Senden fehlgeschlagen"), label)
                    } else {
                        label
                    };
                    let confirmed = u64::try_from(scheduled.id).is_ok();
                    self.composer_list_row(
                        label,
                        div()
                            .flex()
                            .items_center()
                            .gap_2()
                            .child(self.button(
                                scoped_id("schedule-edit", context, scheduled.id, 0),
                                "Bearbeiten",
                                ButtonStyle::Quiet,
                                if !scheduled.editable {
                                    Availability::when(
                                        false,
                                        "Dieser Text ist für die Bearbeitung zu lang.",
                                    )
                                } else {
                                    self.message_available(MessageFeature::Schedules)
                                },
                                LocalAction::MessageIn(
                                    context,
                                    Action::EditScheduled(scheduled.id),
                                ),
                                cx,
                            ))
                            .child(self.button(
                                scoped_id("schedule-cancel", context, scheduled.id, 0),
                                "Planung abbrechen",
                                ButtonStyle::Quiet,
                                if !confirmed {
                                    Availability::when(false, "Die Planung wird noch gespeichert.")
                                } else {
                                    self.message_available(MessageFeature::Schedules)
                                },
                                LocalAction::MessageIn(
                                    context,
                                    Action::Command(MessageAction::CancelScheduled(scheduled.id)),
                                ),
                                cx,
                            )),
                    )
                })
                .collect();
            if !scheduled.is_empty() {
                rows.push(self.composer_card().children(scheduled).into_any_element());
            }
            let reminders: Vec<_> = panel
                .reminders
                .as_slice()
                .iter()
                .take(if self.composer_active { 8 } else { 0 })
                .map(|reminder| {
                    self.composer_list_row(
                        self.settings_port
                            .local_datetime(reminder.timestamp)
                            .and_then(|time| {
                                super::localization::local_datetime(self.locale, time.as_str())
                            })
                            .map(|time| format!("{} · {}", self.l("Erinnerung"), time))
                            .unwrap_or_else(|| self.l("Erinnerung").to_owned()),
                        self.button(
                            scoped_id("reminder-cancel", context, reminder.message, 0),
                            "Erinnerung löschen",
                            ButtonStyle::Quiet,
                            self.message_available(MessageFeature::Reminders),
                            LocalAction::Message(Action::Command(MessageAction::Reminder {
                                message: reminder.message,
                                timestamp: None,
                            })),
                            cx,
                        ),
                    )
                })
                .collect();
            if !reminders.is_empty() {
                rows.push(self.composer_card().children(reminders).into_any_element());
            }
        }
        rows.extend(self.message_deadline_card_on(false, cx));
        rows.extend(self.failed_draft_card(cx));
        rows
    }
    fn quote_marker(&self) -> Div {
        div()
            .absolute()
            .left_0()
            .top(px(3.))
            .bottom(px(3.))
            .w(px(3.))
            .rounded_full()
            .bg(self.theme.accent)
    }
    pub(super) fn quote_preview(&self, author: &str, body: Option<&str>) -> Div {
        div()
            .relative()
            .min_w_0()
            .pl(px(11.))
            .py(px(2.))
            .flex()
            .flex_col()
            .gap(px(2.))
            .child(self.quote_marker())
            .child(
                div()
                    .truncate()
                    .text_size(self.sp(12.))
                    .line_height(self.sp(18.))
                    .font_weight(FontWeight::MEDIUM)
                    .text_color(self.theme.muted)
                    .child(author.to_owned()),
            )
            .when_some(body, |d, body| {
                d.child(
                    div()
                        .text_size(self.sp(13.))
                        .line_height(self.sp(18.))
                        .text_color(self.theme.muted)
                        .line_clamp(2)
                        .overflow_hidden()
                        .child(body.to_owned()),
                )
            })
    }
    fn composer_reply_preview(
        &self,
        context: ConversationRef,
        reply: u128,
        cx: &Context<Self>,
    ) -> Div {
        // Read the current projection rather than retaining a copied excerpt:
        // edits, deletion and access loss must also update the composer preview.
        let source = self
            .snapshot
            .messages
            .as_slice()
            .iter()
            .find(|message| message.local_id == reply && !message.metadata.deleted)
            .filter(|_| self.snapshot.content != ContentState::Denied);
        let (author, body) = source.map_or(
            (
                self.l("Ursprüngliche Nachricht nicht mehr verfügbar."),
                None,
            ),
            |message| {
                (
                    message.author.as_str(),
                    Some(if message.body.as_str().is_empty() {
                        message
                            .attachments
                            .as_slice()
                            .first()
                            .map_or("", |file| file.name.as_str())
                    } else {
                        message.body.as_str()
                    }),
                )
            },
        );
        div()
            .flex()
            .items_center()
            .gap_2()
            .pl_3()
            .pr_1()
            .py_2()
            .rounded(px(8.))
            .bg(self.theme.subtle)
            .child(self.quote_preview(author, body).flex_1())
            .child(self.icon_button(
                "cancel-compose-mode",
                ("x", "Abbrechen"),
                ButtonStyle::Quiet,
                Availability::available(),
                LocalAction::MessageIn(context, Action::CancelCompose),
                cx,
            ))
    }
    /// One slim, dismissible composer state line: icon, short label, close icon.
    fn composer_strip(
        &self,
        icon: Option<&'static str>,
        label: &'static str,
        close_id: &'static str,
        close: LocalAction,
        cx: &Context<Self>,
    ) -> Div {
        let t = self.theme;
        div()
            .relative()
            .flex()
            .items_center()
            .gap_2()
            .pl(px(20.))
            .pr_1()
            .py_1()
            .rounded(px(8.))
            .bg(t.subtle)
            .child(self.quote_marker().left(px(8.)).top(px(8.)).bottom(px(8.)))
            .when_some(icon, |d, icon| {
                d.child(self.scaled_icon(icon, 16., t.muted))
            })
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .truncate()
                    .text_size(self.sp(13.))
                    .text_color(t.text)
                    .child(self.l(label).to_owned()),
            )
            .child(self.icon_button(
                close_id,
                ("x", "Abbrechen"),
                ButtonStyle::Quiet,
                Availability::available(),
                close,
                cx,
            ))
    }
    fn composer_card(&self) -> Div {
        self.style_card(div(), super::controls::CardStyle::Outlined)
            .flex()
            .flex_col()
            .bg(self.theme.surface)
            .overflow_hidden()
    }
    fn composer_list_row(&self, text: String, action: impl IntoElement) -> Div {
        div()
            .flex()
            .items_center()
            .justify_between()
            .gap_2()
            .pl_3()
            .pr_1()
            .py_1()
            .child(div().flex_1().min_w_0().text_size(self.sp(13.)).child(text))
            .child(action)
    }
}
// One local guard covers deliberate writes until the correlated completion.
// Reads and send/edit keep their existing, separately owned result lifecycles.
fn action_notice(action: &MessageAction) -> Option<Option<&'static str>> {
    Some(match action {
        MessageAction::Pin(_)
        | MessageAction::Unpin(_)
        | MessageAction::Delete(_)
        | MessageAction::MarkUnread
        | MessageAction::MarkUnreadFrom(_)
        | MessageAction::React { .. }
        | MessageAction::ShareUpload { .. } => None,
        MessageAction::Forward { .. } => Some("Nachricht weitergeleitet."),
        MessageAction::Reminder {
            timestamp: Some(_), ..
        } => Some("Erinnerung gespeichert."),
        MessageAction::Reminder {
            timestamp: None, ..
        } => Some("Erinnerung entfernt."),
        MessageAction::Schedule { .. } => Some("Nachricht geplant."),
        MessageAction::EditScheduled { .. } => Some("Planung aktualisiert."),
        MessageAction::CancelScheduled(_) => Some("Geplante Nachricht entfernt."),
        _ => return None,
    })
}
// Talk file_shared system messages still carry visible file content. Keep
// ordinary activity notices compact without bypassing the attachment renderer.
fn compact_system_message(message: &MessageView) -> bool {
    message.metadata.system
        && (message.metadata.deleted
            || !message
                .metadata
                .rich
                .as_slice()
                .iter()
                .any(|object| object.kind.as_str() == "file"))
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
fn may_replace_composer(action: &Action, editing: Option<u128>, pending_edit: bool) -> bool {
    !matches!(
        action,
        Action::Reply(_) | Action::PrivateReply(_) | Action::Edit(_)
    ) || (editing.is_none() && !pending_edit)
}

fn reply_allowed(
    message: &MessageView,
    content: ContentState,
    send: Decision,
    reply_feature: bool,
    queue_available: bool,
    pending: bool,
) -> bool {
    message_actions_allowed(message)
        && message.metadata.replyable
        && (content == ContentState::Ready || content == ContentState::Offline && queue_available)
        && decision_allows_intent(send, queue_available)
        && reply_feature
        && !pending
}

fn decision_allows_intent(decision: Decision, queue_available: bool) -> bool {
    // A queued intent is not permission authority. The owner validates current
    // server policy before dispatch; a known denial stays a UI boundary too.
    decision == Decision::Allowed || queue_available && decision != Decision::Denied
}

fn thread_reply_feature_allowed(
    content: ContentState,
    value: talk_core::ui::conversation::Availability,
    queue_available: bool,
    observed_root: bool,
) -> bool {
    use talk_core::ui::presentation::FeatureSupport;
    content == ContentState::Ready && value.enabled()
        || matches!(content, ContentState::Ready | ContentState::Offline)
            && queue_available
            && observed_root
            && value.server != FeatureSupport::Unsupported
            && value.allowed != Decision::Denied
}

fn feature_allows_intent(
    content: ContentState,
    value: talk_core::ui::conversation::Availability,
    feature: MessageFeature,
    queue_available: bool,
) -> bool {
    let queueable = matches!(
        feature,
        MessageFeature::React
            | MessageFeature::Reply
            | MessageFeature::Edit
            | MessageFeature::Delete
            | MessageFeature::Pins
            | MessageFeature::Reminders
            | MessageFeature::Schedules
            | MessageFeature::Forward
            | MessageFeature::MarkUnread
    );
    value.enabled() && content == ContentState::Ready
        || queue_available
            && queueable
            && matches!(content, ContentState::Ready | ContentState::Offline)
            && value.implemented
            && value.server != talk_core::ui::presentation::FeatureSupport::Unsupported
            && decision_allows_intent(value.allowed, true)
}

fn message_mutation_allowed(
    message: &MessageView,
    feature: MessageFeature,
    feature_enabled: bool,
    queue_available: bool,
) -> bool {
    let decision = match feature {
        MessageFeature::Edit if !message.metadata.truncated => message.metadata.edit,
        MessageFeature::Delete => message.metadata.delete,
        _ => return false,
    };
    feature_enabled
        && message_actions_allowed(message)
        && decision_allows_intent(decision, queue_available && message.own)
}
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn offline_thread_reply_stages_from_an_observed_root_without_stale_authority() {
        use talk_core::ui::conversation::Availability;
        use talk_core::ui::presentation::FeatureSupport;
        let unknown = Availability::UNKNOWN;
        assert!(thread_reply_feature_allowed(
            ContentState::Offline,
            unknown,
            true,
            true
        ));
        assert!(!thread_reply_feature_allowed(
            ContentState::Offline,
            unknown,
            false,
            true
        ));
        assert!(!thread_reply_feature_allowed(
            ContentState::Offline,
            unknown,
            true,
            false
        ));
        assert!(!thread_reply_feature_allowed(
            ContentState::Denied,
            unknown,
            true,
            true
        ));
        assert!(!thread_reply_feature_allowed(
            ContentState::Offline,
            Availability {
                server: FeatureSupport::Unsupported,
                ..unknown
            },
            true,
            true
        ));
        assert!(!thread_reply_feature_allowed(
            ContentState::Offline,
            Availability {
                allowed: Decision::Denied,
                ..unknown
            },
            true,
            true
        ));
        assert!(thread_reply_feature_allowed(
            ContentState::Ready,
            unknown,
            true,
            true
        ));
        assert!(thread_reply_feature_allowed(
            ContentState::Ready,
            Availability {
                implemented: true,
                server: FeatureSupport::Supported,
                allowed: Decision::Allowed,
            },
            false,
            true
        ));
    }

    #[test]
    fn automatic_read_marker_never_rewinds_when_scrolling_back_or_repainting() {
        use talk_core::ui::{AccountId, AccountScope, ConversationId, HostId, RuntimeGeneration};
        let room = ConversationRef {
            account: AccountScope {
                account: AccountId::from_local(1),
                runtime: RuntimeGeneration::from_local(1),
            },
            conversation: ConversationId {
                account: AccountId::from_local(1),
                host: HostId::from_local(1),
                local: 1,
            },
        };
        let other = ConversationRef {
            conversation: ConversationId {
                local: 2,
                ..room.conversation
            },
            ..room
        };
        assert!(read_advances(None, room, 100));
        for visible in [100, 99, 80, 2] {
            assert!(!read_advances(Some((room, 100)), room, visible));
        }
        assert!(read_advances(Some((room, 100)), room, 101));
        assert!(read_advances(Some((room, 100)), other, 2));
    }

    #[test]
    fn read_candidate_requires_painted_message_end_inside_the_history_viewport() {
        let bounds = |top, height| {
            gpui::Bounds::new(
                gpui::point(px(0.), px(top)),
                gpui::size(px(200.), px(height)),
            )
        };
        let viewport = bounds(100., 200.);
        // Paint callbacks need not be ordered. Offscreen/partially visible
        // message ends and header-covered rows must not advance the receipt.
        let painted = [
            (9, 90, bounds(280., 40.), true),
            (8, 80, bounds(240., 40.), true),
            (2, 20, bounds(80., 40.), true),
            (15, 150, bounds(500., 40.), true),
            (1, 10, bounds(60., 30.), true),
            (16, u128::MAX, bounds(275., 20.), false),
        ];
        assert_eq!(visible_read(&painted, viewport), Some(80));
        assert_eq!(visible_read(&painted, bounds(0., 0.)), None);
        assert_eq!(visible_read(&[], viewport), None);
        assert_eq!(visible_read(&painted[4..], viewport), None);
    }

    #[test]
    fn context_feedback_and_retry_follow_the_current_request_and_target() {
        use talk_core::ui::{UnavailableReason, conversation::RemoteState};
        let mut load = ContextLoad {
            target: 42,
            state: RemoteState::Loading(RequestId(7)),
        };
        assert!(
            context_status(&load).is_none(),
            "accepted loading is not a warning"
        );
        assert!(!current_context_retry(&load, RequestId(7), 42));
        load.state = RemoteState::Failed(RequestId(7), UnavailableReason::PolicyStale);
        assert_eq!(context_status(&load).unwrap().retry, Some(RequestId(7)));
        assert!(current_context_retry(&load, RequestId(7), 42));
        assert!(!current_context_retry(&load, RequestId(6), 42));
        assert!(!current_context_retry(&load, RequestId(7), 41));
        load.state = RemoteState::Loading(RequestId(8));
        assert!(!current_context_retry(&load, RequestId(7), 42));
        load.state = RemoteState::Idle;
        assert!(
            context_status(&load).is_none(),
            "successful retry removes its own error feedback"
        );
        load.state = RemoteState::Failed(RequestId(9), UnavailableReason::Unsupported);
        assert!(context_status(&load).is_some());
        assert!(context_status(&load).unwrap().retry.is_none());
    }
    #[test]
    fn thread_failure_and_rejected_admission_never_become_empty_success() {
        use talk_core::ui::{
            AccountId, AccountScope, ConversationId, HostId, RuntimeGeneration, UnavailableReason,
            conversation::RemoteState, messaging::MessagePanel,
        };
        let account = AccountId::from_local(1);
        let context = ConversationRef {
            account: AccountScope {
                account,
                runtime: RuntimeGeneration::from_local(1),
            },
            conversation: ConversationId {
                account,
                host: HostId::from_local(1),
                local: 1,
            },
        };
        let mut panel = MessagePanel::new(context);
        panel.thread_root = Some(42);
        let mut load = ThreadLoad {
            context,
            root: 42,
            request: None,
        };
        assert_eq!(load.content_state(Some(&panel)), ContentState::Failed);
        load.request = Some(RequestId(7));
        assert_eq!(load.content_state(None), ContentState::Loading);
        panel.thread_state = RemoteState::Loading(RequestId(7));
        assert_eq!(load.content_state(Some(&panel)), ContentState::Loading);
        for reason in [
            UnavailableReason::PolicyUnknown,
            UnavailableReason::PolicyStale,
        ] {
            panel.thread_state = RemoteState::Failed(RequestId(7), reason);
            assert_eq!(load.content_state(Some(&panel)), ContentState::Failed);
        }
        panel.thread_state = RemoteState::Failed(RequestId(7), UnavailableReason::PermissionDenied);
        assert_eq!(load.content_state(Some(&panel)), ContentState::Denied);
        load.request = Some(RequestId(8));
        assert_eq!(
            load.content_state(Some(&panel)),
            ContentState::Loading,
            "an old error cannot replace a retry"
        );
        panel.thread_state = RemoteState::Idle;
        assert_eq!(load.content_state(Some(&panel)), ContentState::Ready);
        panel.thread_root = Some(99);
        assert_eq!(load.content_state(Some(&panel)), ContentState::Loading);
        panel.thread_root = Some(42);
        panel.context.account.runtime = RuntimeGeneration::from_local(2);
        assert_eq!(load.content_state(Some(&panel)), ContentState::Loading);
    }
    #[test]
    fn reply_thread_reply_and_new_edit_cannot_replace_an_unfinished_edit() {
        for action in [Action::Reply(2), Action::Edit(4)] {
            assert!(may_replace_composer(&action, None, false));
            assert!(!may_replace_composer(&action, Some(1), false));
            assert!(!may_replace_composer(&action, Some(1), true));
            assert!(!may_replace_composer(&action, None, true));
        }
        // Resolving the edit stays available, including while its server result
        // is pending. Its correlated receipt owns whether the editor restores.
        assert!(may_replace_composer(&Action::CancelCompose, Some(1), true));
        assert!(may_replace_composer(&Action::ToggleSearch, Some(1), false));
    }
    #[test]
    fn system_file_share_uses_attachment_surface_while_activity_notice_stays_compact() {
        let mut message = MessageView {
            local_id: 7,
            author: Text::new("Fixture").unwrap(),
            initials: Text::new("F").unwrap(),
            time_label: Text::new("09:00").unwrap(),
            body: Text::new("preview.png").unwrap(),
            own: false,
            delivery: talk_core::ui::presentation::DeliveryState::Received,
            attachments: talk_core::ui::Page::empty(),
            reply_count: 0,
            metadata: talk_core::ui::messaging::MessageMetadata::default(),
        };
        message.metadata.system = true;
        assert!(compact_system_message(&message));
        message
            .metadata
            .rich
            .try_push(talk_core::ui::messaging::RichObject {
                placeholder: Text::new("file").unwrap(),
                kind: Text::new("file").unwrap(),
                id: Some(Text::new("42").unwrap()),
                name: Text::new("preview.png").unwrap(),
                link: None,
                mime_type: Some(Text::new("image/png").unwrap()),
                size: Some(100),
            })
            .unwrap();
        assert!(!compact_system_message(&message));
        message.metadata.deleted = true;
        assert!(compact_system_message(&message));
    }

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
        let allowed = |m: &MessageView| {
            reply_allowed(
                m,
                ContentState::Ready,
                Decision::Allowed,
                true,
                false,
                false,
            )
        };
        assert!(allowed(&message));
        message.metadata.deleted = true;
        assert!(!allowed(&message));
        message.metadata.deleted = false;
        message.metadata.system = true;
        assert!(!allowed(&message));
        message.metadata.system = false;
        for delivery in [
            talk_core::ui::presentation::DeliveryState::Queued,
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
            false,
            false
        ));
        for send in [Decision::Denied, Decision::Unknown, Decision::Stale] {
            assert!(!reply_allowed(
                &message,
                ContentState::Ready,
                send,
                true,
                false,
                false
            ));
        }
        assert!(!reply_allowed(
            &message,
            ContentState::Ready,
            Decision::Allowed,
            false,
            false,
            false
        ));
        assert!(!reply_allowed(
            &message,
            ContentState::Ready,
            Decision::Allowed,
            true,
            false,
            true
        ));
        for send in [Decision::Allowed, Decision::Unknown, Decision::Stale] {
            assert!(reply_allowed(
                &message,
                ContentState::Offline,
                send,
                true,
                true,
                false
            ));
        }
        assert!(!reply_allowed(
            &message,
            ContentState::Offline,
            Decision::Denied,
            true,
            true,
            false
        ));
        assert!(!reply_allowed(
            &message,
            ContentState::Denied,
            Decision::Unknown,
            true,
            true,
            false
        ));
        assert!(!reply_allowed(
            &message,
            ContentState::Offline,
            Decision::Unknown,
            false,
            true,
            false
        ));
        assert!(!reply_allowed(
            &message,
            ContentState::Offline,
            Decision::Unknown,
            true,
            true,
            true
        ));

        message.own = true;
        for feature in [MessageFeature::Edit, MessageFeature::Delete] {
            assert!(message_mutation_allowed(&message, feature, true, true));
            assert!(!message_mutation_allowed(&message, feature, true, false));
            assert!(!message_mutation_allowed(&message, feature, false, true));
            message.own = false;
            assert!(!message_mutation_allowed(&message, feature, true, true));
            message.own = true;
        }
        message.metadata.edit = Decision::Denied;
        message.metadata.delete = Decision::Denied;
        assert!(!message_mutation_allowed(
            &message,
            MessageFeature::Edit,
            true,
            true
        ));
        assert!(!message_mutation_allowed(
            &message,
            MessageFeature::Delete,
            true,
            true
        ));
        message.metadata.edit = Decision::Allowed;
        message.metadata.truncated = true;
        assert!(!message_mutation_allowed(
            &message,
            MessageFeature::Edit,
            true,
            true
        ));
    }

    #[test]
    fn offline_queueable_features_preserve_explicit_denials_and_live_only_reads() {
        use talk_core::ui::{conversation::Availability, presentation::FeatureSupport};
        let unknown = Availability::UNKNOWN;
        for feature in [
            MessageFeature::React,
            MessageFeature::Reply,
            MessageFeature::Edit,
            MessageFeature::Delete,
            MessageFeature::Pins,
            MessageFeature::Reminders,
            MessageFeature::Schedules,
            MessageFeature::Forward,
        ] {
            assert!(feature_allows_intent(
                ContentState::Offline,
                unknown,
                feature,
                true
            ));
            assert!(!feature_allows_intent(
                ContentState::Offline,
                unknown,
                feature,
                false
            ));
            assert!(!feature_allows_intent(
                ContentState::Denied,
                unknown,
                feature,
                true
            ));
            for denied in [
                Availability {
                    allowed: Decision::Denied,
                    ..unknown
                },
                Availability {
                    server: FeatureSupport::Unsupported,
                    ..unknown
                },
                Availability {
                    implemented: false,
                    ..unknown
                },
            ] {
                assert!(!feature_allows_intent(
                    ContentState::Offline,
                    denied,
                    feature,
                    true
                ));
            }
        }
        for feature in [
            MessageFeature::Search,
            MessageFeature::Threads,
            MessageFeature::Files,
            MessageFeature::UpcomingReminders,
        ] {
            assert!(!feature_allows_intent(
                ContentState::Offline,
                unknown,
                feature,
                true
            ));
        }
    }
}
