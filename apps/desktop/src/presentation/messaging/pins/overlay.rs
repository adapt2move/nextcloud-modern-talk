//! Deferred paint of the finite pin owner; no state or tasks live here.
use super::*;

impl Desktop {
    /// A window-root sibling, deferred until the header and virtualized history
    /// have recorded this frame's bounds. It never participates in row layout,
    /// clipping, hit testing, selection, delivery, or mutation authority.
    pub(in crate::presentation) fn pin_motion_overlay(
        &self,
        _cx: &Context<Self>,
    ) -> Option<impl IntoElement> {
        let owner = self.message_ui.pins.motion.clone();
        if !matches!(owner.borrow().active, Some(Activity::Copy(_))) {
            return None;
        }
        let paint_owner = owner.clone();
        let list = self.message_ui.list.clone();
        let inset = self.message_ui.geometry.borrow().header_inset;
        let theme = self.theme;
        let font = self.sp(13.);
        let copy_height = self.sp(40.);
        let frames = self.motion_frame_counter();
        let canvas = gpui::canvas(
            move |_, window, cx| {
                let viewport = list.viewport_bounds();
                let visible = gpui::Bounds::from_corners(
                    gpui::point(
                        viewport.left(),
                        viewport.top() + inset.min(viewport.size.height),
                    ),
                    viewport.bottom_right(),
                );
                let now = Instant::now();
                let mut owner = owner.borrow_mut();
                owner.layout_copy(visible, now);
                let Some(Activity::Copy(copy)) = owner.active.as_ref() else {
                    return None;
                };
                let started = copy.started?;
                let from = copy.from?;
                let duration = match copy.kind {
                    Kind::Pin => motion::FLIGHT,
                    Kind::Unpin => motion::SHRED,
                }
                .min(motion::DEADLINE.saturating_sub(started.duration_since(copy.admitted)));
                let progress = (now.duration_since(started).as_secs_f32()
                    / duration.as_secs_f32().max(0.001))
                .min(1.);
                let ticket = copy.ticket;
                let excerpt = copy.excerpt.clone();
                let kind = copy.kind;
                let from = gpui::Bounds {
                    origin: from.origin,
                    size: gpui::size(
                        from.size.width.min(px(360.)),
                        from.size.height.min(copy_height),
                    ),
                };
                let rect = if kind == Kind::Pin {
                    let to = copy.to?;
                    motion::interpolate(
                        from,
                        gpui::Bounds {
                            origin: to.origin,
                            size: gpui::size(
                                to.size.width.min(px(360.)),
                                to.size.height.min(copy_height),
                            ),
                        },
                        progress,
                    )
                } else {
                    from
                };
                drop(owner);
                let mut elements = Vec::with_capacity(6);
                let card = || {
                    div()
                        .w(rect.size.width)
                        .h(rect.size.height)
                        .overflow_hidden()
                        .rounded(px(6.))
                        .bg(theme.surface)
                        .border_1()
                        .border_color(theme.border)
                        .px_3()
                        .flex()
                        .items_center()
                        .text_size(font)
                        .text_color(theme.text)
                        .child(div().min_w_0().truncate().child(excerpt.clone()))
                };
                if kind == Kind::Pin {
                    let mut element = card().opacity(0.94).into_any_element();
                    element.layout_as_root(
                        gpui::size(
                            gpui::AvailableSpace::Definite(rect.size.width),
                            gpui::AvailableSpace::Definite(rect.size.height),
                        ),
                        window,
                        cx,
                    );
                    element.prepaint_at(rect.origin, window, cx);
                    elements.push(element);
                } else {
                    // Six clipped strips retain one small excerpt for this frame;
                    // only their decorative copies disperse. The real row stays put.
                    let width = rect.size.width / 6.;
                    for strip in 0..6 {
                        let x = width * strip as f32;
                        let mut element = div()
                            .w(width)
                            .h(rect.size.height)
                            .overflow_hidden()
                            .opacity(1. - progress)
                            .child(card().relative().left(-x))
                            .into_any_element();
                        element.layout_as_root(
                            gpui::size(
                                gpui::AvailableSpace::Definite(width),
                                gpui::AvailableSpace::Definite(rect.size.height),
                            ),
                            window,
                            cx,
                        );
                        element.prepaint_at(
                            rect.origin
                                + gpui::point(
                                    x + px((strip as f32 - 2.5) * 5. * progress),
                                    rect.size.height * (0.5 + strip as f32 / 8.) * progress,
                                ),
                            window,
                            cx,
                        );
                        elements.push(element);
                    }
                }
                crate::presentation::motion::count_frame(&frames);
                Some((ticket, elements))
            },
            move |_, paint, window, cx| {
                if let Some((ticket, mut elements)) = paint {
                    let current = paint_owner.borrow().active.as_ref().is_some_and(
                        |active| matches!(active,Activity::Copy(copy) if copy.ticket == ticket),
                    );
                    if current {
                        for element in &mut elements {
                            element.paint(window, cx);
                        }
                    }
                }
            },
        )
        .absolute()
        .inset_0();
        Some(gpui::deferred(canvas).with_priority(1))
    }
}
