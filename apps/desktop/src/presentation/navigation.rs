//! Private keyboard navigation and focus revelation for the finite native views.
use super::{
    Desktop, LocalAction, Overlay, SearchKind,
    theme::{Appearance, Palette},
};
use gpui::{
    App, Bounds, Context, Div, ElementId, FocusHandle, KeyDownEvent, Pixels, ScrollHandle,
    Stateful, Window, div, prelude::*, px,
};
use std::{cell::RefCell, rc::Rc};

#[derive(Clone, Copy)]
pub(super) enum Group {
    Appearance,
    Palette,
    Scale,
    Density,
    Details,
    Search,
}
impl Group {
    fn count(self) -> usize {
        match self {
            Self::Appearance | Self::Details => 3,
            Self::Palette | Self::Density => 2,
            Self::Scale => 4,
            Self::Search => 5,
        }
    }
    fn id(self, i: usize) -> ElementId {
        match self {
            Self::Appearance => ("appearance-card", i).into(),
            Self::Palette => ("palette-card", i).into(),
            Self::Details => ("details-tab", i).into(),
            Self::Search => ("search-tab", i).into(),
            Self::Scale => ("text-scale", [100usize, 125, 150, 200][i]).into(),
            Self::Density => if i == 0 {
                "density-compact"
            } else {
                "density-relaxed"
            }
            .into(),
        }
    }
    fn selected(self, d: &Desktop) -> usize {
        match self {
            Self::Appearance => match d.appearance {
                Appearance::System => 0,
                Appearance::Light => 1,
                Appearance::Dark => 2,
            },
            Self::Palette => usize::from(d.palette == Palette::Sage),
            Self::Scale => [100, 125, 150, 200]
                .iter()
                .position(|n| *n == d.scale)
                .unwrap_or(0),
            Self::Density => usize::from(!d.compact),
            Self::Details => match d.overlay {
                Some(Overlay::Participants) => 1,
                Some(Overlay::Files) => 2,
                _ => 0,
            },
            Self::Search => match d.search_kind {
                SearchKind::All => 0,
                SearchKind::Messages => 1,
                SearchKind::Files => 2,
                SearchKind::Conversations => 3,
                SearchKind::People => 4,
            },
        }
    }
    pub fn for_action(action: &LocalAction) -> Option<Self> {
        match action {
            LocalAction::Scale(_) => Some(Self::Scale),
            LocalAction::Density(_) => Some(Self::Density),
            LocalAction::SearchKind(_) => Some(Self::Search),
            LocalAction::Open(Overlay::RoomInfo | Overlay::Participants | Overlay::Files) => {
                Some(Self::Details)
            }
            _ => None,
        }
    }
}
// Exactly these surfaces own scroll handles; no per-item scroll cache.
#[derive(Clone, Copy)]
pub(super) enum Region {
    Settings,
    Onboarding,
    Rooms,
    Timeline,
    Conversation,
    Overlay,
    Prejoin,
    Call,
}
impl Region {
    fn index(self) -> usize {
        self as usize
    }
    fn name(self) -> &'static str {
        match self {
            Self::Settings => "settings-scroll",
            Self::Onboarding => "onboarding-scroll",
            Self::Rooms => "conversation-list",
            Self::Timeline => "timeline",
            Self::Conversation => "conversation-resize-scroll",
            Self::Overlay => "overlay-scroll",
            Self::Prejoin => "prejoin-scroll",
            Self::Call => "call-scroll",
        }
    }
}
struct Viewport {
    scroll: ScrollHandle,
    focus: FocusHandle,
}
pub(super) struct Navigation {
    viewports: [Viewport; 8],
    pub reveal: Reveal,
    pub hints: HintGeometry,
    history_scope: Option<talk_core::ui::presentation::ConversationRef>,
    locating_message: bool,
}
// Exactly two optional rectangles per window: focused and delayed-hover control.
// Element callbacks own frame-local clones; no per-item geometry map exists.
#[derive(Clone, Default)]
pub(super) struct HintGeometry(Rc<RefCell<HintBounds>>);
#[derive(Default)]
struct HintBounds {
    focused: Option<(ElementId, Bounds<Pixels>)>,
    hovered: Option<(ElementId, Bounds<Pixels>)>,
}
impl HintGeometry {
    pub fn begin(&self) {
        *self.0.borrow_mut() = HintBounds::default();
    }
    pub fn record(&self, id: ElementId, bounds: Bounds<Pixels>, focused: bool, hovered: bool) {
        let mut state = self.0.borrow_mut();
        if focused {
            state.focused = Some((id.clone(), bounds));
        }
        if hovered {
            state.hovered = Some((id, bounds));
        }
    }
    pub fn get(&self, id: &ElementId) -> Option<Bounds<Pixels>> {
        let state = self.0.borrow();
        [&state.focused, &state.hovered]
            .into_iter()
            .flatten()
            .find(|(key, _)| key == id)
            .map(|(_, bounds)| *bounds)
    }
}
#[derive(Clone, Default)]
pub(super) struct Reveal(Rc<RefCell<RevealState>>);
#[derive(Default)]
struct RevealState {
    bounds: Option<Bounds<Pixels>>,
    pending: bool,
    adjusted: bool,
    rooms_overflow: bool,
}
impl Reveal {
    pub fn geometry(&self) -> Option<Bounds<Pixels>> {
        self.0.borrow().bounds
    }
    pub fn request(&self) {
        self.0.borrow_mut().pending = true;
    }
    pub fn begin(&self) {
        let mut s = self.0.borrow_mut();
        s.bounds = None;
        s.adjusted = false;
    }
    pub fn record(&self, bounds: Bounds<Pixels>) {
        self.0.borrow_mut().bounds = Some(bounds);
    }
    pub fn finish(&self) {
        let mut s = self.0.borrow_mut();
        s.pending = s.adjusted;
    }
    fn observe_rooms_overflow(&self, viewport: &ScrollHandle, window: &mut Window) {
        let value = viewport.max_offset().height > px(1.);
        let mut state = self.0.borrow_mut();
        if state.rooms_overflow != value {
            state.rooms_overflow = value;
            // One redraw only when measured overflow changes, including first
            // layout. No polling and no per-row state or additional scroll owner.
            window.on_next_frame(|window, _| window.refresh());
        }
    }
    fn reveal(&self, viewport: &ScrollHandle, window: &mut Window) {
        let mut s = self.0.borrow_mut();
        if !s.pending {
            return;
        }
        let Some(target) = s.bounds else {
            return;
        };
        let area = viewport.bounds();
        let dy = reveal_delta(
            f32::from(target.top()),
            f32::from(target.bottom()),
            f32::from(area.top()) + 8.,
            f32::from(area.bottom()) - 8.,
        );
        if dy.abs() < 0.5 {
            return;
        }
        let old = viewport.offset();
        let next = (old.y + px(dy)).clamp(-viewport.max_offset().height, px(0.));
        if (next - old.y).abs() < px(0.5) {
            return;
        }
        viewport.set_offset(gpui::point(old.x, next));
        if !s.adjusted {
            // refresh() is deliberately ignored during GPUI drawing. Queue one
            // redraw after this frame so the new scroll offset becomes visible
            // without requiring a second key or any pointer wheel movement.
            window.on_next_frame(|window, _| window.refresh());
        }
        s.adjusted = true;
    }

