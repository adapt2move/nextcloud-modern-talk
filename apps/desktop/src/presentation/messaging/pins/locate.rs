//! Deliberate locate: measure a bounded loaded page, then move from its actual
//! painted viewport. The bottom sentinel and unmeasured heights are not pixels.
use super::*;
use crate::presentation::messaging::pins::motion::{ScrollMotion, ScrollPhase};

const MAX_ROWS: usize = 100;

#[derive(Clone, Copy, Debug, PartialEq)]
struct Plan {
    from: gpui::Pixels,
    goal: gpui::Pixels,
}

fn plan(
    viewport_height: gpui::Pixels,
    content_height: gpui::Pixels,
    anchor_top: gpui::Pixels,
    anchor_offset: gpui::Pixels,
    target_top: gpui::Pixels,
    target_offset: gpui::Pixels,
) -> Option<Plan> {
    if ![
        viewport_height,
        content_height,
        anchor_top,
        anchor_offset,
        target_top,
        target_offset,
    ]
    .iter()
    .all(|value| f32::from(*value).is_finite())
        || viewport_height <= px(0.)
        || content_height < px(0.)
    {
        return None;
    }
    let maximum = (content_height - viewport_height).max(px(0.));
    Some(Plan {
        from: (anchor_top + anchor_offset).clamp(px(0.), maximum),
        goal: (target_top + target_offset).clamp(px(0.), maximum),
    })
}

impl ScrollMotion {
    fn measured(&mut self, measured: Option<Plan>, now: Instant) -> bool {
        if now.duration_since(self.admitted) >= motion::DEADLINE {
            return false;
        }
        match (measured, &mut self.phase) {
            (Some(plan), ScrollPhase::Measuring) => {
                self.phase = ScrollPhase::Animating {
                    started: now,
                    from: plan.from,
                    goal: plan.goal,
                };
            }
            (Some(plan), ScrollPhase::Animating { goal, .. }) => *goal = plan.goal,
            (None, _) => {
                // A width/content invalidation needs another completed measure,
                // within the original deadline. Unknown is never a zero distance.
                self.phase = ScrollPhase::Measuring;
                self.measure_requested = false;
            }
        }
        true
    }
    fn step(&self, now: Instant) -> Option<(gpui::Pixels, bool)> {
        let ScrollPhase::Animating {
            started,
            from,
            goal,
        } = self.phase
        else {
            return None;
        };
        if (goal - from).abs() <= px(1.) {
            return Some((goal, true));
        }
        let duration = motion::SCROLL
            .min(motion::DEADLINE.saturating_sub(started.duration_since(self.admitted)));
        let progress =
            (now.duration_since(started).as_secs_f32() / duration.as_secs_f32().max(0.001)).min(1.);
        Some((
            from + (goal - from) * gpui::ease_out_quint()(progress),
            progress >= 1.,
        ))
    }
}

impl Desktop {
    fn pin_locate_target(&self, context: ConversationRef, source: u128) -> Option<ListOffset> {
        if self.snapshot.selected != Some(context) {
            return None;
        }
        let index = self
            .snapshot
            .messages
            .as_slice()
            .iter()
            .position(|message| message.local_id == source && !message.metadata.deleted)?;
        let index = self.media_span(source).map_or(index, |span| span.start);
        Some(ListOffset {
            item_ix: index,
            offset_in_item: if index == 0 {
                px(0.)
            } else {
                -self.message_ui.geometry.borrow().header_inset
            },
        })
    }

    pub(in crate::presentation::messaging) fn locate_message_smooth(
        &mut self,
        context: ConversationRef,
        source: u128,
    ) -> bool {
        let Some(target) = self.pin_locate_target(context, source) else {
            return false;
        };
        self.message_ui.arrival.borrow_mut().manual_scroll();
        self.message_ui.pins.focus = None;
        let mut owner = self.message_ui.pins.motion.borrow_mut();
        let generation = owner.next();
        if let Some(generation) = generation
            .filter(|_| !self.reduced_motion && self.message_ui.list.item_count() <= MAX_ROWS)
        {
            // Leave the displayed anchor in place. The next list prepaint will
            // measure all <=100 loaded rows before the after-layout hook starts
            // the clock. In particular, Context completion must not treat its
            // freshly invalidated row heights as a zero-distance target.
            owner.active = Some(Activity::Scroll(ScrollMotion {
                generation,
                context,
                source,
                admitted: Instant::now(),
                measure_requested: false,
                phase: ScrollPhase::Measuring,
            }));
        } else {
            self.message_ui.list.scroll_to(target);
        }
        self.highlighted_message = self
            .snapshot
            .messages
            .as_slice()
            .get(target.item_ix)
            .map(|message| (context, message.local_id));
        true
    }

