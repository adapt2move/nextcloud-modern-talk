//! Frame-local hit testing for one active voice player. Dragging previews a
//! position locally and submits one bounded seek only when the pointer is released.
use super::*;
use gpui::{
    DispatchPhase, HitboxBehavior, KeyDownEvent, MouseButton, MouseDownEvent, MouseMoveEvent,
    MouseUpEvent,
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
        file_id: &str,
        enabled: bool,
        cx: &Context<Self>,
    ) -> gpui::Stateful<Div> {
        let fraction = if duration_ms == 0 {
            0.0
        } else {
            (position_ms as f32 / duration_ms as f32).clamp(0.0, 1.0)
        };
        let id = scoped_id(
            "voice-wave",
            reference.context,
            reference.message,
            super::placeholder_key(reference.placeholder.as_str()),
        );
        let focus = self.control_focus(id.clone(), cx);
        let enabled = enabled && duration_ms > 0 && focus.is_some();
        let pointer_focus = focus.clone();
        let keyboard_reference = reference.clone();
        let bars = waveform(file_id);
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
                        if let Some(focus) = &pointer_focus {
                            focus.focus(window);
                        }
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
            .id(id)
            .relative()
            .flex_1()
            .min_w(px(90.))
            .h(px(34.))
            .flex()
            .items_center()
            .justify_between()
            .gap(px(1.))
            .when_some(focus, |d, focus| {
                d.track_focus(&focus.clone().tab_stop(enabled && !self.background_inert))
                    .child(self.field_ring(focus, 5.))
            })
            .children(bars.into_iter().enumerate().map(|(index, height)| {
                div()
                    .flex_1()
                    .min_w(px(1.))
                    .h(px(f32::from(height)))
                    .rounded_full()
                    .bg(if (index as f32 + 0.5) / 40. <= fraction {
                        self.theme.accent
                    } else {
                        self.theme.muted
                    })
            }))
            .child(surface)
            .when(enabled && !self.background_inert, |d| {
                d.on_key_down(cx.listener(move |this, event: &KeyDownEvent, window, cx| {
                    let direction = match event.keystroke.key.as_str() {
                        "left" => -1,
                        "right" => 1,
                        _ => return,
                    };
                    if event.keystroke.modifiers.modified()
                        || !this.voice_scrub_current(&keyboard_reference)
                    {
                        return;
                    }
                    if let Some(playback) = this.snapshot.content_panel.voice_playback.as_ref()
                        && playback.attachment == keyboard_reference
                        && let Some(duration) = playback.duration_ms
                    {
                        let position_ms =
                            keyboard_position(playback.position_ms, duration, direction);
                        this.submit_content(ContentAction::SeekVoice {
                            attachment: keyboard_reference.clone(),
                            position_ms,
                        });
                        window.prevent_default();
                        cx.stop_propagation();
                        cx.notify();
                    }
                }))
            })
    }
}

/// Until a bounded decoded envelope is available, each file has a stable
/// visual placeholder. No message data or random state is retained per frame.
fn waveform(file_id: &str) -> [u8; 40] {
    let mut state = file_id.bytes().fold(0xcbf29ce484222325u64, |hash, byte| {
        (hash ^ u64::from(byte)).wrapping_mul(0x100000001b3)
    });
    std::array::from_fn(|_| {
        state ^= state << 13;
        state ^= state >> 7;
        state ^= state << 17;
        5 + (state % 22) as u8
    })
}

fn keyboard_position(position: u64, duration: u64, direction: i8) -> u64 {
    if direction < 0 {
        position.saturating_sub(5_000)
    } else {
        position.saturating_add(5_000).min(duration)
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
    #[test]
    fn waveform_is_stable_and_keyboard_seeks_are_bounded() {
        assert_eq!(waveform("file-1"), waveform("file-1"));
        assert_ne!(waveform("file-1"), waveform("file-2"));
        assert_eq!(keyboard_position(2_000, 20_000, -1), 0);
        assert_eq!(keyboard_position(18_000, 20_000, 1), 20_000);
    }
}
