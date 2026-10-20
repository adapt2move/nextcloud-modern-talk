//! Finite decorative motion. Input, layout and focus always take effect immediately.
use super::Desktop;
use gpui::{Animation, AnimationExt, AnyElement, Div, ElementId, div, prelude::*};
use std::time::{Duration, Instant};

const REVEAL: Duration = Duration::from_millis(140);
const CONTROL: Duration = Duration::from_millis(110);

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
            .filter(|trigger| {
                !self.reduced_motion && trigger.reveal_elapsed(&id, Instant::now()).is_some()
            })
            .map(|trigger| trigger.admitted);
        let Some(admitted) = admitted else {
            return element.into_any_element();
        };
        // GPUI retains an animation's mount time. Its fixed frame driver is
        // independent of opacity, which uses this one monotonic admission time;
        // a mid-lifetime remount therefore cannot restart or reverse the fade.
        let frames = self.motion_frame_counter();
        element
            .with_animation(id, Animation::new(REVEAL), move |element, _| {
                count_frame(&frames);
                let progress = (admitted.elapsed().as_secs_f32() / REVEAL.as_secs_f32()).min(1.);
                element.opacity(0.6 + 0.4 * gpui::ease_out_quint()(progress))
            })
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

    /// Mounted only while hovered. Persistent selection is static, including
    /// when a virtualized row returns or a modal closes. GPUI's one-shot settles and
    /// stops requesting frames; unmounting retires it without a timer or task.
    pub(super) fn control_wash(&self, color: gpui::Hsla, radius: f32) -> AnyElement {
        let wash = div()
            .absolute()
            .inset_0()
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
