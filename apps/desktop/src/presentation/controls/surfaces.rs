//! Shared elevation and geometry for content surfaces and context menus.
use crate::presentation::Desktop;
use gpui::{BoxShadow, Div, div, point, prelude::*, px, rgb};

pub(in crate::presentation) const CONTENT_RADIUS: f32 = 12.;

impl Desktop {
    pub(in crate::presentation) fn context_menu_surface(&self) -> Div {
        div()
            .flex()
            .flex_col()
            .gap(px(4.))
            .p_1()
            .rounded(px(10.))
            .border_1()
            .border_color(self.theme.border)
            .bg(self.theme.surface)
            .shadow_lg()
    }
    /// Keep the shadow outside the rectangular overflow mask. GPUI 0.2.2 does
    /// not round that mask: full-bleed children must also round their exposed
    /// corners, rather than relying on `overflow_hidden` to clip their fill.
    pub(in crate::presentation) fn content_card(&self, content: Div) -> Div {
        let color = if self.theme.dark {
            gpui::black()
        } else {
            rgb(0x26354b).into()
        };
        div()
            .relative()
            .size_full()
            .rounded(px(CONTENT_RADIUS))
            .bg(self.theme.surface)
            .shadow(vec![
                BoxShadow {
                    color: color.opacity(if self.theme.dark { 0.18 } else { 0.045 }),
                    offset: point(px(0.), px(1.)),
                    blur_radius: px(3.),
                    spread_radius: px(0.),
                },
                BoxShadow {
                    color: color.opacity(if self.theme.dark { 0.22 } else { 0.065 }),
                    offset: point(px(0.), px(3.)),
                    blur_radius: px(10.),
                    spread_radius: px(0.),
                },
            ])
            .child(
                content
                    .relative()
                    .size_full()
                    .rounded(px(CONTENT_RADIUS))
                    .overflow_hidden(),
            )
    }
}