    pub fn reveal_history(
        &self,
        list: &gpui::ListState,
        header_inset: Pixels,
        painted_anchor: Option<gpui::ListOffset>,
        window: &mut Window,
    ) -> bool {
        let mut s = self.0.borrow_mut();
        if !s.pending {
            return false;
        }
        let Some(target) = s.bounds else {
            return false;
        };
        let area = list.viewport_bounds();
        let dy = reveal_delta(
            f32::from(target.top()),
            f32::from(target.bottom()),
            f32::from(area.top() + header_inset) + 8.,
            f32::from(area.bottom()) - 8.,
        );
        if dy.abs() < 0.5 {
            return false;
        }
        // Bottom alignment exposes an end sentinel. Use the same frame's
        // measured first row before asking GPUI for a relative displacement.
        if list.logical_scroll_top().item_ix >= list.item_count()
            && let Some(anchor) = painted_anchor
        {
            list.scroll_to(anchor);
        }
        let old = list.logical_scroll_top();
        list.scroll_by(px(-dy));
        let next = list.logical_scroll_top();
        if old.item_ix == next.item_ix && (old.offset_in_item - next.offset_in_item).abs() < px(0.5)
        {
            return false;
        }
        if !s.adjusted {
            window.on_next_frame(|window, _| window.refresh());
        }
        s.adjusted = true;
        true
    }
}
fn reading_latest(offset: f32, maximum: f32) -> bool {
    (offset + maximum).abs() <= 2.
}
// Minimal displacement, with the start visible when a control itself exceeds
// the viewport. This avoids alternating top/bottom jumps for large text.
fn reveal_delta(top: f32, bottom: f32, start: f32, end: f32) -> f32 {
    if top < start || bottom - top > end - start {
        start - top
    } else if bottom > end {
        end - bottom
    } else {
        0.
    }
}
impl Navigation {
    pub fn geometry(&self) -> [Bounds<Pixels>; 8] {
        std::array::from_fn(|i| self.viewports[i].scroll.bounds())
    }
    pub fn prepare_history(&mut self, scope: Option<talk_core::ui::presentation::ConversationRef>) {
        let scroll = &self.viewports[Region::Timeline.index()].scroll;
        if !self.locating_message
            && (scope != self.history_scope
                || reading_latest(
                    f32::from(scroll.offset().y),
                    f32::from(scroll.max_offset().height),
                ))
        {
            scroll.scroll_to_bottom();
        }
        self.history_scope = scope;
        self.locating_message = false;
    }
    pub fn locate_message(&mut self, index: usize, window: &mut Window) {
        self.locating_message = true;
        let v = &self.viewports[Region::Timeline.index()];
        v.scroll.scroll_to_item(index + 1); // first child is the truthful local history divider
        v.focus.focus(window);
    }
    pub fn history_focus(&self) -> FocusHandle {
        self.viewports[Region::Timeline.index()].focus.clone()
    }
    pub fn reset_onboarding(&self) {
        self.viewports[Region::Onboarding.index()]
            .scroll
            .set_offset(gpui::point(px(0.), px(0.)));
    }
    pub fn history_geometry(&self) -> (gpui::Point<Pixels>, gpui::Size<Pixels>) {
        let s = &self.viewports[Region::Timeline.index()].scroll;
        (s.offset(), s.max_offset())
    }
    pub fn new(cx: &App) -> Self {
        Self {
            viewports: std::array::from_fn(|_| Viewport {
                scroll: ScrollHandle::new(),
                focus: cx.focus_handle(),
            }),
            reveal: Reveal::default(),
            hints: HintGeometry::default(),
            history_scope: None,
            locating_message: false,
        }
    }
}
impl Desktop {
    pub(super) fn scroll_area(&self, region: Region) -> Stateful<Div> {
        let viewport = &self.navigation.viewports[region.index()];
        let scroll = viewport.scroll.clone();
        let group = viewport.focus.clone();
        let reveal = self.navigation.reveal.clone();
        let history = matches!(region, Region::Timeline);
        let keyboard_scroll = scroll.clone();
        let keyboard_focus = group.clone();
        div()
            .on_children_prepainted(move |_, window, cx| {
                if matches!(region, Region::Rooms) {
                    reveal.observe_rooms_overflow(&scroll, window);
                }
                if group.contains_focused(window, cx) {
                    reveal.reveal(&scroll, window);
                }
            })
            .id(region.name())
            .track_scroll(&viewport.scroll)
            .track_focus(
                &viewport
                    .focus
                    .clone()
                    .tab_stop(history && !self.background_inert),
            )
            .on_key_down(move |e: &KeyDownEvent, window, cx| {
                // Only the history itself owns these keys. Editor/IME and child
                // controls keep their own navigation and caret behavior.
                if !history || !keyboard_focus.is_focused(window) {
                    return;
                }
                let offset = keyboard_scroll.offset();
                let max = keyboard_scroll.max_offset().height;
                let page = keyboard_scroll.bounds().size.height * 0.85;
                let next = match e.keystroke.key.as_str() {
                    "pageup" => offset.y + page,
                    "pagedown" => offset.y - page,
                    "home" => px(0.),
                    "end" => -max,
                    _ => return,
                };
                keyboard_scroll.set_offset(gpui::point(offset.x, next.clamp(-max, px(0.))));
                cx.stop_propagation();
                window.refresh();
            })
            .overflow_y_scroll()
    }
    pub(super) fn roving(
        &self,
        control: Stateful<Div>,
        group: Group,
        index: usize,
        cx: &Context<Self>,
    ) -> Stateful<Div> {
        let selected = group.selected(self);
        control
            .when_some(self.control_focus(group.id(index), cx), |d, f| {
                d.track_focus(&f.tab_stop(index == selected && !self.background_inert))
            })
            .on_key_down(cx.listener(move |this, e: &KeyDownEvent, window, cx| {
                let key = e.keystroke.key.as_str();
                let next = match key {
                    "left" | "up" => Some((index + group.count() - 1) % group.count()),
                    "right" | "down" => Some((index + 1) % group.count()),
                    "home" => Some(0),
                    "end" => Some(group.count() - 1),
                    _ => None,
                };
                if key == "tab" {
                    // GPUI's map starts from the focused non-tabstop; normalize
                    // to the group's selected entry before leaving, in this key
                    // event, without an intermediate paint or activation.
                    if let Some(entry) = this.control_focus(group.id(group.selected(this)), cx) {
                        entry.focus(window);
                    }
                    if e.keystroke.modifiers.shift {
                        window.focus_prev()
                    } else {
                        window.focus_next()
                    }
                } else if let Some(next) = next {
                    if let Some(focus) = this.control_focus(group.id(next), cx) {
                        focus.focus(window);
                    }
                } else {
                    return;
                }
                this.navigation.reveal.request();
                cx.stop_propagation();
                cx.notify();
            }))
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn history_follows_latest_but_never_resets_an_older_reading_position() {
        assert!(reading_latest(0., 0.));
        assert!(reading_latest(-3400., 3400.));
        assert!(reading_latest(-3399., 3400.));
        assert!(!reading_latest(0., 3400.));
        assert!(!reading_latest(-1700., 3400.));
    }
    #[test]
    fn focus_revelation_uses_minimal_scroll_and_large_controls_do_not_oscillate() {
        assert_eq!(reveal_delta(40., 70., 10., 100.), 0.);
        assert_eq!(reveal_delta(90., 120., 10., 100.), -20.);
        assert_eq!(reveal_delta(-20., 30., 10., 100.), 30.);
        assert_eq!(reveal_delta(10., 210., 10., 100.), 0.);
        assert_eq!(reveal_delta(-80., 120., 10., 100.), 90.);
    }
}
