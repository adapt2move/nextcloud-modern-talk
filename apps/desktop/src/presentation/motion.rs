//! Finite decorative motion. Input, layout and focus always take effect immediately.
use super::{Desktop, Overlay};
use gpui::{Animation, AnimationExt, AnyElement, Div, ElementId, div, prelude::*};
use std::time::{Duration, Instant};
mod reveal;

const REVEAL: Duration = Duration::from_millis(140);
const CONTROL: Duration = Duration::from_millis(110);
const DIALOG: Duration = Duration::from_millis(180);
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
}

/// One recent deliberate action per window, never a per-control cache.
/// A first mount, stale response, unrelated action or return to a dialog cannot
/// animate a switch. Virtualized menu remounts cannot restart a reveal.
pub(super) struct MotionTrigger {
    pub id: ElementId,
    pub kind: TriggerKind,
    pub admitted: Instant,
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
}

impl Desktop {
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

    pub(super) fn call_speaking_bars(&self, id: ElementId, color: gpui::Hsla) -> AnyElement {
        let bars = div().flex().items_end().gap_1().h(gpui::px(12.)).children(
            [5., 10., 7.].into_iter().map(|height| {
                div()
                    .w(gpui::px(2.))
                    .h(gpui::px(height))
                    .rounded_full()
                    .bg(color)
            }),
        );
        if self.reduced_motion {
            bars.into_any_element()
        } else {
            bars.with_animation(
                id,
                Animation::new(Duration::from_millis(900)).repeat(),
                |bars, progress| bars.opacity(0.65 + 0.35 * (1. - (2. * progress - 1.).abs())),
            )
            .into_any_element()
        }
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
}
