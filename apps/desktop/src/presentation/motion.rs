//! Finite decorative motion. Input, layout and focus always take effect immediately.
use super::{Desktop, Overlay};
use gpui::{Animation, AnimationExt, AnyElement, Div, ElementId, div, prelude::*};
use std::time::{Duration, Instant};
mod reveal;

pub(super) const REVEAL: Duration = Duration::from_millis(140);
pub(super) const CONTROL: Duration = Duration::from_millis(110);
pub(super) const SHIMMER: Duration = Duration::from_millis(1_600);
pub(super) const PIN_FLIGHT: Duration = Duration::from_millis(300);
pub(super) const PIN_SHRED: Duration = Duration::from_millis(220);
pub(super) const PIN_SCROLL: Duration = Duration::from_millis(280);
const DIALOG: Duration = REVEAL;
const DIALOG_SURFACES: usize = 20;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum DialogSurface {
    Overlay(Overlay),
    AccountRemoval,
    Prejoin,
}

/// One decorative admission per window. The bounded session remembers surfaces
/// already opened so returning from a nested dialog cannot replay its parent.
/// No content, task, closing view or authority is retained by this owner.
pub(super) struct DialogMotion {
    active: Option<DialogSurface>,
    seen: Vec<DialogSurface>,
    entrance: Option<(DialogSurface, Instant)>,
}
impl Default for DialogMotion {
    fn default() -> Self {
        Self {
            active: None,
            seen: Vec::with_capacity(DIALOG_SURFACES),
            entrance: None,
        }
    }
}
impl DialogMotion {
    pub fn prepare(&mut self, surface: Option<DialogSurface>, reduced: bool, now: Instant) {
        let changed = self.active != surface;
        self.active = surface;
        let Some(surface) = surface else {
            self.seen.clear();
            self.entrance = None;
            return;
        };
        if changed {
            self.entrance = None;
            if !self.seen.contains(&surface) && self.seen.len() < DIALOG_SURFACES {
                self.seen.push(surface);
                if !reduced {
                    self.entrance = Some((surface, now));
                }
            }
            // At capacity a further surface is static; interaction is admitted
            // normally and an existing surface is never evicted or replayed.
        }
        if reduced {
            self.entrance = None;
        }
    }
    fn admission(&self, surface: DialogSurface) -> Option<Instant> {
        self.entrance
            .filter(|(active, _)| self.active == Some(surface) && *active == surface)
            .map(|(_, at)| at)
    }
}

fn dialog_animation_id(layer: &'static str, surface: DialogSurface) -> ElementId {
    gpui::SharedString::from(format!("{layer}-{surface:?}")).into()
}

fn dialog_panel_kind(surface: DialogSurface) -> reveal::Kind {
    if surface == DialogSurface::Overlay(Overlay::Thread) {
        reveal::Kind::ThreadPanel
    } else {
        reveal::Kind::Panel
    }
}

pub(super) fn count_frame(counter: &Option<std::rc::Rc<std::cell::Cell<u64>>>) {
    if let Some(counter) = counter {
        counter.set(counter.get().saturating_add(1));
    }
}

pub(super) enum TriggerKind {
    Switch(bool),
    Reveal,
    Pulse,
}

#[derive(Debug, Eq, PartialEq)]
pub(super) enum UnreadUpdate {
    Retire,
    Keep,
    Admit(talk_core::ui::presentation::ConversationRef),
}

/// A projection baseline, account change or newly loaded room is not an
/// incoming unread event. Retain at most one decoration for the window.
pub(super) fn unread_update(
    previous: &talk_core::ui::presentation::PresentationSnapshot,
    next: &talk_core::ui::presentation::PresentationSnapshot,
    reduced: bool,
) -> UnreadUpdate {
    use talk_core::ui::presentation::ContentState;
    if reduced
        || previous.active_account.is_none()
        || previous.active_account != next.active_account
        || previous.conversation_list.content != ContentState::Ready
        || next.conversation_list.content != ContentState::Ready
    {
        return UnreadUpdate::Retire;
    }
    next.conversations
        .as_slice()
        .iter()
        .find_map(|row| {
            previous
                .conversations
                .as_slice()
                .iter()
                .find(|old| old.context == row.context && row.unread > old.unread)
                .map(|_| UnreadUpdate::Admit(row.context))
        })
        .unwrap_or(UnreadUpdate::Keep)
}

