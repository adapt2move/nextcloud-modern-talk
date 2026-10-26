//! One window-owned decoration or locate operation, with no executor task.
//! Geometry is a bounded record of the current paint, never a scroll-position cache.
use super::*;

pub(super) const DEADLINE: Duration = Duration::from_millis(450);
pub(super) const FLIGHT: Duration = Duration::from_millis(300);
pub(super) const SHRED: Duration = Duration::from_millis(220);
pub(super) const SCROLL: Duration = Duration::from_millis(280);
const SOURCES: usize = 100;
pub(super) const EXCERPT_BYTES: usize = 240;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum Kind {
    Pin,
    Unpin,
}
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) struct Ticket {
    pub generation: u64,
    pub request: RequestId,
    pub context: ConversationRef,
    pub source: u128,
    pub fingerprint: u64,
}
pub(super) struct CopyMotion {
    pub ticket: Ticket,
    pub kind: Kind,
    pub head: u128,
    pub excerpt: String,
    pub admitted: Instant,
    pub observed: bool,
    pub started: Option<Instant>,
    pub from: Option<gpui::Bounds<gpui::Pixels>>,
    pub to: Option<gpui::Bounds<gpui::Pixels>>,
    pub error_baseline: Option<u64>,
}
pub(super) struct ScrollMotion {
    pub generation: u64,
    pub context: ConversationRef,
    pub source: u128,
    pub admitted: Instant,
    pub measure_requested: bool,
    pub phase: ScrollPhase,
}
pub(super) enum ScrollPhase {
    Measuring,
    Animating {
        started: Instant,
        from: gpui::Pixels,
        goal: gpui::Pixels,
    },
}
pub(super) enum Activity {
    Copy(CopyMotion),
    Scroll(ScrollMotion),
    Cycle { started: Instant, forward: bool },
}
#[derive(Default)]
pub(super) struct Motion {
    generation: u64,
    pub active: Option<Activity>,
    pub frame: u64,
    pub header: Option<(u128, gpui::Bounds<gpui::Pixels>)>,
    pub sources: Vec<(u128, gpui::Bounds<gpui::Pixels>)>,
}
impl Motion {
    pub fn cancel(&mut self) {
        self.active = None;
    }
    pub fn clear(&mut self) {
        self.cancel();
        self.header = None;
        self.sources.clear();
    }
    pub fn next(&mut self) -> Option<u64> {
        self.cancel();
        self.generation = self.generation.checked_add(1)?;
        Some(self.generation)
    }
    pub fn begin_frame(&mut self) -> u64 {
        self.header = None;
        self.sources.clear();
        self.frame = self.frame.saturating_add(1);
        self.frame
    }
    pub fn source(&mut self, frame: u64, id: u128, bounds: gpui::Bounds<gpui::Pixels>) {
        if frame == self.frame && self.sources.len() < SOURCES && usable(bounds) {
            self.sources.push((id, bounds));
        }
    }
    pub fn expire(&mut self, now: Instant) {
        let expired = match self.active.as_ref() {
            Some(Activity::Copy(copy)) => {
                now.duration_since(copy.admitted) >= DEADLINE
                    || copy.started.is_some_and(|at| {
                        now.duration_since(at)
                            >= match copy.kind {
                                Kind::Pin => FLIGHT,
                                Kind::Unpin => SHRED,
                            }
                    })
            }
            Some(Activity::Scroll(scroll)) => now.duration_since(scroll.admitted) >= DEADLINE,
            Some(Activity::Cycle { started, .. }) => now.duration_since(*started) >= CYCLE,
            None => false,
        };
        if expired {
            self.cancel();
        }
    }
    // Only a matching live request's *changed* snapshot can start decoration.
    pub fn observe(&mut self, ticket: Ticket, pinned: bool, rejected: bool) -> bool {
        let Some(Activity::Copy(copy)) = self.active.as_mut() else {
            return false;
        };
        if copy.ticket != ticket {
            return false;
        }
        if rejected || copy.observed && pinned != (copy.kind == Kind::Pin) {
            self.cancel();
            return false;
        }
        if pinned == (copy.kind == Kind::Pin) {
            copy.observed = true;
        }
        copy.observed
    }
    pub fn retain_source(
        &mut self,
        context: Option<ConversationRef>,
        fingerprint: Option<u64>,
    ) -> bool {
        let current = match self.active.as_ref() {
            Some(Activity::Copy(copy)) => {
                context == Some(copy.ticket.context) && fingerprint == Some(copy.ticket.fingerprint)
            }
            _ => true,
        };
        if !current {
            self.clear();
        }
        current
    }
    pub fn retain_pin_authority(
        &mut self,
        change: Decision,
        feature: talk_core::ui::conversation::Availability,
    ) -> bool {
        let current = change == Decision::Allowed && feature.enabled();
        if !current && matches!(self.active, Some(Activity::Copy(_))) {
            self.clear();
        }
        current
    }
    pub fn layout_copy(&mut self, viewport: gpui::Bounds<gpui::Pixels>, now: Instant) {
        self.expire(now);
        let Some(Activity::Copy(copy)) = self.active.as_mut() else {
            return;
        };
        if !copy.observed {
            return;
        }
        let header = self
            .header
            .filter(|(id, _)| *id == copy.ticket.source)
            .map(|(_, b)| b);
        match copy.kind {
            Kind::Pin => {
                let source = self
                    .sources
                    .iter()
                    .find(|(id, _)| *id == copy.head)
                    .map(|(_, b)| *b)
                    .and_then(|b| visible_source(b, viewport));
                let Some((source, header)) = source.zip(header) else {
                    self.cancel();
                    return;
                };
                if copy.started.is_some() {
                    // A header reflow or a source leaving the current paint
                    // retires geometry instead of flying toward a stale target.
                    if copy.to != Some(header) {
                        self.cancel();
                    }
                    return;
                }
                copy.from = Some(source);
                copy.to = Some(header);
            }
            Kind::Unpin => {
                if copy.from.is_none() {
                    self.cancel();
                    return;
                }
            }
        }
        if copy.started.is_none() {
            copy.started = Some(now);
        }
    }
}
pub(super) fn excerpt(text: &str) -> String {
    let mut end = text.len().min(EXCERPT_BYTES);
    while !text.is_char_boundary(end) {
        end -= 1;
    }
    text[..end].to_owned()
}
pub(super) fn usable(b: gpui::Bounds<gpui::Pixels>) -> bool {
    [b.origin.x, b.origin.y, b.size.width, b.size.height]
        .iter()
        .all(|v| f32::from(*v).is_finite())
        && b.size.width > px(0.)
        && b.size.height > px(0.)
}
pub(super) fn visible_source(
    b: gpui::Bounds<gpui::Pixels>,
    viewport: gpui::Bounds<gpui::Pixels>,
) -> Option<gpui::Bounds<gpui::Pixels>> {
    if !usable(b) || !usable(viewport) {
        return None;
    }
    let left = b.left().max(viewport.left());
    let top = b.top().max(viewport.top());
    let right = b.right().min(viewport.right());
    let bottom = b.bottom().min(viewport.bottom());
    (right > left && bottom > top)
        .then(|| gpui::Bounds::from_corners(gpui::point(left, top), gpui::point(right, bottom)))
}
pub(super) fn interpolate(
    a: gpui::Bounds<gpui::Pixels>,
    b: gpui::Bounds<gpui::Pixels>,
    progress: f32,
) -> gpui::Bounds<gpui::Pixels> {
    let p = gpui::ease_out_quint()(progress.clamp(0., 1.));
    gpui::Bounds {
        origin: gpui::point(
            a.origin.x + (b.origin.x - a.origin.x) * p,
            a.origin.y + (b.origin.y - a.origin.y) * p,
        ),
        size: gpui::size(
            a.size.width + (b.size.width - a.size.width) * p,
            a.size.height + (b.size.height - a.size.height) * p,
        ),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use talk_core::ui::{AccountId, AccountScope, ConversationId, HostId, RuntimeGeneration};
    fn context(room: u128) -> ConversationRef {
        let account = AccountId::from_local(1);
        ConversationRef {
            account: AccountScope {
                account,
                runtime: RuntimeGeneration::from_local(1),
            },
            conversation: ConversationId {
                account,
                host: HostId::from_local(1),
                local: room,
            },
        }
    }
    fn bounds(x: f32, y: f32, w: f32, h: f32) -> gpui::Bounds<gpui::Pixels> {
        gpui::Bounds {
            origin: gpui::point(px(x), px(y)),
            size: gpui::size(px(w), px(h)),
        }
    }
    fn admitted(owner: &mut Motion, now: Instant, kind: Kind) -> Ticket {
        let ticket = Ticket {
            generation: owner.next().expect("finite generation"),
            request: RequestId(12),
            context: context(1),
            source: 7,
            fingerprint: 23,
        };
        owner.active = Some(Activity::Copy(CopyMotion {
            ticket,
            kind,
            head: 7,
            excerpt: excerpt("private synthetic excerpt"),
            admitted: now,
            observed: false,
            started: None,
            from: None,
            to: None,
            error_baseline: None,
        }));
        ticket
    }
    fn copy(owner: &Motion) -> &CopyMotion {
        match owner.active.as_ref() {
            Some(Activity::Copy(c)) => c,
            _ => panic!("expected copy"),
        }
    }

    #[test]
    fn restored_or_background_pin_snapshot_cannot_admit_motion() {
        let mut owner = Motion::default();
        let now = Instant::now();
        let ticket = Ticket {
            generation: 1,
            request: RequestId(12),
            context: context(1),
            source: 7,
            fingerprint: 23,
        };
        assert!(!owner.observe(ticket, true, false));
        assert!(owner.active.is_none());
        let ticket = admitted(&mut owner, now, Kind::Pin);
        assert!(!owner.observe(ticket, false, false));
        assert!(!copy(&owner).observed);
        assert!(owner.observe(ticket, true, false));
    }
    #[test]
    fn replacement_has_one_slot_and_rejects_every_obsolete_identity_component() {
        let now = Instant::now();
        let mut owner = Motion::default();
        let old = admitted(&mut owner, now, Kind::Pin);
        let current = admitted(&mut owner, now, Kind::Pin);
        assert!(current.generation > old.generation);
        for stale in [
            old,
            Ticket {
                request: RequestId(13),
                ..current
            },
            Ticket {
                context: context(2),
                ..current
            },
            Ticket {
                source: 8,
                ..current
            },
            Ticket {
                fingerprint: 24,
                ..current
            },
        ] {
            assert!(!owner.observe(stale, true, true));
            assert_eq!(copy(&owner).ticket, current);
            assert!(!copy(&owner).observed);
        }
    }
    #[test]
    fn failed_or_rolled_back_mutation_drops_its_copy_immediately() {
        let now = Instant::now();
        let mut owner = Motion::default();
        let ticket = admitted(&mut owner, now, Kind::Pin);
        assert!(!owner.observe(ticket, true, true));
        assert!(owner.active.is_none());
        let ticket = admitted(&mut owner, now, Kind::Pin);
        assert!(owner.observe(ticket, true, false));
        assert!(!owner.observe(ticket, false, false));
        assert!(owner.active.is_none());
    }
    #[test]
    fn current_visible_source_and_selected_header_define_the_flight() {
        let now = Instant::now();
        let mut owner = Motion::default();
        let ticket = admitted(&mut owner, now, Kind::Pin);
        let frame = owner.begin_frame();
        owner.source(frame, 7, bounds(30., 180., 400., 70.));
        owner.header = Some((7, bounds(30., 60., 500., 44.)));
        assert!(owner.observe(ticket, true, false));
        owner.layout_copy(bounds(0., 100., 700., 500.), now);
        assert_eq!(copy(&owner).from, Some(bounds(30., 180., 400., 70.)));
        assert_eq!(copy(&owner).to, Some(bounds(30., 60., 500., 44.)));
        assert_eq!(copy(&owner).started, Some(now));
    }
    #[test]
    fn missing_stale_occluded_or_wrong_header_geometry_settles_statically() {
        let now = Instant::now();
        for mode in 0..4 {
            let mut owner = Motion::default();
            let ticket = admitted(&mut owner, now, Kind::Pin);
            let old = owner.begin_frame();
            let frame = owner.begin_frame();
            match mode {
                0 => owner.source(old, 7, bounds(0., 200., 300., 40.)),
                1 => owner.source(frame, 7, bounds(0., -200., 300., 40.)),
                _ => owner.source(frame, 7, bounds(0., 200., 300., 40.)),
            }
            if mode != 2 {
                owner.header = Some((if mode == 3 { 8 } else { 7 }, bounds(0., 40., 300., 40.)));
            }
            assert!(owner.observe(ticket, true, false));
            owner.layout_copy(bounds(0., 100., 700., 500.), now);
            assert!(owner.active.is_none(), "mode {mode}");
        }
    }
    #[test]
    fn pending_and_running_copies_expire_without_an_autonomous_timer() {
        let now = Instant::now();
        let mut owner = Motion::default();
        admitted(&mut owner, now, Kind::Pin);
        owner.expire(now + DEADLINE - Duration::from_millis(1));
        assert!(owner.active.is_some());
        owner.expire(now + DEADLINE);
        assert!(owner.active.is_none());
        let ticket = admitted(&mut owner, now, Kind::Unpin);
        if let Some(Activity::Copy(c)) = owner.active.as_mut() {
            c.from = Some(bounds(1., 2., 30., 40.));
        }
        assert!(owner.observe(ticket, false, false));
        owner.layout_copy(bounds(0., 0., 700., 500.), now);
        owner.expire(now + SHRED);
        assert!(owner.active.is_none());
        assert!(DEADLINE <= Duration::from_millis(500));
    }
    #[test]
    fn cancellation_retires_protected_copy_and_all_geometry() {
        let now = Instant::now();
        let mut owner = Motion::default();
        admitted(&mut owner, now, Kind::Unpin);
        let frame = owner.begin_frame();
        owner.source(frame, 7, bounds(0., 2., 30., 40.));
        owner.header = Some((7, bounds(0., 0., 30., 40.)));
        // Used by room retirement, source revocation, reduced motion, focus,
        // window, wheel/key scrolling and resize. No task can reinsert a copy.
        owner.clear();
        assert!(owner.active.is_none());
        assert!(owner.header.is_none());
        assert!(owner.sources.is_empty());
    }
    #[test]
    fn geometry_capacity_and_excerpt_allocation_are_bounded_before_copying() {
        let mut owner = Motion::default();
        let frame = owner.begin_frame();
        for id in 0..150 {
            owner.source(frame, id, bounds(0., 0., 30., 40.));
        }
        assert_eq!(owner.sources.len(), SOURCES);
        let text = "🦀".repeat(1000);
        let value = excerpt(&text);
        assert!(value.len() <= EXCERPT_BYTES);
        assert!(text.starts_with(&value));
        assert!(value.is_char_boundary(value.len()));
    }
    #[test]
    fn deleted_edited_revoked_or_replaced_source_drops_excerpt_before_paint() {
        let now = Instant::now();
        for (selected, fingerprint) in [
            (Some(context(1)), None),
            (Some(context(1)), Some(24)),
            (None, Some(23)),
            (Some(context(2)), Some(23)),
        ] {
            let mut owner = Motion::default();
            admitted(&mut owner, now, Kind::Pin);
            assert!(!owner.retain_source(selected, fingerprint));
            assert!(owner.active.is_none());
        }
        let mut owner = Motion::default();
        admitted(&mut owner, now, Kind::Pin);
        assert!(owner.retain_source(Some(context(1)), Some(23)));
        assert!(owner.active.is_some());
    }
    #[test]
    fn active_flight_drops_if_the_current_header_moves_or_source_leaves_paint() {
        let now = Instant::now();
        for moved_header in [false, true] {
            let mut owner = Motion::default();
            let ticket = admitted(&mut owner, now, Kind::Pin);
            let frame = owner.begin_frame();
            owner.source(frame, 7, bounds(0., 200., 300., 40.));
            owner.header = Some((7, bounds(0., 40., 300., 40.)));
            assert!(owner.observe(ticket, true, false));
            owner.layout_copy(bounds(0., 100., 700., 500.), now);
            let frame = owner.begin_frame();
            if moved_header {
                owner.source(frame, 7, bounds(0., 200., 300., 40.));
            }
            owner.header = Some((
                7,
                bounds(0., if moved_header { 60. } else { 40. }, 300., 40.),
            ));
            owner.layout_copy(
                bounds(0., 100., 700., 500.),
                now + Duration::from_millis(16),
            );
            assert!(owner.active.is_none());
        }
    }
    #[test]
    fn pin_decoration_requires_current_mutation_and_feature_authority() {
        use talk_core::ui::{conversation::Availability, presentation::FeatureSupport};
        let now = Instant::now();
        let available = Availability {
            implemented: true,
            server: FeatureSupport::Supported,
            allowed: Decision::Allowed,
        };
        for (change, feature) in [
            (Decision::Denied, available),
            (Decision::Unknown, available),
            (
                Decision::Allowed,
                Availability {
                    allowed: Decision::Denied,
                    ..available
                },
            ),
            (
                Decision::Allowed,
                Availability {
                    allowed: Decision::Unknown,
                    ..available
                },
            ),
            (
                Decision::Allowed,
                Availability {
                    server: FeatureSupport::Unsupported,
                    ..available
                },
            ),
            (
                Decision::Allowed,
                Availability {
                    server: FeatureSupport::Unknown,
                    ..available
                },
            ),
            (
                Decision::Allowed,
                Availability {
                    implemented: false,
                    ..available
                },
            ),
        ] {
            let mut owner = Motion::default();
            let ticket = admitted(&mut owner, now, Kind::Pin);
            assert!(owner.observe(ticket, true, false));
            assert!(!owner.retain_pin_authority(change, feature));
            assert!(owner.active.is_none());
        }
        let mut owner = Motion::default();
        admitted(&mut owner, now, Kind::Pin);
        assert!(owner.retain_pin_authority(Decision::Allowed, available));
        assert!(owner.active.is_some());
    }
    #[test]
    fn generation_exhaustion_cannot_reuse_a_retired_ticket() {
        let mut owner = Motion {
            generation: u64::MAX,
            ..Motion::default()
        };
        assert!(owner.next().is_none());
        assert!(owner.next().is_none());
        assert!(owner.active.is_none());
    }
    #[test]
    fn geometry_uses_painted_corners_and_clips_header_occlusion() {
        let source = bounds(20., 80., 200., 100.);
        let header = bounds(30., 30., 400., 44.);
        assert_eq!(
            visible_source(source, bounds(0., 100., 700., 500.)),
            Some(bounds(20., 100., 200., 80.))
        );
        assert!(visible_source(bounds(0., 50., 200., 44.), bounds(0., 100., 700., 500.)).is_none());
        assert_eq!(interpolate(source, header, 0.), source);
        assert_eq!(interpolate(source, header, 1.), header);
        assert!(usable(bounds(0., 0., 200., 44.)));
        assert!(!usable(bounds(f32::NAN, 0., 200., 44.)));
    }
}
