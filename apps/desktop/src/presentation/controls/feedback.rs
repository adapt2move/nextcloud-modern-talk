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
            Self::Warning => (theme.warning, theme.warning_bg, "info"),
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

impl BannerRole {
    /// The wash behind a notice. It is a faint tint of the semantic background
    /// over the surface it sits on, never a saturated block: the icon carries
    /// the role and the text keeps the neutral primary/secondary tiers.
    pub(in crate::presentation) fn wash(self, theme: crate::presentation::Theme) -> gpui::Hsla {
        let (_, background, _) = self.colors(theme);
        background.opacity(if theme.dark { 0.55 } else { 0.72 })
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

    /// The one frame behind every inline notice, banner and toast: a quiet
    /// rounded wash, no outline, neutral text. Role is expressed by the leading
    /// icon and the words, so it survives monochrome and increased contrast.
    pub(in crate::presentation) fn banner_frame(&self, role: BannerRole) -> Div {
        div()
            .min_h(px(36.))
            .px_3()
            .py(px(6.))
            .rounded(px(10.))
            .flex()
            .items_center()
            .gap_2()
            .bg(role.wash(self.theme))
            .text_color(self.theme.text)
            .text_size(self.sp(13.))
            .line_height(self.sp(18.))
    }

    pub(in crate::presentation) fn banner_with_leading(
        &self,
        role: BannerRole,
        label: &str,
        leading: impl IntoElement,
    ) -> Div {
        self.banner_frame(role)
            .child(div().flex_none().child(leading))
            .child(div().flex_1().min_w_0().child(label.to_owned()))
    }

    /// Two-tier notice: a short title that says what happened and one muted
    /// sentence that says what to do. Callers append an action with
    /// [`Desktop::notice_action`]; at most one action is primary.
    pub(in crate::presentation) fn notice(
        &self,
        role: BannerRole,
        title: &str,
        detail: Option<&str>,
        icon: Option<&'static str>,
    ) -> Div {
        let (foreground, _, symbol) = role.colors(self.theme);
        // A wrapping row: in a wide view the action sits at the end of the
        // line, in the 260 pt sidebar it drops under the text.
        self.banner_frame(role)
            .items_start()
            .flex_wrap()
            .gap_y_1()
            .py(px(8.))
            .child(div().flex_none().pt(px(1.)).child(self.scaled_icon(
                icon.unwrap_or(symbol),
                16.,
                foreground,
            )))
            .child(
                div()
                    .flex_1()
                    .min_w(px(150.))
                    .pr_2()
                    .flex()
                    .flex_col()
                    .child(
                        div()
                            .font_weight(gpui::FontWeight::MEDIUM)
                            .child(title.to_owned()),
                    )
                    .when_some(detail, |d, detail| {
                        d.child(
                            div()
                                .text_size(self.sp(12.))
                                .text_color(self.theme.muted)
                                .child(detail.to_owned()),
                        )
                    }),
            )
    }

    /// Text-weight action inside a notice. It is deliberately not a filled or
    /// outlined button: the notice is information first.
    pub(in crate::presentation) fn notice_action(
        &self,
        id: impl Into<gpui::ElementId>,
        label: &'static str,
        action: crate::presentation::LocalAction,
        cx: &gpui::Context<Self>,
    ) -> gpui::Stateful<Div> {
        self.button(
            id,
            label,
            crate::presentation::ButtonStyle::Quiet,
            true,
            action,
            cx,
        )
        .flex_none()
        .ml(px(24.))
        .min_h(px(28.))
        .py(px(2.))
        .px_2()
        .text_size(self.sp(13.))
        .font_weight(gpui::FontWeight::MEDIUM)
        .text_color(self.theme.selected_text)
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
