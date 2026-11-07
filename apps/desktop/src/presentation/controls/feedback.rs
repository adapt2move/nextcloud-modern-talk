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
    fn colors(self, theme: crate::presentation::Theme) -> (gpui::Hsla, gpui::Hsla, &'static str) {
        match self {
            Self::Info if theme.dark => (theme.selected_text, theme.selected, "info"),
            Self::Info => (rgb(0x315e98).into(), rgb(0xeaf1fb).into(), "info"),
            Self::Warning => (theme.warning, theme.warning_bg, "cloud-off"),
            Self::Error => (theme.danger, theme.danger_bg, "info"),
        }
    }

    pub(in crate::presentation) fn foreground(
        self,
        theme: crate::presentation::Theme,
    ) -> gpui::Hsla {
        self.colors(theme).0
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
        let (foreground, _, symbol) = role.colors(self.theme);
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
        let (foreground, background, _) = role.colors(self.theme);
        div()
            .min_h(px(36.))
            .px_3()
            .py_1()
            .rounded(px(8.))
            .border_1()
            .border_color(foreground.opacity(0.20))
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

    pub(in crate::presentation) fn empty_state(
        &self,
        leading: impl IntoElement,
        title: impl Into<gpui::SharedString>,
        detail: &str,
    ) -> Div {
        let title: gpui::SharedString = title.into();
        div()
            .relative()
            .flex()
            .flex_col()
            .items_center()
            .justify_center()
            .gap_2()
            .text_center()
            .children(self.materials.layer(
                if self.theme.dark {
                    super::Texture::PatternDark
                } else {
                    super::Texture::PatternLight
                },
                self.theme.reduced_transparency,
            ))
            .child(leading)
            .child(
                div()
                    .font_weight(gpui::FontWeight::SEMIBOLD)
                    .text_color(self.theme.text)
                    .child(title),
            )
            .child(self.hint(detail))
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