/// One recent deliberate action per window, never a per-control cache.
/// A first mount, stale response, unrelated action or return to a dialog cannot
/// animate a switch. Virtualized menu remounts cannot restart a reveal.
pub(super) struct MotionTrigger {
    pub id: ElementId,
    pub kind: TriggerKind,
    pub admitted: Instant,
}

/// Which message row owns the hover toolbar. The pointer may occupy the row
/// body or the toolbar itself, which overlaps the row's top edge and so lies
/// outside the row's own hit area. Either part keeps the toolbar visible; a late
/// leave from an older row cannot clear the new owner. Reduced Motion keeps the
/// ownership (visibility is behavior) and only drops the reveal admission.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum HoverPart {
    Row,
    Toolbar,
}
#[derive(Default)]
pub(super) struct HoverMotion {
    /// Row whose toolbar began revealing, and when; moving within it never restarts.
    target: Option<(ElementId, Instant)>,
    /// The single row currently occupied by the pointer: (id, in row, in toolbar).
    active: Option<(ElementId, bool, bool)>,
}
impl HoverMotion {
    pub fn update(&mut self, id: &ElementId, hovered: bool, reduced: bool, now: Instant) -> bool {
        self.set(id, HoverPart::Row, hovered, reduced, now)
    }

    pub fn update_toolbar(
        &mut self,
        id: &ElementId,
        hovered: bool,
        reduced: bool,
        now: Instant,
    ) -> bool {
        self.set(id, HoverPart::Toolbar, hovered, reduced, now)
    }

    fn set(
        &mut self,
        id: &ElementId,
        part: HoverPart,
        hovered: bool,
        reduced: bool,
        now: Instant,
    ) -> bool {
        let current = self
            .active
            .as_ref()
            .is_some_and(|(active, ..)| active == id);
        if !current {
            if !hovered {
                return false;
            }
            self.active = Some((
                id.clone(),
                part == HoverPart::Row,
                part == HoverPart::Toolbar,
            ));
            self.target = (!reduced).then(|| (id.clone(), now));
            return true;
        }
        let Some((_, row, toolbar)) = self.active.as_mut() else {
            return false;
        };
        let previous = match part {
            HoverPart::Row => std::mem::replace(row, hovered),
            HoverPart::Toolbar => std::mem::replace(toolbar, hovered),
        };
        if previous == hovered {
            return false;
        }
        if !*row && !*toolbar {
            self.active = None;
            self.target = None;
        }
        true
    }

    pub fn visible(&self, id: &ElementId) -> bool {
        self.active
            .as_ref()
            .is_some_and(|(active, ..)| active == id)
    }

    pub fn admission(&self, id: &ElementId, now: Instant) -> Option<Instant> {
        self.target
            .as_ref()
            .filter(|(target, at)| target == id && now.saturating_duration_since(*at) < REVEAL)
            .map(|(_, at)| *at)
    }

    pub fn clear(&mut self) {
        self.target = None;
        self.active = None;
    }
}
impl MotionTrigger {
    fn switch_from(&self, id: &ElementId, value: Option<bool>, now: Instant) -> Option<bool> {
        let TriggerKind::Switch(from) = self.kind else {
            return None;
        };
        (self.id == *id
            && value == Some(!from)
            && now.saturating_duration_since(self.admitted) < Duration::from_millis(500))
        .then_some(from)
    }

    fn reveal_elapsed(&self, id: &ElementId, now: Instant) -> Option<Duration> {
        let elapsed = now.saturating_duration_since(self.admitted);
        (self.id == *id && matches!(self.kind, TriggerKind::Reveal) && elapsed < REVEAL)
            .then_some(elapsed)
    }

    fn pulse_elapsed(&self, id: &ElementId, now: Instant) -> Option<Duration> {
        let elapsed = now.saturating_duration_since(self.admitted);
        (self.id == *id && matches!(self.kind, TriggerKind::Pulse) && elapsed < CONTROL)
            .then_some(elapsed)
    }
}

