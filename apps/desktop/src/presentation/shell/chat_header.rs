//! Same-frame header measurement with a live timeline underneath.
#[cfg(not(target_os = "windows"))]
use crate::presentation::controls::CONTENT_RADIUS;
#[cfg(not(target_os = "windows"))]
use gpui::Corners;
use gpui::{
    AnyElement, App, AvailableSpace, Bounds, ContentMask, Div, Element, ElementId, GlobalElementId,
    InspectorElementId, IntoElement, LayoutId, Pixels, Style, Window, div, prelude::*, px, size,
};

pub(in crate::presentation) fn theme(
    theme: crate::presentation::Theme,
    palette: crate::presentation::Palette,
) -> crate::presentation::Theme {
    let theme = theme.chat_header_theme(palette);
    crate::presentation::Theme {
        sidebar_secondary: theme.text,
        ..theme
    }
}

/// Keep enough tint for labels while letting the current-frame backdrop remain
/// visible. The former 92% tint looked opaque even with a correct GPU blur pass.
pub(in crate::presentation) fn surface(theme: crate::presentation::Theme) -> gpui::Background {
    if cfg!(target_os = "windows") {
        return theme.chat_header_surface();
    }
    let [top, bottom] = [0.84, 0.80];
    gpui::linear_gradient(
        180.,
        gpui::linear_color_stop(
            gpui::Hsla {
                a: top,
                ..theme.surface
            },
            0.,
        ),
        gpui::linear_color_stop(
            gpui::Hsla {
                a: bottom,
                ..theme.surface
            },
            1.,
        ),
    )
}

type History = Box<dyn FnOnce(Pixels, &mut Window, &mut App) -> Div>;

pub(in crate::presentation) fn scroll_under_header(
    header: Div,
    constrained: bool,
    history: impl FnOnce(Pixels, &mut Window, &mut App) -> Div + 'static,
) -> impl IntoElement {
    ScrollUnderHeader {
        header: header.into_any_element(),
        history: Some(Box::new(history)),
        constrained,
    }
}

struct ScrollUnderHeader {
    header: AnyElement,
    history: Option<History>,
    constrained: bool,
}

impl IntoElement for ScrollUnderHeader {
    type Element = Self;
    fn into_element(self) -> Self {
        self
    }
}

impl Element for ScrollUnderHeader {
    type RequestLayoutState = LayoutId;
    type PrepaintState = (AnyElement, Pixels);

    fn id(&self) -> Option<ElementId> {
        None
    }

    fn source_location(&self) -> Option<&'static core::panic::Location<'static>> {
        None
    }

    fn request_layout(
        &mut self,
        _: Option<&GlobalElementId>,
        _: Option<&InspectorElementId>,
        window: &mut Window,
        cx: &mut App,
    ) -> (LayoutId, LayoutId) {
        let header = self.header.request_layout(window, cx);
        let mut history = div()
            .flex_1()
            .min_h_0()
            .when(self.constrained, |d| d.flex_none().h(px(200.)))
            .into_any_element();
        let history = history.request_layout(window, cx);
        let style = Style {
            display: gpui::Display::Flex,
            flex_direction: gpui::FlexDirection::Column,
            flex_grow: 1.,
            min_size: size(px(0.).into(), px(0.).into()),
            ..Default::default()
        };
        (window.request_layout(style, [header, history], cx), header)
    }