    pub(super) fn advance_pin_locate(&self, now: Instant) {
        let target = {
            let owner = self.message_ui.pins.motion.borrow();
            let Some(Activity::Scroll(scroll)) = owner.active.as_ref() else {
                return;
            };
            self.pin_locate_target(scroll.context, scroll.source)
        };
        self.message_ui.arrival.borrow_mut().manual_scroll();
        let mut owner = self.message_ui.pins.motion.borrow_mut();
        let Some(Activity::Scroll(scroll)) = owner.active.as_mut() else {
            return;
        };
        let Some(target) = target else {
            owner.clear();
            return;
        };
        let list = &self.message_ui.list;
        if now.duration_since(scroll.admitted) >= motion::DEADLINE {
            // An unavailable/zero-sized layout cannot keep work pending. Settle
            // to the current typed target with the ordinary static API.
            list.scroll_to(target);
            owner.cancel();
            return;
        }
        if matches!(scroll.phase, ScrollPhase::Measuring) && !scroll.measure_requested {
            if list.item_count() > MAX_ROWS {
                list.scroll_to(target);
                owner.cancel();
                return;
            }
            let _ = list.clone().measure_all();
            scroll.measure_requested = true;
        }
        if let Some((desired, finished)) = scroll.step(now) {
            if finished {
                list.scroll_to(target);
                owner.cancel();
            } else {
                // after-layout normalized this to an actual painted row, never
                // the bottom sentinel's content-end coordinate. GPUI may clamp
                // during layout; its correction is not an explicit user scroll.
                let current = -list.scroll_px_offset_for_scrollbar().y;
                list.scroll_by(desired - current);
            }
        }
    }