impl Desktop {
    /// A short wash responds to a deliberate send or reaction without moving
    /// the native control or implying that the underlying operation succeeded.
    pub(super) fn action_pulse(&self, id: ElementId, color: gpui::Hsla, radius: f32) -> AnyElement {
        let admitted = self
            .motion_trigger
            .as_ref()
            .filter(|trigger| trigger.pulse_elapsed(&id, Instant::now()).is_some())
            .map(|trigger| trigger.admitted);
        reveal::ActionReveal::new(
            gpui::SharedString::from(format!("action-pulse:{id}")).into(),
            div()
                .absolute()
                .inset(gpui::px(2.))
                .rounded(gpui::px(radius))
                .bg(color),
            admitted,
            !self.reduced_motion,
            self.motion_frame_counter(),
        )
        .kind(reveal::Kind::Pulse)
        .into_any_element()
    }

    pub(super) fn unread_badge(
        &self,
        id: ElementId,
        context: talk_core::ui::presentation::ConversationRef,
        badge: Div,
    ) -> AnyElement {
        let admitted = self
            .unread_pop
            .filter(|(room, at)| *room == context && at.elapsed() < REVEAL)
            .map(|(_, at)| at);
        reveal::ActionReveal::new(
            id,
            badge,
            admitted,
            !self.reduced_motion,
            self.motion_frame_counter(),
        )
        .kind(reveal::Kind::Badge)
        .into_any_element()
    }
    /// The live recording indicator becomes static under Reduce Motion.
    pub(super) fn recording_dot(&self) -> AnyElement {
        let dot = div()
            .size(gpui::px(10.))
            .rounded_full()
            .bg(self.theme.danger);
        if self.reduced_motion {
            dot.into_any_element()
        } else {
            dot.with_animation(
                "voice-recording-dot",
                Animation::new(Duration::from_millis(1_200)).repeat(),
                |dot, progress| dot.opacity(0.55 + 0.45 * (1. - (2. * progress - 1.).abs())),
            )
            .into_any_element()
        }
    }

    pub(super) fn call_live_dot(&self) -> AnyElement {
        let dot = div()
            .size(gpui::px(6.))
            .rounded_full()
            .bg(self.theme.success);
        if self.reduced_motion {
            dot.into_any_element()
        } else {
            dot.with_animation(
                "room-live-call-dot",
                Animation::new(Duration::from_millis(1600)).repeat(),
                |dot, progress| dot.opacity(0.7 + 0.3 * (1. - (2. * progress - 1.).abs())),
            )
            .into_any_element()
        }
    }

    /// A waiting indicator belongs to its mounted call surface and does not
    /// create a task. Reduced Motion leaves the same icon stationary.
    pub(super) fn call_spinner(&self, id: ElementId, size: f32, color: gpui::Hsla) -> AnyElement {
        let icon = self.scaled_icon("refresh", size, color);
        if self.reduced_motion {
            icon.into_any_element()
        } else {
            icon.with_animation(
                id,
                Animation::new(Duration::from_millis(1_200)).repeat(),
                |icon, progress| {
                    icon.with_transformation(gpui::Transformation::rotate(gpui::radians(
                        progress * std::f32::consts::TAU,
                    )))
                },
            )
            .into_any_element()
        }
    }

    pub(super) fn incoming_call_avatar(&self, avatar: Div) -> AnyElement {
        let ring = div()
            .p(gpui::px(6.))
            .rounded_full()
            .border_2()
            .border_color(self.theme.accent)
            .child(avatar);
        if self.reduced_motion {
            ring.into_any_element()
        } else {
            ring.with_animation(
                "incoming-call-ring",
                Animation::new(Duration::from_millis(1600)).repeat(),
                |ring, progress| ring.opacity(0.72 + 0.28 * (1. - (2. * progress - 1.).abs())),
            )
            .into_any_element()
        }
    }

