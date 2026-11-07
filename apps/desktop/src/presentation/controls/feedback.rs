//! Compact, composable status rows; action ownership remains with the caller.
use crate::presentation::Desktop;
use gpui::{Div, div, prelude::*, px, rgb};

#[derive(Clone, Copy)]
pub(in crate::presentation) enum BannerRole {
    Info,
    Warning,
    Error,
}

impl BannerRole {
    fn colors(self) -> (gpui::Hsla, gpui::Hsla, &'static str) {
        let (foreground, background, symbol) = match self {
            Self::Info => (0x315e98, 0xeaf1fb, "info"),
            Self::Warning => (0x795300, 0xfff4d6, "cloud-off"),
            Self::Error => (0x9e3e32, 0xfff0ec, "info"),
        };
        (rgb(foreground).into(), rgb(background).into(), symbol)
    }

    pub(in crate::presentation) fn foreground(self) -> gpui::Hsla {
        self.colors().0
    }
}

impl Desktop {
    pub(in crate::presentation) fn banner(&self, role: BannerRole, label: &str) -> Div {
        self.banner_with_icon(role, label, None)
    }

    pub(in crate::presentation) fn banner_with_icon(
        &self,
        role: BannerRole,
        label: &str,
        icon: Option<&'static str>,
    ) -> Div {
        let (foreground, _, symbol) = role.colors();
        self.banner_with_leading(
            role,
            label,
            self.scaled_icon(icon.unwrap_or(symbol), 16., foreground),
        )
    }

    pub(in crate::presentation) fn banner_with_leading(
        &self,
        role: BannerRole,
        label: &str,
        leading: impl IntoElement,
    ) -> Div {
        let (foreground, background, _) = role.colors();
        div()
            .min_h(px(36.))
            .px_3()
            .py_1()
            .rounded(px(8.))
            .flex()
            .items_center()
            .gap_2()
            .bg(background)
            .text_color(foreground)
            .text_size(self.sp(12.))
            .line_height(self.sp(18.))
            .child(leading)
            .child(div().flex_1().min_w_0().child(label.to_owned()))
    }

    pub(in crate::presentation) fn empty_row(&self, label: &str) -> Div {
        div()
            .min_h(px(40.))
            .flex()
            .items_center()
            .px_3()
            .child(self.hint(label))
    }

    pub(in crate::presentation) fn loading_row(&self, label: &str) -> Div {
        div()
            .min_h(px(40.))
            .flex()
            .items_center()
            .gap_2()
            .px_3()
            .child(self.pending_dots(self.theme.muted))
            .child(self.hint(label))
    }
}
