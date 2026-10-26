//! One anchored message menu; its callbacks carry a room, message and opening generation.
use super::*;
use gpui::{AnyElement, Bounds, Element, ElementId, KeyDownEvent, Pixels, Point, Size};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct Target {
    context: ConversationRef,
    message: u128,
    generation: u64,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum Choice {
    React,
    Reaction(u8),
    Edit,
    Delete,
    ConfirmDelete,
    Pin,
    Unpin,
    Remind,
    Forward,
    Thread,
    Back,
}

#[derive(Clone, Copy, Default)]
enum Mode {
    #[default]
    Actions,
    Reactions,
    Delete,
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub(crate) enum Source {
    #[default]
    Actions,
    Reactions,
    Badges,
}
impl Source {
    pub(super) fn key(self) -> &'static str {
        match self {
            Self::Actions => "message-menu",
            Self::Reactions => "message-reactions",
            Self::Badges => "message-reactions-badges",
        }
    }
}

#[derive(Default)]
struct Session {
    generation: u64,
    target: Option<Target>,
    mode: Mode,
}
impl Session {
    fn open(&mut self, context: ConversationRef, message: u128) -> Option<Target> {
        self.generation = self.generation.checked_add(1)?;
        let target = Target {
            context,
            message,
            generation: self.generation,
        };
        self.target = Some(target);
        self.mode = Mode::Actions;
        Some(target)
    }
    fn accepts(&self, target: Target, selected: Option<ConversationRef>) -> bool {
        self.target == Some(target) && selected == Some(target.context)
    }
    fn close(&mut self) {
        self.target = None;
        self.mode = Mode::Actions;
    }
}

#[derive(Default)]
struct Geometry {
    target: Option<Target>,
    anchor: Option<Bounds<Pixels>>,
    panel: Option<Bounds<Pixels>>,
}

pub(super) struct Menu {
    session: Session,
    source: Source,
    focus: FocusHandle,
    scroll: gpui::ScrollHandle,
    return_focus: Option<FocusHandle>,
    pending_focus: bool,
    geometry: Rc<RefCell<Geometry>>,
}
impl Menu {
    pub fn new(cx: &Context<Desktop>) -> Self {
        Self {
            session: Session::default(),
            source: Source::Actions,
            focus: cx.focus_handle(),
            scroll: gpui::ScrollHandle::new(),
            return_focus: None,
            pending_focus: false,
            geometry: Rc::new(RefCell::new(Geometry::default())),
        }
    }
    pub fn target(&self) -> Option<Target> {
        self.session.target
    }
    pub fn message(&self) -> Option<u128> {
        self.target().map(|target| target.message)
    }
    pub fn is_for(&self, context: ConversationRef, message: u128) -> bool {
        self.target()
            .is_some_and(|target| target.context == context && target.message == message)
    }
    pub fn anchored_to(&self, context: ConversationRef, message: u128, source: Source) -> bool {
        self.is_for(context, message) && self.source == source
    }
    pub fn clear(&mut self) {
        self.session.close();
        self.return_focus = None;
        self.pending_focus = false;
        self.scroll.set_offset(gpui::point(px(0.), px(0.)));
        *self.geometry.borrow_mut() = Geometry::default();
    }
}

struct Item {
    choice: Choice,
    label: String,
    enabled: bool,
    reason: &'static str,
}
impl Item {
    fn new(choice: Choice, label: impl Into<String>, enabled: bool, reason: &'static str) -> Self {
        Self {
            choice,
            label: label.into(),
            enabled,
            reason,
        }
    }
    fn id(&self, target: Target) -> ElementId {
        scoped_id(
            "message-popup-item",
            target.context,
            target.message,
            u128::from(target.generation) * 128 + self.choice.ordinal(),
        )
    }
}
impl Choice {
    fn icon(self) -> &'static str {
        match self {
            Self::React => "smile",
            Self::Reaction(_) => "smile", // The emoji grid does not render this icon.
            Self::Edit => "file",
            Self::Delete | Self::ConfirmDelete => "trash-2",
            Self::Pin | Self::Unpin => "pin",
            Self::Remind => "clock",
            Self::Forward => "arrow-right",
            Self::Thread => "chat",
            Self::Back => "arrow-left",
        }
    }
    fn ordinal(self) -> u128 {
        match self {
            Self::React => 0,
            Self::Reaction(index) => 32 + u128::from(index),
            Self::Edit => 1,
            Self::Delete => 2,
            Self::ConfirmDelete => 3,
            Self::Pin => 4,
            Self::Unpin => 10,
            Self::Remind => 5,
            Self::Forward => 6,
            Self::Thread => 8,
            Self::Back => 9,
        }
    }
}