    /// Static bars beside the speaking ring. Someone speaks for most of a call,
    /// so a repeating pulse here redrew the whole window at display rate.
    pub(super) fn call_speaking_bars(&self, color: gpui::Hsla) -> Div {
        div()
            .flex()
            .items_end()
            .gap_1()
            .h(gpui::px(12.))
            .children([5., 10., 7.].into_iter().map(|height| {
                div()
                    .w(gpui::px(2.))
                    .h(gpui::px(height))
                    .rounded_full()
                    .bg(color)
            }))
    }

    /// All application dialog panels share the same finite entrance. Relative
    /// positioning moves paint and native hit geometry together; focus and
    /// actions are live from the first frame. Closing remains immediate.
    pub(super) fn dialog_panel(&self, surface: DialogSurface, panel: Div) -> AnyElement {
        reveal::ActionReveal::new(
            dialog_animation_id("dialog-panel", surface),
            panel,
            self.dialog_motion.admission(surface),
            !self.reduced_motion,
            self.motion_frame_counter(),
        )
        .kind(dialog_panel_kind(surface))
        .into_any_element()
    }

    pub(super) fn dialog_backdrop(&self, surface: DialogSurface, backdrop: Div) -> AnyElement {
        reveal::ActionReveal::new(
            dialog_animation_id("dialog-backdrop", surface),
            backdrop,
            self.dialog_motion.admission(surface),
            !self.reduced_motion,
            self.motion_frame_counter(),
        )
        .kind(reveal::Kind::Backdrop)
        .into_any_element()
    }

    pub(super) fn motion_frame_counter(&self) -> Option<std::rc::Rc<std::cell::Cell<u64>>> {
        self.development.then(|| self.motion_frames.clone())
    }

    pub(super) fn begin_surface_reveal(&mut self, id: ElementId) {
        if !self.reduced_motion {
            self.motion_trigger = Some(MotionTrigger {
                id,
                kind: TriggerKind::Reveal,
                admitted: Instant::now(),
            });
        }
    }

    pub(super) fn reveal_from_action(&self, id: ElementId, element: Div) -> AnyElement {
        let admitted = self
            .motion_trigger
            .as_ref()
            .filter(|trigger| trigger.reveal_elapsed(&id, Instant::now()).is_some())
            .map(|trigger| trigger.admitted);
        reveal::ActionReveal::new(
            id,
            element,
            admitted,
            !self.reduced_motion,
            self.motion_frame_counter(),
        )
        .into_any_element()
    }

    pub(super) fn poll_result_reveal(&self, poll: u64, option: u16, element: Div) -> AnyElement {
        let trigger: ElementId = ("poll-results", poll as usize).into();
        let admitted = self
            .motion_trigger
            .as_ref()
            .filter(|motion| motion.reveal_elapsed(&trigger, Instant::now()).is_some())
            .map(|motion| motion.admitted);
        reveal::ActionReveal::new(
            (
                "poll-result-bar",
                ((poll as usize) << 8) ^ usize::from(option),
            )
                .into(),
            element,
            admitted,
            !self.reduced_motion,
            self.motion_frame_counter(),
        )
        .into_any_element()
    }
    pub(super) fn reveal_observation(
        &self,
        id: ElementId,
        element: Div,
        observed: Option<Instant>,
    ) -> AnyElement {
        reveal::ActionReveal::new(
            id,
            element,
            observed,
            !self.reduced_motion,
            self.motion_frame_counter(),
        )
        .into_any_element()
    }

    /// The hover toolbar eases down four points while it fades in; it keeps
    /// its stable identity so a press that spans the reveal is not discarded.
    pub(super) fn reveal_toolbar(
        &self,
        id: ElementId,
        element: Div,
        observed: Option<Instant>,
    ) -> AnyElement {
        reveal::ActionReveal::new(
            id,
            element,
            observed,
            !self.reduced_motion,
            self.motion_frame_counter(),
        )
        .kind(reveal::Kind::Toolbar)
        .into_any_element()
    }

    pub(super) fn reveal(&self, id: impl Into<ElementId>, element: Div) -> AnyElement {
        if self.reduced_motion {
            element.into_any_element()
        } else {
            let frames = self.motion_frame_counter();
            element
                .with_animation(
                    id,
                    Animation::new(REVEAL).with_easing(gpui::ease_out_quint()),
                    move |element, progress| {
                        count_frame(&frames);
                        element.opacity(0.6 + progress * 0.4)
                    },
                )
                .into_any_element()
        }
    }

