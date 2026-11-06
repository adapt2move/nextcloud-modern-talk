//! Frame-local hit testing for one active voice player. Dragging previews a
//! position locally and submits one bounded seek only when the pointer is released.
use super::*;
use gpui::{
    DispatchPhase, HitboxBehavior, MouseButton, MouseDownEvent, MouseMoveEvent, MouseUpEvent,
};
use std::time::{Duration, Instant};
use talk_core::ui::content::PlaybackState;

pub(super) struct Scrub {
    pub(super) reference: AttachmentRef,
    pub(super) position_ms: u64,
    started: Instant,
}

impl Desktop {
    fn voice_scrub_current(&self, reference: &AttachmentRef) -> bool {
        !self.background_inert
            && self.snapshot.content == ContentState::Ready
            && self.snapshot.selected == Some(reference.context)
            && self
                .snapshot
                .content_panel
                .voice_playback
                .as_ref()
                .is_some_and(|playback| {
                    playback.attachment == *reference
                        && playback.duration_ms.is_some()
                        && matches!(
                            playback.state,
                            PlaybackState::Paused | PlaybackState::Playing | PlaybackState::Ended
                        )
                })
    }

    pub(super) fn retire_voice_scrub(&mut self) {
        if self.content_ui.voice_scrub.as_ref().is_some_and(|scrub| {
            scrub.started.elapsed() > Duration::from_secs(30)
                || !self.voice_scrub_current(&scrub.reference)
        }) {
            self.content_ui.voice_scrub = None;
        }
    }

    pub(super) fn voice_scrub_bar(
        &self,
        reference: AttachmentRef,
        position_ms: u64,
        duration_ms: u64,
        enabled: bool,
        cx: &Context<Self>,
    ) -> Div {
        let fraction = if duration_ms == 0 {
            0.0
        } else {
            (position_ms as f32 / duration_ms as f32).clamp(0.0, 1.0)
        };
        let owner = cx.entity().downgrade();
        let surface = gpui::canvas(
            |bounds, window, _| {
                let clip = bounds.intersect(&window.content_mask().bounds);
                (clip, window.insert_hitbox(bounds, HitboxBehavior::Normal))
            },
            move |bounds, (clip, hitbox), window, _| {
                if !enabled {
                    return;
                }
                window.set_cursor_style(gpui::CursorStyle::PointingHand, &hitbox);
                let down_owner = owner.clone();
                let down_reference = reference.clone();
                window.on_mouse_event(move |event: &MouseDownEvent, phase, window, cx| {
                    if phase != DispatchPhase::Bubble
                        || event.button != MouseButton::Left
                        || !hitbox.is_hovered(window)
                        || !clip.contains(&event.position)
                    {
                        return;
                    }
                    let admitted = down_owner
                        .update(cx, |this, cx| {
                            if !this.voice_scrub_current(&down_reference) {
                                return false;
                            }
                            this.content_ui.voice_scrub = Some(Scrub {
                                reference: down_reference.clone(),
                                position_ms: seek_position(bounds, event.position.x, duration_ms),
                                started: Instant::now(),
                            });
                            cx.notify();
                            true
                        })
                        .unwrap_or(false);
                    if admitted {
                        window.prevent_default();
                        cx.stop_propagation();
                    }
                });
                let move_owner = owner.clone();
                let move_reference = reference.clone();
                window.on_mouse_event(move |event: &MouseMoveEvent, phase, _, cx| {
                    if phase != DispatchPhase::Bubble {
                        return;
                    }
                    let _ = move_owner.update(cx, |this, cx| {
                        if !this.voice_scrub_current(&move_reference) {
                            return;
                        }
                        if let Some(scrub) = this.content_ui.voice_scrub.as_mut()
                            && scrub.reference == move_reference
                        {
                            if event.pressed_button != Some(MouseButton::Left) {
                                this.content_ui.voice_scrub = None;
                            } else {
                                scrub.position_ms =
                                    seek_position(bounds, event.position.x, duration_ms);
                            }
                            cx.notify();
                        }
                    });
                });
                window.on_mouse_event(move |event: &MouseUpEvent, phase, window, cx| {
                    if phase != DispatchPhase::Bubble || event.button != MouseButton::Left {
                        return;
                    }
                    let committed = owner
                        .update(cx, |this, cx| {
                            if !this
                                .content_ui
                                .voice_scrub
                                .as_ref()
                                .is_some_and(|scrub| scrub.reference == reference)
                            {
                                return false;
                            }
                            this.content_ui.voice_scrub = None;
                            if this.voice_scrub_current(&reference) {
                                this.submit_content(ContentAction::SeekVoice {
                                    attachment: reference.clone(),
                                    position_ms: seek_position(
                                        bounds,
                                        event.position.x,
                                        duration_ms,
                                    ),
                                });
                            }
                            cx.notify();
                            true
                        })
                        .unwrap_or(false);
                    if committed {
                        window.prevent_default();
                        cx.stop_propagation();
                    }
                });
            },
        )
        .absolute()
        .inset_0();
        div()
            .relative()
            .w_full()
            .h(px(20.))
            .flex()
            .items_center()
            .child(
                div()
                    .w_full()
                    .h(px(5.))
                    .rounded(px(3.))
                    .bg(self.theme.border)
                    .child(
                        div()
                            .h_full()
                            .w(gpui::relative(fraction))
                            .rounded(px(3.))
                            .bg(self.theme.accent),
                    ),
            )
            .child(surface)
    }
}

fn seek_position(bounds: gpui::Bounds<gpui::Pixels>, x: gpui::Pixels, duration_ms: u64) -> u64 {
    let width = f32::from(bounds.size.width);
    let offset = f32::from(x - bounds.origin.x);
    if !width.is_finite() || width <= 0. || !offset.is_finite() {
        return 0;
    }
    (duration_ms as f64 * f64::from((offset / width).clamp(0., 1.))).round() as u64
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn pointer_seek_is_clamped_to_the_actual_track() {
        let bounds = gpui::Bounds {
            origin: gpui::point(px(10.), px(0.)),
            size: gpui::size(px(200.), px(20.)),
        };
        assert_eq!(seek_position(bounds, px(-1.), 10_000), 0);
        assert_eq!(seek_position(bounds, px(60.), 10_000), 2_500);
        assert_eq!(seek_position(bounds, px(300.), 10_000), 10_000);
        assert_eq!(seek_position(gpui::Bounds::default(), px(10.), 10_000), 0);
    }
}
