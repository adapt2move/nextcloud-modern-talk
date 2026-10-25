//! Geometry comes from the exact shaped runs used to paint the selectable
//! text. Each soft-wrapped fragment is clipped separately, never its union.
use super::document::{DISPLAY_BYTES, MAX_LINKS};
use super::interaction::boundary;
use gpui::{Bounds, Pixels, Point, TextLayout, point, px};
use std::ops::Range;
use talk_core::ui::markup::Link;

pub(super) type Layouts = Vec<(Range<usize>, TextLayout)>;

#[derive(Clone, Debug)]
pub(super) struct LinkRect {
    pub ordinal: u16,
    pub bounds: Bounds<Pixels>,
}

pub(super) fn clipped_rect(
    ordinal: u16,
    bounds: Bounds<Pixels>,
    clip: Bounds<Pixels>,
) -> Option<LinkRect> {
    let bounds = bounds.intersect(&clip);
    (bounds.size.width > px(0.) && bounds.size.height > px(0.))
        .then_some(LinkRect { ordinal, bounds })
}

pub(super) fn link_at(rects: &[LinkRect], position: Point<Pixels>) -> Option<u16> {
    rects
        .iter()
        .find(|rect| {
            position.x >= rect.bounds.left()
                && position.x < rect.bounds.right()
                && position.y >= rect.bounds.top()
                && position.y < rect.bounds.bottom()
        })
        .map(|rect| rect.ordinal)
}

pub(super) fn link_rects(layouts: &Layouts, links: &[Link], clip: Bounds<Pixels>) -> Vec<LinkRect> {
    let mut rects = Vec::with_capacity(links.len().min(MAX_LINKS));
    for (range, layout) in layouts {
        if !layout.bounds().intersects(&clip) {
            continue;
        }
        let Some(wrapped) = layout.line_layout_for_index(0) else {
            continue;
        };
        let unwrapped = &wrapped.unwrapped_layout;
        let breaks = wrapped
            .wrap_boundaries
            .iter()
            .filter_map(|boundary| {
                unwrapped
                    .runs
                    .get(boundary.run_ix)?
                    .glyphs
                    .get(boundary.glyph_ix)
                    .map(|glyph| glyph.index)
            })
            .chain(std::iter::once(range.len()));
        let mut start = 0;
        for (row, end) in breaks.enumerate() {
            for (ordinal, link) in links.iter().take(MAX_LINKS).enumerate() {
                let from = link.range.start.max(range.start + start);
                let to = link.range.end.min(range.start + end);
                if from >= to {
                    continue;
                }
                let origin = layout.bounds().origin;
                let base = unwrapped.x_for_index(start);
                let x1 = unwrapped.x_for_index(from - range.start) - base;
                let x2 = unwrapped.x_for_index(to - range.start) - base;
                let y = origin.y + layout.line_height() * row;
                let bounds = Bounds::from_corners(
                    point(origin.x + x1.min(x2), y),
                    point(origin.x + x1.max(x2), y + layout.line_height()),
                );
                if let Some(rect) = clipped_rect(ordinal as u16, bounds, clip) {
                    // Every fragment covers at least one display byte. This
                    // cap also guards future shaping/parser changes.
                    if rects.len() == DISPLAY_BYTES {
                        return rects;
                    }
                    rects.push(rect);
                }
            }
            start = end;
        }
    }
    rects
}

pub(super) fn mouse_index(layouts: &Layouts, position: Point<Pixels>, text: &str) -> usize {
    let Some((range, layout)) = layouts
        .iter()
        .find(|(_, layout)| position.y <= layout.bounds().bottom())
        .or_else(|| layouts.last())
    else {
        return 0;
    };
    let local = layout
        .index_for_position(position)
        .unwrap_or_else(|index| index);
    boundary(text, range.start + local.min(range.len()))
}
