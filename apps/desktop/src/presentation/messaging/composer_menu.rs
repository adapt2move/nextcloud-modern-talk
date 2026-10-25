//! One root-layer composer menu, anchored to the current room's plus button.
use super::*;
use gpui::{AnyElement, Bounds, Element, ElementId, KeyDownEvent, Pixels, Point, Size};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct Target {
    context: ConversationRef,
    generation: u64,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum Choice {
    File,
    Poll,
    Schedule,
    Scheduled,
    Silent,
    Reminders,
}
impl Choice {
    fn id(self, target: Target) -> ElementId {
        scoped_id(
            "composer-popup-item",
            target.context,
            u128::from(target.generation),
            self as u128,
        )
    }
}

#[derive(Default)]
struct Session {
    generation: u64,
    target: Option<Target>,
}
impl Session {
    fn open(&mut self, context: ConversationRef) -> Option<Target> {
        self.generation = self.generation.checked_add(1)?;
        let target = Target {
            context,
            generation: self.generation,
        };
        self.target = Some(target);
        Some(target)
    }
    fn accepts(&self, target: Target, selected: Option<ConversationRef>) -> bool {
        self.target == Some(target) && selected == Some(target.context)
    }
}

#[derive(Default)]
struct Geometry {
    target: Option<Target>,
    anchor: Option<Bounds<Pixels>>,
    panel: Option<Bounds<Pixels>>,
}

pub(in crate::presentation) struct Menu {
    session: Session,
    focus: FocusHandle,
    return_focus: Option<FocusHandle>,
    pending_focus: bool,
    scroll: gpui::ScrollHandle,
    geometry: Rc<RefCell<Geometry>>,
}
impl Menu {
    pub(in crate::presentation) fn new(cx: &Context<Desktop>) -> Self {
        Self {
            session: Session::default(),
            focus: cx.focus_handle(),
            return_focus: None,
            pending_focus: false,
            scroll: gpui::ScrollHandle::new(),
            geometry: Rc::new(RefCell::new(Geometry::default())),
        }
    }
    fn clear(&mut self) {
        self.session.target = None;
        self.pending_focus = false;
        self.return_focus = None;
        self.scroll.set_offset(gpui::point(px(0.), px(0.)));
        *self.geometry.borrow_mut() = Geometry::default();
    }
}

struct Item {
    choice: Choice,
    label: &'static str,
    icon: &'static str,
    availability: Availability,
}

impl Desktop {
    fn composer_menu_context_current(&self) -> bool {
        self.snapshot.selected.is_some_and(|context| {
            self.snapshot.active_account == Some(context.account)
                && self
                    .snapshot
                    .conversations
                    .as_slice()
                    .iter()
                    .any(|row| row.context == context)
        }) && matches!(
            self.snapshot.content,
            ContentState::Ready | ContentState::Offline
        )
    }

    fn composer_menu_items(&self, target: Target) -> Vec<Item> {
        if !self.content_ui.menu
            || !self
                .content_ui
                .composer_menu
                .session
                .accepts(target, self.snapshot.selected)
            || !self.composer_menu_context_current()
        {
            return Vec::new();
        }
        let message = |choice, label, icon, feature| Item {
            choice,
            label,
            icon,
            availability: self.message_available(feature),
        };
        vec![
            message(
                Choice::File,
                "Datei anhängen",
                "paperclip",
                MessageFeature::Files,
            ),
            Item {
                choice: Choice::Poll,
                label: "Umfrage erstellen",
                icon: "plus",
                availability: Availability::when(
                    self.content_scope_current()
                        && self.snapshot.content_panel.poll_create.enabled(),
                    crate::presentation::conversation_dialog::unavailable(
                        self.snapshot.content_panel.poll_create,
                    ),
                ),
            },
            message(
                Choice::Schedule,
                "Nachricht planen",
                "clock",
                MessageFeature::Schedules,
            ),
            message(
                Choice::Scheduled,
                "Geplante Nachrichten",
                "inbox",
                MessageFeature::Schedules,
            ),
            message(
                Choice::Silent,
                "Leise senden",
                "moon",
                MessageFeature::Silent,
            ),
            message(
                Choice::Reminders,
                "Erinnerungen",
                "bell",
                MessageFeature::Reminders,
            ),
        ]
    }

    pub(in crate::presentation) fn toggle_composer_menu(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.content_ui.composer_menu.session.target.is_some() {
            self.dismiss_composer_menu(true, window, cx);
            return;
        }
        if !self.composer_menu_context_current() {
            return;
        }
        let Some(context) = self.snapshot.selected else {
            return;
        };
        let Some(target) = self.content_ui.composer_menu.session.open(context) else {
            return;
        };
        self.message_ui.menu.clear();
        self.content_ui.menu = true;
        self.content_ui.composer_menu.geometry.borrow_mut().target = Some(target);
        self.content_ui.composer_menu.return_focus = self
            .control_focus
            .current(&"attach".into())
            .or_else(|| window.focused(cx));
        self.content_ui.composer_menu.pending_focus = true;
        self.hover_hints.cancel();
        cx.notify();
    }

    fn dismiss_composer_menu(
        &mut self,
        restore: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let target = self.content_ui.composer_menu.session.target;
        let focus = self.content_ui.composer_menu.return_focus.take();
        self.content_ui.menu = false;
        self.content_ui.composer_menu.clear();
        if restore && target.is_some_and(|target| Some(target.context) == self.snapshot.selected) {
            if let Some(focus) = focus.filter(|focus| self.root_focus.contains(focus, window)) {
                focus.focus(window);
            } else {
                self.composer.focus_handle(cx).focus(window);
            }
        }
        cx.notify();
    }

    pub(in crate::presentation) fn choose_composer_menu(
        &mut self,
        target: Target,
        choice: Choice,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        // A retired popup cannot act on a replacement room or bypass new policy.
        if !self
            .composer_menu_items(target)
            .iter()
            .any(|item| item.choice == choice && item.availability.is_enabled())
        {
            return;
        }
        self.dismiss_composer_menu(false, window, cx);
        // A picker/dialog must inherit the editor, never this retired popup.
        self.composer.focus_handle(cx).focus(window);
        let action = match choice {
            Choice::File => LocalAction::Message(Action::Command(MessageAction::PickUpload)),
            Choice::Poll => LocalAction::Content(crate::presentation::content::Action::CreatePoll),
            Choice::Schedule => LocalAction::Message(Action::Schedule),
            Choice::Scheduled => LocalAction::Message(Action::Command(MessageAction::Scheduled)),
            Choice::Silent => LocalAction::Message(Action::Silent),
            Choice::Reminders => LocalAction::Message(Action::Command(MessageAction::Reminders)),
        };
        self.handle(action, window, cx);
    }

    pub(in crate::presentation) fn composer_menu_anchor(
        &self,
        context: ConversationRef,
    ) -> impl IntoElement {
        let geometry = self.content_ui.composer_menu.geometry.clone();
        let target = self
            .content_ui
            .composer_menu
            .session
            .target
            .filter(|target| target.context == context);
        gpui::canvas(
            move |bounds, _, _| {
                if let Some(target) = target {
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

    pub(crate) fn prepare_composer_menu(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(target) = self.content_ui.composer_menu.session.target {
            let moved_focus = !self.content_ui.composer_menu.pending_focus
                && !self
                    .content_ui
                    .composer_menu
                    .focus
                    .contains_focused(window, cx);
            if !self.content_ui.menu
                || !self
                    .content_ui
                    .composer_menu
                    .session
                    .accepts(target, self.snapshot.selected)
                || !self.composer_menu_context_current()
                || self.message_ui.menu.target().is_some()
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
                self.dismiss_composer_menu(false, window, cx);
            }
        }
        let mut geometry = self.content_ui.composer_menu.geometry.borrow_mut();
        geometry.anchor = None;
        geometry.panel = None;
    }

    pub(crate) fn composer_menu_popup(
        &self,
        window: &Window,
        cx: &Context<Self>,
    ) -> Option<AnyElement> {
        let target = self.content_ui.composer_menu.session.target?;
        let items = self.composer_menu_items(target);
        if items.is_empty() {
            return None;
        }
        let geometry = self.content_ui.composer_menu.geometry.clone();
        let outside_geometry = geometry.clone();
        let width = self
            .sp(260.)
            .min((window.viewport_size().width - px(24.)).max(px(1.)));
        let silent = self.drafts.target(target.context).silent;
        let body = div()
            .id(scoped_id(
                "composer-popup",
                target.context,
                0,
                u128::from(target.generation),
            ))
            .track_focus(&self.content_ui.composer_menu.focus)
            .key_context("ComposerMenu")
            .occlude()
            .w(width)
            .max_h((window.viewport_size().height - px(24.)).max(px(1.)))
            .overflow_y_scroll()
            .track_scroll(&self.content_ui.composer_menu.scroll)
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
                    if this.content_ui.composer_menu.session.target == Some(target)
                        && !outside_geometry
                            .borrow()
                            .anchor
                            .is_some_and(|bounds| bounds.contains(&event.position))
                    {
                        // Let the clicked native control receive its normal focus.
                        this.dismiss_composer_menu(false, window, cx);
                    }
                },
            ))
            .children(items.into_iter().map(|item| {
                let id = item.choice.id(target);
                self.control(
                    id.clone(),
                    if item.choice == Choice::Silent && silent {
                        ButtonStyle::Secondary
                    } else {
                        ButtonStyle::Quiet
                    },
                    item.availability.hinted(),
                    LocalAction::Content(crate::presentation::content::Action::ComposerChoice(
                        target,
                        item.choice,
                    )),
                    cx,
                )
                .w_full()
                .min_w_0()
                .justify_start()
                .gap_2()
                .child(self.scaled_icon(item.icon, 15., self.theme.muted))
                .child(div().min_w_0().truncate().child(self.l(item.label)))
                .when(item.choice == Choice::Silent && silent, |d| {
                    d.child(self.scaled_icon("check", 14., self.theme.accent))
                })
                .child(self.reason_tip(
                    id,
                    item.availability,
                    Some(item.label),
                    cx,
                ))
            }));
        Some(
            Popup {
                target,
                geometry,
                child: body.into_any_element(),
                width,
            }
            .into_any_element(),
        )
    }

    pub(crate) fn finish_composer_menu_frame(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(target) = self.content_ui.composer_menu.session.target else {
            return;
        };
        if self
            .content_ui
            .composer_menu
            .geometry
            .borrow()
            .panel
            .is_none()
        {
            self.dismiss_composer_menu(true, window, cx);
            return;
        }
        if self.content_ui.composer_menu.pending_focus {
            self.content_ui.composer_menu.pending_focus = false;
            if let Some((index, focus)) = self
                .composer_menu_items(target)
                .iter()
                .enumerate()
                .filter(|(_, item)| item.availability.is_enabled())
                .find_map(|(index, item)| {
                    self.control_focus
                        .current(&item.choice.id(target))
                        .map(|focus| (index, focus))
                })
            {
                focus.focus(window);
                self.content_ui.composer_menu.scroll.scroll_to_item(index);
            } else {
                self.content_ui.composer_menu.focus.focus(window);
            }
            cx.notify();
        }
    }

    pub(crate) fn composer_menu_key(
        &mut self,
        event: &KeyDownEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> bool {
        let Some(target) = self.content_ui.composer_menu.session.target else {
            return false;
        };
        let key = event.keystroke.key.as_str();
        if matches!(key, "escape" | "tab") {
            self.dismiss_composer_menu(true, window, cx);
            self.keyboard_navigation = true;
            return true;
        }
        if event.keystroke.modifiers.modified() || !matches!(key, "up" | "down" | "home" | "end") {
            return false;
        }
        let focus: Vec<_> = self
            .composer_menu_items(target)
            .iter()
            .enumerate()
            .filter(|(_, item)| item.availability.is_enabled())
            .filter_map(|(index, item)| {
                self.control_focus
                    .current(&item.choice.id(target))
                    .map(|handle| (index, handle))
            })
            .collect();
        let current = focus
            .iter()
            .position(|(_, handle)| handle.is_focused(window));
        if let Some(next) = next_index(current, focus.len(), key)
            && let Some((row, handle)) = focus.get(next)
        {
            handle.focus(window);
            self.content_ui.composer_menu.scroll.scroll_to_item(*row);
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

/// Layout uses this frame's plus-button bounds after the composer prepaints.
/// Mounting beside the card keeps the menu and focus feedback outside its clip.
struct Popup {
    target: Target,
    geometry: Rc<RefCell<Geometry>>,
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
        let viewport = window.viewport_size();
        if anchor.bottom() <= px(0.)
            || anchor.top() >= viewport.height
            || anchor.right() <= px(0.)
            || anchor.left() >= viewport.width
        {
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
        let origin = popup_origin(anchor, size, viewport);
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
    let left = anchor
        .left()
        .max(margin)
        .min((viewport.width - size.width - margin).max(px(0.)));
    let above = anchor.top() - size.height - px(4.);
    let top = if above >= margin {
        above
    } else {
        anchor.bottom() + px(4.)
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
    fn reopened_and_replaced_room_reject_old_popup_callbacks() {
        let mut session = Session::default();
        let old = session.open(room(1)).unwrap();
        assert!(session.accepts(old, Some(room(1))));
        assert!(!session.accepts(old, Some(room(2))));
        session.target = None;
        assert!(!session.accepts(old, Some(room(1))));
        let new = session.open(room(1)).unwrap();
        assert_ne!(old, new);
        assert!(!session.accepts(old, Some(room(1))));
        assert!(session.accepts(new, Some(room(1))));
        session.generation = u64::MAX;
        assert!(session.open(room(2)).is_none());
    }
    #[test]
    fn popup_prefers_above_and_stays_inside_every_viewport_edge() {
        let viewport = gpui::size(px(800.), px(600.));
        let size = gpui::size(px(260.), px(244.));
        let low = Bounds::new(gpui::point(px(34.), px(540.)), gpui::size(px(32.), px(32.)));
        assert_eq!(
            popup_origin(low, size, viewport),
            gpui::point(px(34.), px(292.))
        );
        for (x, y) in [(0., 0.), (780., 0.), (0., 580.), (780., 580.)] {
            let anchor = Bounds::new(gpui::point(px(x), px(y)), gpui::size(px(32.), px(32.)));
            let origin = popup_origin(anchor, size, viewport);
            assert!(origin.x >= px(12.) && origin.y >= px(12.));
            assert!(origin.x + size.width <= viewport.width - px(12.));
            assert!(origin.y + size.height <= viewport.height - px(12.));
        }
    }
    #[test]
    fn keyboard_navigation_wraps_and_handles_an_empty_menu() {
        assert_eq!(next_index(None, 0, "down"), None);
        assert_eq!(next_index(None, 3, "up"), Some(2));
        assert_eq!(next_index(Some(2), 3, "down"), Some(0));
        assert_eq!(next_index(Some(0), 3, "up"), Some(2));
        assert_eq!(next_index(Some(1), 3, "home"), Some(0));
        assert_eq!(next_index(Some(1), 3, "end"), Some(2));
    }
}