impl Desktop {
    fn current_menu_message(&self, target: Target) -> Option<&MessageView> {
        if !self
            .message_ui
            .menu
            .session
            .accepts(target, self.snapshot.selected)
            || self.snapshot.content != ContentState::Ready && !self.local_message_intents()
        {
            return None;
        }
        self.snapshot
            .messages
            .as_slice()
            .iter()
            .find(|message| message.local_id == target.message && message_actions_allowed(message))
    }

    fn message_menu_items(&self, target: Target) -> Vec<Item> {
        let Some(message) = self.current_menu_message(target) else {
            return Vec::new();
        };
        let idle = self.snapshot.message_panel.as_ref().is_some_and(|panel| {
            panel.context == target.context
                && (!panel.state.pending() || self.local_message_intents())
        });
        let feature = |feature| idle && self.message_feature_enabled(feature);
        let edit = message_mutation_allowed(
            message,
            MessageFeature::Edit,
            feature(MessageFeature::Edit),
            self.local_message_intents(),
        ) && !self.drafts.pending(target.context)
            && self.message_ui.pending_edit.is_none()
            && self.message_ui.editing.is_none();
        let delete = message_mutation_allowed(
            message,
            MessageFeature::Delete,
            feature(MessageFeature::Delete),
            self.local_message_intents(),
        );
        let forward = self.forward_source_available(target.context, target.message);
        let pinned = self.snapshot.message_panel.as_ref().is_some_and(|panel| {
            panel.context == target.context
                && panel
                    .pinned_messages
                    .as_slice()
                    .iter()
                    .any(|m| m.local_id == target.message)
        });
        let pin_change = feature(MessageFeature::Pins)
            && self.snapshot.message_panel.as_ref().is_some_and(|panel| {
                decision_allows_intent(panel.pin_change, self.local_message_intents())
            });
        let back = || Item::new(Choice::Back, "Zurück", true, "");
        match self.message_ui.menu.session.mode {
            Mode::Actions => {
                let mut items = vec![
                    Item::new(
                        Choice::React,
                        "Reaktion hinzufügen",
                        self.reaction_available(target.message).is_enabled(),
                        "Reaktionen sind derzeit nicht verfügbar.",
                    ),
                    Item::new(
                        Choice::Edit,
                        "Bearbeiten",
                        edit,
                        "Diese Nachricht kann nicht bearbeitet werden.",
                    ),
                    Item::new(
                        Choice::Delete,
                        "Löschen",
                        delete,
                        "Diese Nachricht kann nicht gelöscht werden.",
                    ),
                    Item::new(
                        if pinned { Choice::Unpin } else { Choice::Pin },
                        if pinned {
                            "Angeheftete Nachricht lösen"
                        } else {
                            "Anheften"
                        },
                        pin_change,
                        "Anheften ist derzeit nicht verfügbar.",
                    ),
                    Item::new(
                        Choice::Remind,
                        "Später erinnern",
                        feature(MessageFeature::Reminders),
                        "Erinnerungen sind derzeit nicht verfügbar.",
                    ),
                    Item::new(
                        Choice::Forward,
                        "Weiterleiten",
                        forward,
                        "Weiterleiten ist derzeit nicht verfügbar.",
                    ),
                ];
                if message.metadata.replyable {
                    items.push(Item::new(
                        Choice::Thread,
                        "Thread öffnen",
                        feature(MessageFeature::Threads),
                        "Threads sind derzeit nicht verfügbar.",
                    ));
                }
                items
            }
            Mode::Reactions => reactions::EMOJI
                .iter()
                .enumerate()
                .map(|(index, emoji)| {
                    Item::new(
                        Choice::Reaction(index as u8),
                        *emoji,
                        self.reaction_available(target.message).is_enabled(),
                        "Reaktionen sind derzeit nicht verfügbar.",
                    )
                })
                .collect(),
            Mode::Delete => vec![
                Item::new(
                    Choice::ConfirmDelete,
                    "Löschen bestätigen",
                    delete,
                    "Diese Nachricht kann nicht gelöscht werden.",
                ),
                back(),
            ],
        }
    }