    /// Called once after the production list releases its prepaint borrow.
    /// All reads use GPUI 0.2.2's public ListState methods. Temporary offset
    /// probes are restored to the same actual viewport before this frame paints.
    pub(in crate::presentation::messaging) fn pin_locate_layout(
        &self,
    ) -> impl Fn(Option<(usize, u128, gpui::Pixels)>, &mut Window) + use<> {
        let owner = self.message_ui.pins.motion.clone();
        let list = self.message_ui.list.clone();
        let current = {
            let owner = owner.borrow();
            if let Some(Activity::Scroll(scroll)) = owner.active.as_ref() {
                self.pin_locate_target(scroll.context, scroll.source)
                    .map(|target| (scroll.generation, target))
            } else {
                None
            }
        };
        move |anchor, window| {
            let Some((generation, target)) = current else {
                return;
            };
            let mut owner = owner.borrow_mut();
            let Some(Activity::Scroll(scroll)) = owner.active.as_mut() else {
                return;
            };
            if scroll.generation != generation {
                return;
            }
            let now = Instant::now();
            let Some((index, _, offset)) = anchor else {
                scroll.measured(None, now);
                return;
            };
            let anchor = ListOffset {
                item_ix: index,
                offset_in_item: offset,
            };
            let viewport = list.viewport_bounds();
            let count = list.item_count();
            let measured =
                if count > 0 && count <= MAX_ROWS && index < count && target.item_ix < count {
                    list.scroll_to(ListOffset {
                        item_ix: 0,
                        offset_in_item: px(0.),
                    });
                    // bounds_for_item(None) is an unknown height, including when a
                    // later page or width change invalidated only part of the list.
                    let all_measured =
                        (0..count).all(|index| list.bounds_for_item(index).is_some());
                    let measured = if all_measured {
                        list.bounds_for_item(count - 1)
                            .zip(list.bounds_for_item(index))
                            .zip(list.bounds_for_item(target.item_ix))
                            .and_then(|((last, anchor), target_bounds)| {
                                plan(
                                    viewport.size.height,
                                    last.bottom() - viewport.top(),
                                    anchor.top() - viewport.top(),
                                    offset,
                                    target_bounds.top() - viewport.top(),
                                    target.offset_in_item,
                                )
                            })
                    } else {
                        None
                    };
                    list.scroll_to(anchor);
                    measured
                } else {
                    None
                };
            if !scroll.measured(measured, now) {
                list.scroll_to(target);
                owner.cancel();
            }
            if owner.active.is_some() {
                window.request_animation_frame();
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use talk_core::ui::{AccountId, AccountScope, ConversationId, HostId, RuntimeGeneration};

    fn pending(now: Instant) -> ScrollMotion {
        let account = AccountId::from_local(1);
        ScrollMotion {
            generation: 1,
            context: ConversationRef {
                account: AccountScope {
                    account,
                    runtime: RuntimeGeneration::from_local(1),
                },
                conversation: ConversationId {
                    account,
                    host: HostId::from_local(1),
                    local: 2,
                },
            },
            source: 7,
            admitted: now,
            measure_requested: true,
            phase: ScrollPhase::Measuring,
        }
    }

    #[test]
    fn bottom_sentinel_locate_starts_at_painted_viewport_not_content_end() {
        let now = Instant::now();
        // A 1000px page in a 300px viewport paints row 6 with 100px
        // consumed. The GPUI logical bottom sentinel instead names 1000px.
        let geometry = plan(px(300.), px(1000.), px(600.), px(100.), px(200.), px(-44.)).unwrap();
        assert_eq!(
            geometry,
            Plan {
                from: px(700.),
                goal: px(156.)
            }
        );
        let mut scroll = pending(now);
        assert!(scroll.measured(Some(geometry), now));
        let (first, finished) = scroll.step(now + Duration::from_millis(16)).unwrap();
        assert!(!finished);
        assert!(first < px(700.) && first > px(156.));
        // GPUI can produce another bottom sentinel/corrected anchor during
        // layout. Reconciling painted geometry preserves the original clock.
        assert!(scroll.measured(Some(geometry), now + Duration::from_millis(17)));
        let (next, finished) = scroll.step(now + Duration::from_millis(32)).unwrap();
        assert!(!finished);
        assert!(next < first);
        assert_eq!(scroll.step(now + motion::SCROLL), Some((px(156.), true)));
    }
    #[test]
    fn context_installation_unknown_heights_wait_for_measurement_then_animate() {
        let now = Instant::now();
        let mut scroll = pending(now);
        assert!(scroll.measured(None, now + Duration::from_millis(16)));
        assert!(scroll.step(now + Duration::from_millis(16)).is_none());
        assert!(!scroll.measure_requested);
        // After the production list's measure_all/prepaint, a real distance
        // becomes available. Its animation clock starts here, not on fetch.
        let measured_at = now + Duration::from_millis(48);
        let geometry = plan(px(300.), px(1200.), px(800.), px(100.), px(300.), px(-44.)).unwrap();
        assert!(scroll.measured(Some(geometry), measured_at));
        assert_eq!(scroll.step(measured_at), Some((px(900.), false)));
        let (position, finished) = scroll
            .step(measured_at + Duration::from_millis(16))
            .unwrap();
        assert!(!finished);
        assert!(position < px(900.) && position > px(256.));
    }
    #[test]
    fn zero_distance_is_known_and_settles_without_a_decorative_clock() {
        let now = Instant::now();
        let mut scroll = pending(now);
        assert!(scroll.measured(None, now));
        assert!(scroll.step(now).is_none());
        assert!(scroll.measured(
            Some(Plan {
                from: px(0.),
                goal: px(0.)
            }),
            now
        ));
        assert_eq!(scroll.step(now), Some((px(0.), true)));
    }
    #[test]
    fn invalidation_during_scroll_remeasures_within_original_deadline() {
        let now = Instant::now();
        let mut scroll = pending(now);
        assert!(scroll.measured(
            Some(Plan {
                from: px(500.),
                goal: px(0.)
            }),
            now
        ));
        assert!(scroll.measured(None, now + Duration::from_millis(32)));
        assert!(scroll.step(now + Duration::from_millis(33)).is_none());
        let measured_at = now + motion::DEADLINE - Duration::from_millis(40);
        assert!(scroll.measured(
            Some(Plan {
                from: px(400.),
                goal: px(0.)
            }),
            measured_at
        ));
        assert_eq!(scroll.admitted, now);
        assert_eq!(scroll.step(now + motion::DEADLINE), Some((px(0.), true)));
        assert!(!scroll.measured(None, now + motion::DEADLINE));
    }
    #[test]
    fn measured_plan_clamps_to_valid_top_and_tail_in_both_directions() {
        assert_eq!(
            plan(px(300.), px(1000.), px(0.), px(0.), px(950.), px(-44.)),
            Some(Plan {
                from: px(0.),
                goal: px(700.)
            })
        );
        assert_eq!(
            plan(px(300.), px(1000.), px(600.), px(100.), px(0.), px(-44.)),
            Some(Plan {
                from: px(700.),
                goal: px(0.)
            })
        );
        assert!(plan(px(0.), px(1000.), px(600.), px(100.), px(0.), px(0.)).is_none());
        assert!(plan(px(300.), px(1000.), px(f32::NAN), px(0.), px(0.), px(0.)).is_none());
    }
    #[test]
    fn explicit_scroll_cancels_pending_measurement_and_running_locate() {
        let now = Instant::now();
        for measured in [
            None,
            Some(Plan {
                from: px(700.),
                goal: px(0.),
            }),
        ] {
            let mut scroll = pending(now);
            scroll.measured(measured, now);
            let mut owner = Motion::default();
            owner.active = Some(Activity::Scroll(scroll));
            owner.clear();
            assert!(owner.active.is_none());
            // A queued after-layout callback only reads the owner slot, so it
            // cannot turn an expired/cancelled measurement back into a scroll.
            assert!(!matches!(owner.active, Some(Activity::Scroll(_))));
        }
    }
}
