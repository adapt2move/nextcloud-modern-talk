//! Compact, composable status rows; action ownership remains with the caller.
use crate::presentation::Desktop;
use gpui::{Div, div, prelude::*, px, rgb};

#[derive(Clone, Copy)]
pub(in crate::presentation) enum BannerRole {
    Info,
    Warning,
    Error,
}

impl Desktop {
    pub(in crate::presentation) fn banner(&self, role: BannerRole, label: &str) -> Div {
        let (foreground, background, symbol) = match role {
            BannerRole::Info => (0x315e98, 0xeaf1fb, "info"),
            BannerRole::Warning => (0x795300, 0xfff4d6, "cloud-off"),
            BannerRole::Error => (0x9e3e32, 0xfff0ec, "info"),
        };
        let foreground = rgb(foreground);
        div().min_h(px(36.)).px_3().py_1().rounded(px(8.))
            .flex().items_center().gap_2().bg(rgb(background)).text_color(foreground)
            .text_size(self.sp(12.)).line_height(self.sp(18.))
            .child(self.scaled_icon(symbol, 16., foreground.into()))
            .child(div().flex_1().min_w_0().child(label.to_owned()))
    }
}