    pub(super) fn trace_message_menu(&self, window: &Window) {
        let geometry = self.message_ui.menu.geometry.borrow();
        let focused = self.message_ui.menu.target().and_then(|target| {
            self.message_menu_items(target).iter().position(|item| {
                self.control_focus
                    .current(&item.id(target))
                    .is_some_and(|focus| focus.is_focused(window))
            })
        });
        println!(
            "UI_MESSAGE_MENU open={} anchor={:?} panel={:?} focused_index={:?}",
            self.message_ui.menu.target().is_some(),
            geometry.anchor,
            geometry.panel,
            focused
        );
        println!(
            "UI_REACTIONS palette={} source={:?} local_pending={}",
            self.message_ui.menu.target().is_some()
                && matches!(self.message_ui.menu.session.mode, Mode::Reactions),
            self.message_ui.menu.source,
            self.message_ui.reaction_request.is_some(),
        );
    }

    pub(super) fn toggle_message_menu(
        &mut self,
        context: ConversationRef,
        message: u128,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.open_message_popup(context, message, Source::Actions, window, cx);
    }

    pub(super) fn toggle_reaction_palette(
        &mut self,
        context: ConversationRef,
        message: u128,
        source: Source,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if source == Source::Actions || !self.reaction_available(message).is_enabled() {
            return;
        }
        self.open_message_popup(context, message, source, window, cx);
    }

    fn open_message_popup(
        &mut self,
        context: ConversationRef,
        message: u128,
        source: Source,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.message_ui.menu.anchored_to(context, message, source) {
            self.dismiss_message_menu(true, window, cx);
            return;
        }
        if Some(context) != self.snapshot.selected
            || self.snapshot.content != ContentState::Ready && !self.local_message_intents()
            || !self
                .snapshot
                .messages
                .as_slice()
                .iter()
                .any(|row| row.local_id == message && message_actions_allowed(row))
        {
            return;
        }
        self.message_ui.menu.clear();
        let Some(target) = self.message_ui.menu.session.open(context, message) else {
            return;
        };
        self.message_ui.menu.source = source;
        self.message_ui.menu.session.mode = if source == Source::Actions {
            Mode::Actions
        } else {
            Mode::Reactions
        };
        self.message_ui.menu.geometry.borrow_mut().target = Some(target);
        self.message_ui.menu.return_focus = self
            .control_focus
            .current(&scoped_id(source.key(), context, message, 0))
            .or_else(|| window.focused(cx));
        self.message_ui.menu.pending_focus = true;
        self.hover_hints.cancel();
    }

    fn dismiss_message_menu(&mut self, restore: bool, window: &mut Window, cx: &mut Context<Self>) {
        let target = self.message_ui.menu.target();
        let focus = self.message_ui.menu.return_focus.take();
        self.message_ui.menu.clear();
        if restore && target.is_some_and(|target| Some(target.context) == self.snapshot.selected) {
            if let Some(focus) = focus.filter(|focus| self.root_focus.contains(focus, window)) {
                focus.focus(window);
            } else {
                self.navigation.history_focus().focus(window);
            }
        }
        cx.notify();
    }