    fn prepaint(
        &mut self,
        _: Option<&GlobalElementId>,
        _: Option<&InspectorElementId>,
        bounds: Bounds<Pixels>,
        header_layout: &mut LayoutId,
        window: &mut Window,
        cx: &mut App,
    ) -> Self::PrepaintState {
        let header_height = window.layout_bounds(*header_layout).size.height;
        // Prepaint in reading order. Only the timeline's hitboxes are
        // clipped below the measured header; it still paints underneath.
        // No cached toolbar height or extra frame is needed after wrapping.
        self.header.prepaint(window, cx);
        let mut history =
            (self.history.take().expect("one prepaint per element"))(header_height, window, cx)
                .size_full()
                .into_any_element();
        history.layout_as_root(
            size(
                AvailableSpace::Definite(bounds.size.width),
                AvailableSpace::Definite(bounds.size.height),
            ),
            window,
            cx,
        );
        let visible = Bounds::from_corners(
            gpui::point(
                bounds.left(),
                (bounds.top() + header_height).min(bounds.bottom()),
            ),
            bounds.bottom_right(),
        );
        window.with_content_mask(Some(ContentMask { bounds: visible }), |window| {
            history.prepaint_at(bounds.origin, window, cx);
        });
        (history, header_height.min(bounds.size.height))
    }

    fn paint(
        &mut self,
        _: Option<&GlobalElementId>,
        _: Option<&InspectorElementId>,
        _bounds: Bounds<Pixels>,
        _: &mut LayoutId,
        prepaint: &mut Self::PrepaintState,
        window: &mut Window,
        cx: &mut App,
    ) {
        let (history, _header_height) = prepaint;
        // GPUI registers tab stops while painting. Explicit sibling groups
        // retain header -> history -> composer navigation despite paint order.
        // The live list uses the enclosing panel's clip, including the header.
        window.with_tab_group(Some(1), |window| history.paint(window, cx));
        // Sample the live history already painted in this frame. The toolkit
        // clips the blur to this header; tint and controls are painted after it.
        // Windows paints the explicit opaque white header fallback. It does
        // not request a blur that its renderer cannot provide.
        #[cfg(not(target_os = "windows"))]
        let _ = window.paint_backdrop(
            Bounds::new(_bounds.origin, size(_bounds.size.width, *_header_height)),
            Corners {
                top_left: px(CONTENT_RADIUS),
                top_right: px(CONTENT_RADIUS),
                bottom_left: px(0.),
                bottom_right: px(0.),
            },
            px(16.),
        );
        window.with_tab_group(Some(0), |window| self.header.paint(window, cx));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::presentation::{Palette, Theme, theme::Appearance};

    fn contrast(a: gpui::Rgba, b: gpui::Rgba) -> f32 {
        let luminance = |c: gpui::Rgba| {
            let linear = |v: f32| {
                if v <= 0.04045 {
                    v / 12.92
                } else {
                    ((v + 0.055) / 1.055).powf(2.4)
                }
            };
            0.2126 * linear(c.r) + 0.7152 * linear(c.g) + 0.0722 * linear(c.b)
        };
        let (a, b) = (luminance(a), luminance(b));
        (a.max(b) + 0.05) / (a.min(b) + 0.05)
    }

    #[test]
    fn frosted_header_keeps_labels_focus_and_icons_readable_over_extreme_content() {
        for palette in [Palette::Blue, Palette::Sage] {
            for mode in [Appearance::Light, Appearance::Dark] {
                let original = Theme::resolve(palette, mode, gpui::WindowAppearance::Light);
                let header = theme(original, palette);
                let tint = gpui::Rgba::from(header.surface);
                let alphas = if cfg!(target_os = "windows") {
                    [1., 1.]
                } else {
                    [0.84, 0.80]
                };
                for alpha in alphas {
                    for underlying in [0., 1.] {
                        let background = gpui::Rgba {
                            r: tint.r * alpha + underlying * (1. - alpha),
                            g: tint.g * alpha + underlying * (1. - alpha),
                            b: tint.b * alpha + underlying * (1. - alpha),
                            a: 1.,
                        };
                        assert!(contrast(header.text.into(), background) >= 4.5);
                        assert!(contrast(header.sidebar_secondary.into(), background) >= 4.5);
                        assert!(contrast(header.focus.into(), background) >= 3.);
                        assert!(contrast(header.sidebar_secondary.into(), background) >= 3.);
                    }
                }
            }
        }
    }
}
