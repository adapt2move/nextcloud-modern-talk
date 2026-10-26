//! A stable native ancestor before, during and after an action reveal. GPUI
//! keeps pending mouse presses under that ancestor; removing an animation
//! wrapper when its timer expires would discard an in-progress click.
use super::{DIALOG, REVEAL, count_frame};
use gpui::{
    AnyElement, App, Bounds, Div, Element, ElementId, GlobalElementId, InspectorElementId,
    IntoElement, LayoutId, Pixels, Window, prelude::*,
};
use std::time::{Duration, Instant};

struct Timing {
    observed: Option<Instant>,
    admitted: Option<Instant>,
    duration: Duration,
}
impl Default for Timing {
    fn default() -> Self {
        Self {
            observed: None,
            admitted: None,
            duration: REVEAL,
        }
    }
}
impl Timing {
    fn sample(&mut self, requested: Option<Instant>, enabled: bool, now: Instant) -> Option<f32> {
        if let Some(requested) = requested
            && self.observed != Some(requested)
        {
            self.observed = Some(requested);
            self.admitted = enabled.then_some(requested);
        }
        if !enabled {
            self.admitted = None;
        }
        self.admitted.and_then(|at| {
            let elapsed = now.saturating_duration_since(at);
            (elapsed < self.duration).then(|| elapsed.as_secs_f32() / self.duration.as_secs_f32())
        })
    }
}

#[derive(Clone, Copy, Eq, PartialEq)]
pub(super) enum Kind {
    Page,
    Panel,
    Backdrop,
}

/// One timing record per mounted page or menu, retired by GPUI on unmount.
/// No task, content cache or deferred close is owned by this element.
pub(super) struct ActionReveal {
    id: ElementId,
    element: Option<Div>,
    requested: Option<Instant>,
    enabled: bool,
    kind: Kind,
    frames: Option<std::rc::Rc<std::cell::Cell<u64>>>,
}
impl ActionReveal {
    pub fn new(
        id: ElementId,
        element: Div,
        requested: Option<Instant>,
        enabled: bool,
        frames: Option<std::rc::Rc<std::cell::Cell<u64>>>,
    ) -> Self {
        Self {
            id,
            element: Some(element),
            requested,
            enabled,
            kind: Kind::Page,
            frames,
        }
    }
    pub fn kind(mut self, kind: Kind) -> Self {
        self.kind = kind;
        self
    }
}
impl IntoElement for ActionReveal {
    type Element = Self;
    fn into_element(self) -> Self::Element {
        self
    }
}
impl Element for ActionReveal {
    type RequestLayoutState = AnyElement;
    type PrepaintState = ();

    fn id(&self) -> Option<ElementId> {
        Some(self.id.clone())
    }
    fn source_location(&self) -> Option<&'static core::panic::Location<'static>> {
        None
    }
    fn request_layout(
        &mut self,
        global_id: Option<&GlobalElementId>,
        _: Option<&InspectorElementId>,
        window: &mut Window,
        cx: &mut App,
    ) -> (LayoutId, AnyElement) {
        window.with_element_state(
            global_id.expect("identified reveal element"),
            |state: Option<Timing>, window| {
                let mut timing = state.unwrap_or_default();
                timing.duration = if self.kind == Kind::Page {
                    REVEAL
                } else {
                    DIALOG
                };
                if self.kind != Kind::Page && self.requested.is_none() {
                    timing.admitted = None;
                }
                let progress = timing.sample(self.requested, self.enabled, Instant::now());
                let eased = progress.map_or(1., |value| gpui::ease_out_quint()(value));
                let child = self.element.take().expect("reveal laid out once");
                let mut child = match self.kind {
                    Kind::Page => child.opacity(0.6 + 0.4 * eased),
                    Kind::Panel => child
                        .relative()
                        .top(gpui::px(8. * (1. - eased)))
                        .opacity(0.85 + 0.15 * eased),
                    Kind::Backdrop => child.opacity(0.65 + 0.35 * eased),
                }
                .into_any_element();
                if progress.is_some() {
                    count_frame(&self.frames);
                    window.request_animation_frame();
                }
                ((child.request_layout(window, cx), child), timing)
            },
        )
    }
    fn prepaint(
        &mut self,
        _: Option<&GlobalElementId>,
        _: Option<&InspectorElementId>,
        _: Bounds<Pixels>,
        child: &mut AnyElement,
        window: &mut Window,
        cx: &mut App,
    ) {
        child.prepaint(window, cx);
    }
    fn paint(
        &mut self,
        _: Option<&GlobalElementId>,
        _: Option<&InspectorElementId>,
        _: Bounds<Pixels>,
        child: &mut AnyElement,
        _: &mut (),
        window: &mut Window,
        cx: &mut App,
    ) {
        child.paint(window, cx);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn a_page_press_can_span_the_reveal_deadline_under_the_same_native_identity() {
        let now = Instant::now();
        let element = ActionReveal::new("page".into(), gpui::div(), Some(now), true, None);
        let mut timing = Timing::default();
        let identity = element.id();
        assert!(
            timing
                .sample(Some(now), true, now + std::time::Duration::from_millis(80))
                .is_some()
        );
        // Other actions may retire the trigger while the native press is held.
        assert!(
            timing
                .sample(None, true, now + std::time::Duration::from_millis(100))
                .is_some()
        );
        assert_eq!(timing.sample(None, true, now + REVEAL), None);
        assert_eq!(element.id(), identity);
        assert_eq!(timing.admitted, Some(now));
        assert_eq!(timing.sample(Some(now), true, now + REVEAL), None);
    }
    #[test]
    fn reduced_motion_cannot_replay_an_observed_admission() {
        let now = Instant::now();
        let mut timing = Timing::default();
        assert_eq!(timing.sample(Some(now), false, now), None);
        assert_eq!(timing.sample(Some(now), true, now), None);
        let next = now + REVEAL;
        assert_eq!(timing.sample(Some(next), true, next), Some(0.));
        assert_eq!(timing.sample(None, false, next), None);
        assert_eq!(timing.sample(Some(next), true, next), None);
    }
    #[test]
    fn dialog_frames_stop_at_the_original_deadline_without_removing_the_identity() {
        let now = Instant::now();
        let element = ActionReveal::new("dialog".into(), gpui::div(), Some(now), true, None)
            .kind(Kind::Panel);
        let identity = element.id();
        let mut timing = Timing {
            duration: DIALOG,
            ..Default::default()
        };
        assert!(timing.sample(Some(now), true, now + DIALOG / 2).is_some());
        assert_eq!(timing.sample(Some(now), true, now + DIALOG), None);
        assert_eq!(timing.sample(Some(now), false, now + DIALOG), None);
        assert_eq!(element.id(), identity);
        assert_eq!(timing.sample(Some(now), true, now + DIALOG), None);
    }
}