    pub(super) fn choose_message_menu(
        &mut self,
        target: Target,
        choice: Choice,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        // Both the opening generation and the currently projected policy are
        // checked at activation; a stale row callback cannot act in a new room.
        if !self
            .message_menu_items(target)
            .iter()
            .any(|item| item.choice == choice && item.enabled)
        {
            return;
        }
        match choice {
            Choice::React => self.message_ui.menu.session.mode = Mode::Reactions,
            Choice::Reaction(index) => {
                if let Some(emoji) = reactions::EMOJI
                    .get(usize::from(index))
                    .and_then(|emoji| Text::new(emoji).ok())
                    && self.toggle_reaction(target.message, emoji)
                {
                    self.dismiss_message_menu(true, window, cx);
                }
                return;
            }
            Choice::Delete => self.message_ui.menu.session.mode = Mode::Delete,
            Choice::Forward => {
                let return_control = scoped_id(
                    self.message_ui.menu.source.key(),
                    target.context,
                    target.message,
                    0,
                );
                self.dismiss_message_menu(false, window, cx);
                self.open_forward_dialog(
                    target.context,
                    target.message,
                    return_control,
                    window,
                    cx,
                );
                return;
            }
            Choice::Back => self.message_ui.menu.session.mode = Mode::Actions,
            _ => {
                let thread = self
                    .current_menu_message(target)
                    .and_then(|row| row.metadata.thread)
                    .unwrap_or(target.message);
                self.dismiss_message_menu(
                    !matches!(choice, Choice::Edit | Choice::Remind),
                    window,
                    cx,
                );
                match choice {
                    Choice::Edit => self.message_action(Action::Edit(target.message), window, cx),
                    Choice::ConfirmDelete => {
                        self.submit_message(MessageAction::Delete(target.message));
                    }
                    Choice::Pin | Choice::Unpin => {
                        self.submit_message(if choice == Choice::Unpin {
                            MessageAction::Unpin(target.message)
                        } else {
                            MessageAction::Pin(target.message)
                        });
                    }
                    Choice::Remind => {
                        self.message_action(Action::Remind(target.message), window, cx);
                        self.message_ui.time_input.focus_handle(cx).focus(window);
                    }
                    Choice::Thread => {
                        self.handle(LocalAction::Thread(target.context, thread), window, cx)
                    }
                    _ => {}
                }
                return;
            }
        }
        self.message_ui
            .menu
            .scroll
            .set_offset(gpui::point(px(0.), px(0.)));
        self.message_ui.menu.pending_focus = true;
    }

    /// Exactly one frame-local rectangle, recorded by the actual three-dot control.
    pub(super) fn message_menu_anchor(
        &self,
        context: ConversationRef,
        message: u128,
        source: Source,
    ) -> impl IntoElement {
        let geometry = self.message_ui.menu.geometry.clone();
        let target = self.message_ui.menu.target().filter(|target| {
            target.context == context
                && target.message == message
                && self.message_ui.menu.source == source
        });
        gpui::canvas(
            move |bounds, _, _| {
                if let Some(target) = target {
                    // GPUI holds ListState mutably while prepainting each row.
                    // Record only this frame's control bounds here; the root
                    // popup checks the viewport after list prepaint completes.
                    let mut geometry = geometry.borrow_mut();
                    if geometry.target == Some(target) {
                        geometry.anchor = Some(bounds);
                    }
                }
            },
            |_, _, _, _| {},
        )
        .absolute()
        .inset_0()
    }

    pub(crate) fn prepare_message_menu(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.revalidate_forward_session();
        self.prepare_forward_dialog(window, cx);
        if let Some(target) = self.message_ui.menu.target() {
            let moved_focus = !self.message_ui.menu.pending_focus
                && !self.message_ui.menu.focus.contains_focused(window, cx);
            if self.current_menu_message(target).is_none()
                || !window.is_window_active()
                || self.update_restart_pending
                || self.projection_blocked
                || self.prejoin.is_some()
                || self.incoming_visible()
                || self.onboarding_open
                || self.account_removal.confirming()
                || self.navigation_open
                || !matches!(
                    self.overlay,
                    None | Some(Overlay::CallChat | Overlay::Thread)
                )
                || moved_focus
            {
                self.dismiss_message_menu(!moved_focus && window.is_window_active(), window, cx);
            }
        }
        let mut geometry = self.message_ui.menu.geometry.borrow_mut();
        geometry.anchor = None;
        geometry.panel = None;
    }