    /// A single subtle pulse acknowledges a raised hand without a perpetual
    /// frame loop; reduced motion keeps the badge visible and static.
    pub(super) fn call_hand_badge(&self, id: ElementId, badge: Div) -> AnyElement {
        if self.reduced_motion {
            badge.into_any_element()
        } else {
            let frames = self.motion_frame_counter();
            badge
                .with_animation(
                    id,
                    Animation::new(Duration::from_millis(700)),
                    move |element, progress| {
                        count_frame(&frames);
                        element.opacity(1. - 0.25 * (std::f32::consts::PI * progress).sin())
                    },
                )
                .into_any_element()
        }
    }

    pub(super) fn call_reaction_motion(&self, id: ElementId, element: Div) -> AnyElement {
        if self.reduced_motion {
            element.into_any_element()
        } else {
            let frames = self.motion_frame_counter();
            element
                .with_animation(
                    id,
                    Animation::new(Duration::from_millis(1600)).with_easing(gpui::ease_out_quint()),
                    move |element, progress| {
                        count_frame(&frames);
                        element
                            .bottom(gpui::relative(0.5 + 0.18 * progress))
                            .opacity(1. - progress)
                    },
                )
                .into_any_element()
        }
    }

    pub(super) fn control_wash(&self, color: gpui::Hsla, radius: f32, border: f32) -> AnyElement {
        let wash = div()
            .absolute()
            .inset(gpui::px(-border))
            .rounded(gpui::px(radius))
            .bg(color);
        if self.reduced_motion {
            wash.into_any_element()
        } else {
            let frames = self.motion_frame_counter();
            wash.with_animation(
                "hover-wash",
                Animation::new(CONTROL).with_easing(gpui::ease_out_quint()),
                move |element, progress| {
                    count_frame(&frames);
                    element.opacity(progress)
                },
            )
            .into_any_element()
        }
    }

    /// Only mounted for visible typing activity; reduced motion keeps static dots.
    pub(super) fn typing_dots(
        &self,
        context: talk_core::ui::presentation::ConversationRef,
    ) -> AnyElement {
        let dots = (0..3).map(|index| {
            let dot = div().size(gpui::px(3.)).rounded_full().bg(self.theme.muted);
            if self.reduced_motion {
                dot.into_any_element()
            } else {
                dot.with_animation(
                    super::controls::scoped_id("typing-dot", context, index as u128, 0),
                    Animation::new(Duration::from_millis(1200)).repeat(),
                    move |dot, phase| {
                        let offset = (phase + index as f32 / 3.).fract();
                        let pulse = (1. - (offset * 2. - 1.).abs()).max(0.);
                        dot.opacity(0.35 + pulse * 0.65)
                    },
                )
                .into_any_element()
            }
        });
        div()
            .flex_none()
            .flex()
            .items_center()
            .gap(gpui::px(2.))
            .children(dots)
            .into_any_element()
    }

