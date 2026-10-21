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
    Edit,
    Delete,
    ConfirmDelete,
    Pin,
    Remind,
    Forward,
    Destination(ConversationRef),
    ConfirmForward,
    Thread,
    Back,
}

#[derive(Clone, Copy, Default)]
enum Mode {
    #[default]
    Actions,
    Delete,
    Forward(Option<ConversationRef>),
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
        match self.choice {
            Choice::Destination(context) => scoped_id(
                "menu-forward-to",
                context,
                target.message,
                u128::from(target.generation),
            ),
            _ => scoped_id(
                "message-popup-item",
                target.context,
                target.message,
                u128::from(target.generation) * 16 + self.choice.ordinal(),
            ),
        }
    }
}
impl Choice {
    fn ordinal(self) -> u128 {
        match self {
            Self::React => 0,
            Self::Edit => 1,
            Self::Delete => 2,
            Self::ConfirmDelete => 3,
            Self::Pin => 4,
            Self::Remind => 5,
            Self::Forward => 6,
            Self::ConfirmForward => 7,
            Self::Thread => 8,
            Self::Back => 9,
            Self::Destination(_) => 10,
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
            || self.snapshot.content != ContentState::Ready
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
        let idle = self
            .snapshot
            .message_panel
            .as_ref()
            .is_some_and(|panel| panel.context == target.context && !panel.state.pending());
        let feature = |feature| idle && self.message_feature_enabled(feature);
        let edit = idle
            && message.metadata.edit == Decision::Allowed
            && !self.drafts.pending(target.context)
            && self.message_ui.pending_edit.is_none()
            && self.message_ui.editing.is_none();
        let delete = idle && message.metadata.delete == Decision::Allowed;
        let forward = feature(MessageFeature::Forward);
        let back = || Item::new(Choice::Back, "Zurück", true, "");
        match self.message_ui.menu.session.mode {
            Mode::Actions => {
                let mut items = vec![
                    Item::new(
                        Choice::React,
                        "Mit 👍 reagieren",
                        feature(MessageFeature::React),
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
                        Choice::Pin,
                        "Anheften",
                        feature(MessageFeature::Pins),
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
            Mode::Delete => vec![
                Item::new(
                    Choice::ConfirmDelete,
                    "Löschen bestätigen",
                    delete,
                    "Diese Nachricht kann nicht gelöscht werden.",
                ),
                back(),
            ],
            Mode::Forward(destination) => {
                let mut items = vec![back()];
                items.extend(
                    self.snapshot
                        .conversations
                        .as_slice()
                        .iter()
                        .filter(|room| {
                            room.context != target.context
                                && room.context.account == target.context.account
                                && room.send == Decision::Allowed
                        })
                        .take(20)
                        .map(|room| {
                            Item::new(
                                Choice::Destination(room.context),
                                room.title.as_str(),
                                forward,
                                "Weiterleiten ist derzeit nicht verfügbar.",
                            )
                        }),
                );
                if let Some(destination) = destination {
                    let current = self.snapshot.conversations.as_slice().iter().any(|room| {
                        room.context == destination
                            && room.context != target.context
                            && room.context.account == target.context.account
                            && room.send == Decision::Allowed
                    });
                    items.push(Item::new(
                        Choice::ConfirmForward,
                        "An dieses Gespräch senden",
                        forward && current,
                        "Das Zielgespräch ist nicht mehr verfügbar.",
                    ));
                }
                items
            }
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
    }

    pub(super) fn toggle_message_menu(
        &mut self,
        context: ConversationRef,
        message: u128,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.message_ui.menu.is_for(context, message) {
            self.dismiss_message_menu(true, window, cx);
            return;
        }
        if Some(context) != self.snapshot.selected
            || self.snapshot.content != ContentState::Ready
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
        self.message_ui.menu.geometry.borrow_mut().target = Some(target);
        self.message_ui.menu.return_focus = self
            .control_focus
            .current(&scoped_id("message-menu", context, message, 0))
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
            Choice::Delete => self.message_ui.menu.session.mode = Mode::Delete,
            Choice::Forward => self.message_ui.menu.session.mode = Mode::Forward(None),
            Choice::Destination(destination) => {
                self.message_ui.menu.session.mode = Mode::Forward(Some(destination))
            }
            Choice::Back => self.message_ui.menu.session.mode = Mode::Actions,
            _ => {
                let destination = match self.message_ui.menu.session.mode {
                    Mode::Forward(destination) => destination,
                    _ => None,
                };
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
                    Choice::React => {
                        self.submit_message(MessageAction::React {
                            message: target.message,
                            emoji: Text::new("👍").expect("emoji"),
                            add: true,
                        });
                    }
                    Choice::Edit => self.message_action(Action::Edit(target.message), window, cx),
                    Choice::ConfirmDelete => {
                        self.submit_message(MessageAction::Delete(target.message));
                    }
                    Choice::Pin => {
                        self.submit_message(MessageAction::Pin(target.message));
                    }
                    Choice::Remind => {
                        self.message_action(Action::Remind(target.message), window, cx);
                        self.message_ui.time_input.focus_handle(cx).focus(window);
                    }
                    Choice::ConfirmForward => {
                        if let Some(destination) = destination {
                            self.submit_message(MessageAction::Forward {
                                message: target.message,
                                destination,
                            });
                        }
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
    ) -> impl IntoElement {
        let geometry = self.message_ui.menu.geometry.clone();
        let target = self
            .message_ui
            .menu
            .target()
            .filter(|target| target.context == context && target.message == message);
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
                || !matches!(self.overlay, None | Some(Overlay::CallChat))
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
        let width = self
            .sp(256.)
            .min((window.viewport_size().width - px(24.)).max(px(1.)));
        let mut body = div()
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
            .rounded(px(10.))
            .border_1()
            .border_color(self.theme.border)
            .shadow_lg()
            .bg(self.theme.surface)
            .p_1()
            .flex()
            .flex_col()
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
            Mode::Forward(destination) => {
                body = body.child(
                    div()
                        .p_3()
                        .child(
                            self.hint(self.l("Weiterleiten als neue Nachricht. Ziel auswählen:")),
                        )
                        .child(self.hint(message.body.as_str())),
                );
                if let Some(room) = destination.and_then(|destination| {
                    self.snapshot
                        .conversations
                        .as_slice()
                        .iter()
                        .find(|room| room.context == destination)
                }) {
                    body = body.child(
                        div()
                            .px_3()
                            .py_2()
                            .bg(self.theme.selected)
                            .child(room.title.as_str().to_owned()),
                    );
                }
            }
            Mode::Actions => {}
        }
        for item in self.message_menu_items(target) {
            let label = if matches!(item.choice, Choice::Destination(_)) {
                item.label.clone()
            } else {
                self.l(&item.label).to_owned()
            };
            // Room titles are user content; do not pass them through button's UI catalog.
            body = body.child(
                self.control(
                    item.id(target),
                    if item.choice == Choice::ConfirmDelete {
                        ButtonStyle::Danger
                    } else {
                        ButtonStyle::Quiet
                    },
                    Availability::when(item.enabled, item.reason).hinted(),
                    LocalAction::MessageIn(target.context, Action::MenuChoice(target, item.choice)),
                    cx,
                )
                .w_full()
                .justify_start()
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
            Mode::Delete | Mode::Forward(None) => 1,
            Mode::Forward(Some(destination)) => {
                1 + usize::from(
                    self.snapshot
                        .conversations
                        .as_slice()
                        .iter()
                        .any(|room| room.context == destination),
                )
            }
        }
    }

    pub(crate) fn finish_message_menu_frame(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
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
                    .scroll_to_item(self.message_menu_prefix() + index);
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
        if event.keystroke.modifiers.modified() || !matches!(key, "up" | "down" | "home" | "end") {
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
        if let Some(index) = next_index(current, focus.len(), key)
            && let Some((row, handle)) = focus.get(index)
        {
            handle.focus(window);
            self.message_ui
                .menu
                .scroll
                .scroll_to_item(self.message_menu_prefix() + *row);
            self.keyboard_navigation = true;
        }
        true
    }
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
}