    pub(crate) fn message_menu_popup(
        &self,
        window: &Window,
        cx: &Context<Self>,
    ) -> Option<AnyElement> {
        let target = self.message_ui.menu.target()?;
        let message = self.current_menu_message(target)?;
        let geometry = self.message_ui.menu.geometry.clone();
        let outside_geometry = geometry.clone();
        let width = self.message_popup_width(window);
        let mut body = self
            .context_menu_surface()
            .id(scoped_id(
                "message-popup",
                target.context,
                target.message,
                u128::from(target.generation),
            ))
            .track_focus(&self.message_ui.menu.focus)
            .key_context("MessageMenu")
            .occlude()
            .w(width)
            .max_h((window.viewport_size().height - px(24.)).max(px(1.)))
            .overflow_y_scroll()
            .track_scroll(&self.message_ui.menu.scroll)
            .on_mouse_down_out(cx.listener(
                move |this, event: &gpui::MouseDownEvent, window, cx| {
                    if this.message_ui.menu.target() == Some(target)
                        && !outside_geometry
                            .borrow()
                            .anchor
                            .is_some_and(|bounds| bounds.contains(&event.position))
                    {
                        this.dismiss_message_menu(true, window, cx);
                    }
                },
            ));
        match self.message_ui.menu.session.mode {
            Mode::Delete => {
                body = body.child(div().p_3().child(self.hint(
                    self.l("Diese Nachricht für alle löschen? Geteilte Dateien bleiben erhalten."),
                )));
            }
            Mode::Actions => {}
            Mode::Reactions => {
                body = body.child(
                    div()
                        .p_2()
                        .font_weight(FontWeight::MEDIUM)
                        .child(self.l("Reaktion hinzufügen")),
                );
            }
        }
        let items = self.message_menu_items(target);
        if matches!(self.message_ui.menu.session.mode, Mode::Reactions) {
            for row in items.chunks(self.reaction_columns(window)) {
                body = body.child(div().px_1().py(px(2.)).flex().gap(px(2.)).children(
                    row.iter().map(|item| {
                        let own = message
                            .metadata
                            .reactions
                            .as_slice()
                            .iter()
                            .any(|reaction| reaction.emoji.as_str() == item.label && reaction.own);
                        let available = Availability::when(item.enabled, item.reason);
                        self.control(
                            item.id(target),
                            ButtonStyle::Quiet,
                            available,
                            LocalAction::MessageIn(
                                target.context,
                                Action::MenuChoice(target, item.choice),
                            ),
                            cx,
                        )
                        .size(self.sp(36.).max(px(34.)))
                        .min_h(self.sp(36.).max(px(34.)))
                        .p_0()
                        .text_size(self.sp(16.))
                        .bg(if own {
                            self.theme.selected
                        } else {
                            gpui::transparent_black()
                        })
                        .child(item.label.clone())
                        .child(self.reason_tip(
                            item.id(target),
                            available,
                            Some(if own {
                                "Eigene Reaktion entfernen"
                            } else {
                                "Reaktion hinzufügen"
                            }),
                            cx,
                        ))
                    }),
                ));
            }
        } else {
            for item in items {
                let label = self.l(&item.label).to_owned();
                let color = if !item.enabled {
                    self.theme.muted
                } else if matches!(item.choice, Choice::Delete | Choice::ConfirmDelete) {
                    self.theme.danger
                } else {
                    self.theme.muted
                };
                body = body.child(
                    self.control(
                        item.id(target),
                        if item.choice == Choice::ConfirmDelete {
                            ButtonStyle::Danger
                        } else {
                            ButtonStyle::Quiet
                        },
                        Availability::when(item.enabled, item.reason).hinted(),
                        LocalAction::MessageIn(
                            target.context,
                            Action::MenuChoice(target, item.choice),
                        ),
                        cx,
                    )
                    .w_full()
                    .justify_start()
                    .child(
                        div()
                            .relative()
                            .size(px(18.))
                            .flex_none()
                            .child(self.scaled_icon(item.choice.icon(), 18., color))
                            .when(item.choice == Choice::Unpin, |icon| {
                                icon.child(
                                    div()
                                        .absolute()
                                        .right(px(-3.))
                                        .top(px(-3.))
                                        .rounded_full()
                                        .bg(self.theme.surface)
                                        .child(self.scaled_icon("x", 10., color)),
                                )
                            }),
                    )
                    .child(label)
                    .when(!item.enabled, |control| {
                        control.child(self.reason_tip(
                            item.id(target),
                            Availability::when(false, item.reason),
                            None,
                            cx,
                        ))
                    }),
                );
            }
        }
        Some(
            Popup {
                target,
                geometry,
                list: self.message_ui.list.clone(),
                header_inset: self.message_ui.geometry.borrow().header_inset,
                child: body.into_any_element(),
                width,
            }
            .into_any_element(),
        )
    }