    pub(super) fn switch_thumb(
        &self,
        id: &ElementId,
        value: Option<bool>,
        thumb: Div,
        off: gpui::Pixels,
        on: gpui::Pixels,
    ) -> AnyElement {
        let prior = self.motion_trigger.as_ref().and_then(|motion| {
            (!self.reduced_motion)
                .then(|| motion.switch_from(id, value, Instant::now()))
                .flatten()
        });
        let Some(prior) = prior else {
            return thumb.into_any_element();
        };
        let (from, to) = if prior { (on, off) } else { (off, on) };
        let frames = self.motion_frame_counter();
        thumb
            .with_animation(
                ("switch-thumb", usize::from(!prior)),
                Animation::new(CONTROL).with_easing(gpui::ease_out_quint()),
                move |element, progress| {
                    count_frame(&frames);
                    element.left(from + (to - from) * progress)
                },
            )
            .into_any_element()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn thread_entrance_uses_the_sideways_panel_motion() {
        assert!(matches!(
            dialog_panel_kind(DialogSurface::Overlay(Overlay::Thread)),
            reveal::Kind::ThreadPanel
        ));
        assert!(matches!(
            dialog_panel_kind(DialogSurface::Prejoin),
            reveal::Kind::Panel
        ));
    }

    #[test]
    fn dialog_redraws_preserve_the_original_admission() {
        let mut motion = DialogMotion::default();
        let surface = DialogSurface::Overlay(Overlay::Settings);
        let now = Instant::now();
        motion.prepare(Some(surface), false, now);
        motion.prepare(Some(surface), false, now + Duration::from_millis(80));
        assert_eq!(motion.admission(surface), Some(now));
        motion.prepare(Some(surface), false, now + DIALOG);
        assert_eq!(motion.admission(surface), Some(now));
    }

    #[test]
    fn nested_dialog_return_is_static_and_a_new_session_can_animate_again() {
        let mut motion = DialogMotion::default();
        let parent = DialogSurface::Overlay(Overlay::CallChat);
        let child = DialogSurface::Overlay(Overlay::Forward);
        let confirmation = DialogSurface::AccountRemoval;
        let now = Instant::now();
        motion.prepare(Some(parent), false, now);
        motion.prepare(Some(child), false, now + Duration::from_millis(20));
        assert!(motion.admission(parent).is_none());
        assert!(motion.admission(child).is_some());
        motion.prepare(Some(parent), false, now + Duration::from_millis(40));
        assert!(motion.admission(parent).is_none());
        motion.prepare(Some(confirmation), false, now + Duration::from_millis(60));
        motion.prepare(Some(parent), false, now + Duration::from_millis(80));
        assert!(motion.admission(parent).is_none());
        motion.prepare(None, false, now + Duration::from_millis(90));
        assert!(motion.seen.is_empty());
        motion.prepare(Some(parent), false, now + Duration::from_millis(100));
        assert_eq!(
            motion.admission(parent),
            Some(now + Duration::from_millis(100))
        );
    }

    #[test]
    fn reduced_motion_cancels_without_retroactive_replay() {
        let mut motion = DialogMotion::default();
        let surface = DialogSurface::Prejoin;
        let now = Instant::now();
        motion.prepare(Some(surface), true, now);
        motion.prepare(Some(surface), false, now + Duration::from_millis(20));
        assert!(motion.admission(surface).is_none());
        motion.prepare(None, false, now);
        motion.prepare(Some(surface), false, now);
        assert!(motion.admission(surface).is_some());
        motion.prepare(Some(surface), true, now + Duration::from_millis(20));
        motion.prepare(Some(surface), false, now + Duration::from_millis(40));
        assert!(motion.admission(surface).is_none());
    }

    #[test]
    fn switch_motion_requires_the_current_deliberate_value_change() {
        let id = ElementId::from("switch");
        let admitted = Instant::now();
        let motion = MotionTrigger {
            id: id.clone(),
            kind: TriggerKind::Switch(false),
            admitted,
        };
        assert_eq!(motion.switch_from(&id, Some(false), admitted), None);
        assert_eq!(motion.switch_from(&id, None, admitted), None);
        assert_eq!(
            motion.switch_from(&ElementId::from("other"), Some(true), admitted),
            None
        );
        assert_eq!(motion.switch_from(&id, Some(true), admitted), Some(false));
        assert_eq!(
            motion.switch_from(&id, Some(true), admitted + Duration::from_millis(500)),
            None
        );
    }

    #[test]
    fn a_menu_remount_cannot_restart_its_expired_user_reveal() {
        let id = ElementId::from("menu");
        let admitted = Instant::now();
        let motion = MotionTrigger {
            id: id.clone(),
            kind: TriggerKind::Reveal,
            admitted,
        };
        assert_eq!(
            motion.reveal_elapsed(&id, admitted + Duration::from_millis(80)),
            Some(Duration::from_millis(80))
        );
        assert_eq!(motion.reveal_elapsed(&id, admitted + REVEAL), None);
        assert_eq!(
            motion.reveal_elapsed(&ElementId::from("other"), admitted),
            None
        );
    }

    #[test]
    fn unread_pop_requires_an_increase_in_the_same_ready_account() {
        let previous = crate::presentation::state::tests::draft_snapshot();
        let mut next = previous.clone();
        let mut row = next.conversations.as_slice()[0].clone();
        row.unread = 1;
        let mut rows = talk_core::ui::Page::empty();
        rows.try_push(row.clone()).unwrap();
        next.conversations = rows;
        assert_eq!(
            unread_update(&previous, &next, false),
            UnreadUpdate::Admit(row.context)
        );
        assert_eq!(unread_update(&previous, &next, true), UnreadUpdate::Retire);
        assert_eq!(unread_update(&next, &next, false), UnreadUpdate::Keep);
        next.active_account = None;
        assert_eq!(unread_update(&previous, &next, false), UnreadUpdate::Retire);
        next.active_account = previous.active_account;
        next.conversation_list.content = talk_core::ui::presentation::ContentState::Offline;
        assert_eq!(unread_update(&previous, &next, false), UnreadUpdate::Retire);
    }

    #[test]
    fn action_pulse_is_scoped_and_has_a_finite_deadline() {
        let now = Instant::now();
        let id = ElementId::from("send");
        let pulse = MotionTrigger {
            id: id.clone(),
            kind: TriggerKind::Pulse,
            admitted: now,
        };
        assert_eq!(pulse.pulse_elapsed(&id, now), Some(Duration::ZERO));
        assert_eq!(pulse.pulse_elapsed(&id, now + CONTROL), None);
        assert_eq!(pulse.pulse_elapsed(&ElementId::from("another"), now), None);
        assert_eq!(pulse.reveal_elapsed(&id, now), None);
    }

    #[test]
    fn hover_reveal_does_not_restart_or_let_an_old_leave_cancel_a_new_row() {
        let now = Instant::now();
        let first = ElementId::from("first");
        let second = ElementId::from("second");
        let mut hover = HoverMotion::default();
        assert!(hover.update(&first, true, false, now));
        assert!(!hover.update(&first, true, false, now + CONTROL));
        assert_eq!(hover.admission(&first, now + CONTROL), Some(now));
        assert_eq!(hover.admission(&first, now + REVEAL), None);
        assert!(hover.update(&second, true, false, now + CONTROL));
        assert!(!hover.update(&first, false, false, now + CONTROL));
        assert_eq!(hover.admission(&second, now + CONTROL), Some(now + CONTROL));
        assert!(hover.visible(&second) && !hover.visible(&first));
    }

    #[test]
    fn the_toolbar_overhang_keeps_its_row_visible_in_either_event_order() {
        let now = Instant::now();
        let above = ElementId::from("above");
        let row = ElementId::from("row");
        let mut hover = HoverMotion::default();
        // Pointer climbs from the row body into its own toolbar: the row's
        // leave arrives while the toolbar still holds the toolbar open.
        assert!(hover.update(&row, true, false, now));
        assert!(hover.update_toolbar(&row, true, false, now));
        hover.update(&row, false, false, now);
        assert!(hover.visible(&row));
        assert!(hover.update_toolbar(&row, false, false, now));
        assert!(!hover.visible(&row));
        // Pointer crosses from the row above into this row's toolbar, which
        // overlaps it. Neither arrival order may strand or steal visibility.
        assert!(hover.update(&above, true, false, now));
        assert!(hover.update_toolbar(&row, true, false, now));
        assert!(!hover.update(&above, false, false, now));
        assert!(hover.visible(&row) && !hover.visible(&above));
        assert!(hover.update_toolbar(&row, false, false, now));
        assert!(!hover.visible(&row));
        assert!(hover.update(&above, true, false, now));
        assert!(hover.visible(&above));
    }

    #[test]
    fn reduced_motion_keeps_toolbar_ownership_but_never_admits_a_reveal() {
        let now = Instant::now();
        let row = ElementId::from("row");
        let mut hover = HoverMotion::default();
        assert!(hover.update(&row, true, true, now));
        assert!(hover.visible(&row));
        assert_eq!(hover.admission(&row, now), None);
        assert!(hover.update(&row, false, true, now));
        assert!(!hover.visible(&row));
        hover.update(&row, true, false, now);
        hover.clear();
        assert!(!hover.visible(&row) && hover.admission(&row, now).is_none());
    }
}