    fn message_menu_prefix(&self) -> usize {
        match self.message_ui.menu.session.mode {
            Mode::Actions => 0,
            Mode::Reactions => 1,
            Mode::Delete => 1,
        }
    }

    fn message_popup_width(&self, window: &Window) -> Pixels {
        self.sp(256.)
            .min((window.viewport_size().width - px(24.)).max(px(1.)))
    }
    fn reaction_columns(&self, window: &Window) -> usize {
        (f32::from(self.message_popup_width(window) - px(20.))
            / f32::from(self.sp(36.).max(px(34.)) + px(2.)))
        .floor()
        .clamp(1., 6.) as usize
    }
    fn message_menu_scroll_index(&self, index: usize, window: &Window) -> usize {
        self.message_menu_prefix()
            + if matches!(self.message_ui.menu.session.mode, Mode::Reactions) {
                index / self.reaction_columns(window)
            } else {
                index
            }
    }

    pub(crate) fn finish_message_menu_frame(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.finish_forward_dialog_frame(window, cx);
        let Some(target) = self.message_ui.menu.target() else {
            return;
        };
        if self.message_ui.menu.geometry.borrow().panel.is_none() {
            self.dismiss_message_menu(true, window, cx);
            return;
        }
        if self.message_ui.menu.pending_focus {
            self.message_ui.menu.pending_focus = false;
            if let Some((index, focus)) = self
                .message_menu_items(target)
                .iter()
                .enumerate()
                .filter(|(_, item)| item.enabled)
                .find_map(|(index, item)| {
                    self.control_focus
                        .current(&item.id(target))
                        .map(|focus| (index, focus))
                })
            {
                focus.focus(window);
                self.message_ui
                    .menu
                    .scroll
                    .scroll_to_item(self.message_menu_scroll_index(index, window));
            } else {
                self.message_ui.menu.focus.focus(window);
            }
            cx.notify();
        }
    }

    pub(crate) fn message_menu_key(
        &mut self,
        event: &KeyDownEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> bool {
        if self.forward_dialog_key(event, window, cx) {
            return true;
        }
        let Some(target) = self.message_ui.menu.target() else {
            return false;
        };
        let key = event.keystroke.key.as_str();
        if key == "escape" {
            self.dismiss_message_menu(true, window, cx);
            return true;
        }
        if key == "tab" {
            // The current native tab tree still contains this popup until the
            // next frame. Return to its stable invoker before normal tabbing.
            self.dismiss_message_menu(true, window, cx);
            self.keyboard_navigation = true;
            return true;
        }
        let reactions = matches!(self.message_ui.menu.session.mode, Mode::Reactions);
        if event.keystroke.modifiers.modified()
            || !(matches!(key, "up" | "down" | "home" | "end")
                || (reactions && matches!(key, "left" | "right")))
        {
            return false;
        }
        let focus: Vec<_> = self
            .message_menu_items(target)
            .iter()
            .enumerate()
            .filter(|(_, item)| item.enabled)
            .filter_map(|(index, item)| {
                self.control_focus
                    .current(&item.id(target))
                    .map(|handle| (index, handle))
            })
            .collect();
        let current = focus
            .iter()
            .position(|(_, handle)| handle.is_focused(window));
        let next = if reactions {
            next_reaction_index(current, focus.len(), self.reaction_columns(window), key)
        } else {
            next_index(current, focus.len(), key)
        };
        if let Some(index) = next
            && let Some((row, handle)) = focus.get(index)
        {
            handle.focus(window);
            self.message_ui
                .menu
                .scroll
                .scroll_to_item(self.message_menu_scroll_index(*row, window));
            self.keyboard_navigation = true;
        }
        true
    }
}

fn next_reaction_index(
    current: Option<usize>,
    count: usize,
    columns: usize,
    key: &str,
) -> Option<usize> {
    if count == 0 {
        return None;
    }
    if matches!(key, "home" | "end") {
        return next_index(current, count, key);
    }
    let step = if matches!(key, "up" | "down") {
        columns.min(count)
    } else {
        1
    };
    Some(match current {
        None => 0,
        Some(index) if matches!(key, "up" | "left") => (index + count - step) % count,
        Some(index) => (index + step) % count,
    })
}

fn next_index(current: Option<usize>, count: usize, key: &str) -> Option<usize> {
    if count == 0 {
        return None;
    }
    Some(match key {
        "home" => 0,
        "end" => count - 1,
        "up" => current.map_or(count - 1, |index| (index + count - 1) % count),
        _ => current.map_or(0, |index| (index + 1) % count),
    })
}

/// Root-layer layout reads the anchor after the virtual list has painted its
/// geometry. No stale screen coordinate or clipping from a message row is retained.
struct Popup {
    target: Target,
    geometry: Rc<RefCell<Geometry>>,
    list: gpui::ListState,
    header_inset: Pixels,
    child: AnyElement,
    width: Pixels,
}
impl IntoElement for Popup {
    type Element = Self;
    fn into_element(self) -> Self {
        self
    }
}
impl Element for Popup {
    type RequestLayoutState = ();
    type PrepaintState = bool;
    fn id(&self) -> Option<ElementId> {
        None
    }
    fn source_location(&self) -> Option<&'static core::panic::Location<'static>> {
        None
    }
    fn request_layout(
        &mut self,
        _: Option<&gpui::GlobalElementId>,
        _: Option<&gpui::InspectorElementId>,
        window: &mut Window,
        cx: &mut gpui::App,
    ) -> (gpui::LayoutId, ()) {
        (
            window.request_layout(
                gpui::Style {
                    position: gpui::Position::Absolute,
                    inset: gpui::Edges::all(px(0.).into()),
                    ..Default::default()
                },
                [],
                cx,
            ),
            (),
        )
    }
    fn prepaint(
        &mut self,
        _: Option<&gpui::GlobalElementId>,
        _: Option<&gpui::InspectorElementId>,
        _: Bounds<Pixels>,
        _: &mut (),
        window: &mut Window,
        cx: &mut gpui::App,
    ) -> bool {
        let anchor = {
            let geometry = self.geometry.borrow();
            (geometry.target == Some(self.target))
                .then_some(geometry.anchor)
                .flatten()
        };
        let Some(anchor) = anchor else {
            return false;
        };
        let viewport = self.list.viewport_bounds();
        if anchor.bottom() <= viewport.top() + self.header_inset
            || anchor.top() >= viewport.bottom()
        {
            self.geometry.borrow_mut().anchor = None;
            return false;
        }
        let size = self.child.layout_as_root(
            gpui::size(
                gpui::AvailableSpace::Definite(self.width),
                gpui::AvailableSpace::MinContent,
            ),
            window,
            cx,
        );
        let origin = popup_origin(anchor, size, window.viewport_size());
        self.child.prepaint_at(origin, window, cx);
        self.geometry.borrow_mut().panel = Some(Bounds::new(origin, size));
        true
    }
    fn paint(
        &mut self,
        _: Option<&gpui::GlobalElementId>,
        _: Option<&gpui::InspectorElementId>,
        _: Bounds<Pixels>,
        _: &mut (),
        painted: &mut bool,
        window: &mut Window,
        cx: &mut gpui::App,
    ) {
        if *painted {
            self.child.paint(window, cx);
        }
    }
}

fn popup_origin(
    anchor: Bounds<Pixels>,
    size: Size<Pixels>,
    viewport: Size<Pixels>,
) -> Point<Pixels> {
    let margin = px(12.);
    let left = (anchor.right() - size.width)
        .max(margin)
        .min((viewport.width - size.width - margin).max(px(0.)));
    let below = anchor.bottom() + px(4.);
    let top = if below + size.height <= viewport.height - margin {
        below
    } else {
        anchor.top() - size.height - px(4.)
    }
    .max(margin)
    .min((viewport.height - size.height - margin).max(px(0.)));
    gpui::point(left, top)
}

#[cfg(test)]
mod tests {
    use super::*;
    use talk_core::ui::{AccountId, AccountScope, ConversationId, HostId, RuntimeGeneration};
    fn room(local: u128) -> ConversationRef {
        let account = AccountId::from_local(1);
        ConversationRef {
            account: AccountScope {
                account,
                runtime: RuntimeGeneration::from_local(1),
            },
            conversation: ConversationId {
                account,
                host: HostId::from_local(1),
                local,
            },
        }
    }
    #[test]
    fn old_popup_callbacks_cannot_reopen_or_act_in_another_room() {
        let mut session = Session::default();
        let first = session.open(room(1), 7).unwrap();
        assert!(session.accepts(first, Some(room(1))));
        assert!(!session.accepts(first, Some(room(2))));
        session.close();
        assert!(!session.accepts(first, Some(room(1))));
        let reopened = session.open(room(1), 7).unwrap();
        assert_ne!(first, reopened);
        assert!(!session.accepts(first, Some(room(1))));
        session.open(room(2), 7).unwrap();
        assert!(!session.accepts(reopened, Some(room(2))));
    }
    #[test]
    fn popup_fits_above_low_anchor_and_keyboard_wraps() {
        let viewport = gpui::size(px(800.), px(600.));
        let size = gpui::size(px(256.), px(280.));
        let anchor = Bounds::new(
            gpui::point(px(740.), px(550.)),
            gpui::size(px(36.), px(36.)),
        );
        let origin = popup_origin(anchor, size, viewport);
        assert!(origin.x >= px(0.) && origin.x + size.width <= viewport.width);
        assert!(origin.y + size.height < anchor.top());
        assert_eq!(next_index(Some(0), 3, "up"), Some(2));
        assert_eq!(next_index(Some(2), 3, "down"), Some(0));
        assert_eq!(next_index(None, 0, "down"), None);
    }
    #[test]
    fn reaction_grid_navigation_tracks_columns_and_opening_identity() {
        assert_eq!(next_reaction_index(Some(1), 36, 6, "down"), Some(7));
        assert_eq!(next_reaction_index(Some(7), 36, 6, "up"), Some(1));
        assert_eq!(next_reaction_index(Some(0), 36, 6, "left"), Some(35));
        assert_eq!(next_reaction_index(Some(35), 36, 6, "right"), Some(0));
        assert_eq!(next_reaction_index(Some(0), 36, 4, "down"), Some(4));
        assert_eq!(next_reaction_index(None, 0, 6, "down"), None);
        let mut session = Session::default();
        let first = session.open(room(1), 7).unwrap();
        let next = session.open(room(1), 7).unwrap();
        assert!(!session.accepts(first, Some(room(1))));
        let item = |target, choice| Item::new(choice, "", true, "").id(target);
        assert_ne!(
            item(first, Choice::Reaction(35)),
            item(next, Choice::Reaction(19))
        );
    }
}
