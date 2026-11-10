//! Shared elevation and geometry for content surfaces and context menus.
use crate::presentation::{Desktop, Overlay};
use gpui::{Div, div, prelude::*, px};

use super::material;
use crate::presentation::theme::Elevation;

pub(in crate::presentation) const CONTENT_RADIUS: f32 = 16.;

#[derive(Clone, Copy)]
pub(in crate::presentation) enum CardStyle {
    Inset,
    InsetRaised,
    Raised,
    Outlined,
}

impl Desktop {
    pub(in crate::presentation) fn style_dialog_title<T: Styled>(&self, title: T) -> T {
        title
            .text_size(self.sp(20.))
            .line_height(self.sp(28.))
            .font_weight(gpui::FontWeight(650.))
            .text_color(self.theme.text)
    }

    pub(in crate::presentation) fn style_panel_title<T: Styled>(&self, title: T) -> T {
        title
            .text_size(self.sp(18.))
            .line_height(self.sp(24.))
            .font_weight(gpui::FontWeight::SEMIBOLD)
            .text_color(self.theme.text)
    }

    pub(in crate::presentation) fn style_card<T: Styled>(&self, card: T, style: CardStyle) -> T {
        let card = card.rounded(px(12.));
        match style {
            CardStyle::Inset => card.bg(self.theme.inset()),
            CardStyle::InsetRaised => card
                .bg(self.theme.inset())
                .shadow(self.theme.elevation(Elevation::Raised)),
            CardStyle::Raised => card
                .bg(self.theme.surface)
                .border_1()
                .border_color(self.theme.border)
                .shadow(self.theme.elevation(Elevation::Raised)),
            CardStyle::Outlined => card.border_1().border_color(self.theme.border),
        }
    }

    pub(in crate::presentation) fn modal_scrim_color(&self) -> gpui::Hsla {
        if self.incoming_visible() {
            gpui::rgba(0x0f172a66).into()
        } else if matches!(
            self.overlay,
            Some(Overlay::ConfirmRecording | Overlay::ConfirmEndCall | Overlay::RecordingConsent)
        ) {
            gpui::rgba(0x0f172a80).into()
        } else {
            self.theme.scrim
        }
    }

    pub(in crate::presentation) fn dialog_scrim(&self) -> Div {
        div()
            .absolute()
            .inset_0()
            .occlude()
            .bg(self.modal_scrim_color())
            .flex()
    }

    pub(in crate::presentation) fn selected_row_color(
        &self,
        selected: bool,
        fallback: gpui::Hsla,
    ) -> gpui::Hsla {
        if selected {
            self.theme.selected
        } else {
            fallback
        }
    }

    /// Common dialog panel; callers own geometry, contents and focus.
    pub(in crate::presentation) fn dialog_shell_surface(&self) -> Div {
        self.style_dialog_shell(div())
    }
    pub(in crate::presentation) fn style_dialog_shell<T: Styled + ParentElement>(
        &self,
        surface: T,
    ) -> T {
        let t = self.theme;
        self.style_floating_surface(
            surface.relative(),
            t.surface,
            t.dark,
            16.,
            Elevation::Dialog,
        )
    }

    pub(in crate::presentation) fn style_floating_surface<T: Styled + ParentElement>(
        &self,
        surface: T,
        tint: gpui::Hsla,
        dark: bool,
        radius: f32,
        elevation: Elevation,
    ) -> T {
        let t = self.theme;
        surface
            .bg(if t.reduced_transparency {
                tint
            } else {
                gpui::transparent_black()
            })
            .rounded(px(radius))
            .border_1()
            .border_color(t.border)
            .shadow(t.elevation(elevation))
            .children((!t.reduced_transparency).then(|| material::glass(tint, dark, radius)))
            .child(inner_highlight(t, radius))
    }
    pub(in crate::presentation) fn dialog_header(&self) -> Div {
        div()
            .flex_none()
            .flex()
            .items_center()
            .justify_between()
            .gap_3()
            .px_4()
            .py_4()
            .rounded_t(px(15.))
            .bg(self.theme.inset())
            .border_b_1()
            .border_color(self.theme.border)
    }
    pub(in crate::presentation) fn dialog_footer(&self) -> Div {
        div()
            .flex_none()
            .flex()
            .flex_wrap()
            .items_center()
            .justify_end()
            .gap_2()
            .px_4()
            .py_3()
            .rounded_b(px(15.))
            .bg(self.theme.inset())
            .border_t_1()
            .border_color(self.theme.border)
    }
    pub(in crate::presentation) fn context_menu_surface(&self) -> Div {
        let t = self.theme;
        self.style_floating_surface(div().relative(), t.surface, t.dark, 12., Elevation::Popover)
            .flex()
            .flex_col()
            .gap(px(4.))
            .p_1()
    }
    /// Keep the shadow outside the rectangular overflow mask. GPUI 0.2.2 does
    /// not round that mask: full-bleed children must also round their exposed
    /// corners, rather than relying on `overflow_hidden` to clip their fill.
    pub(in crate::presentation) fn content_card(&self, content: Div) -> Div {
        let t = self.theme;
        div()
            .relative()
            .size_full()
            .rounded(px(CONTENT_RADIUS))
            .bg(self.theme.content_material())
            .border_1()
            .border_color(if self.theme.dark {
                gpui::rgba(0xffffff10)
            } else {
                gpui::rgba(0x26354b14)
            })
            .shadow(t.elevation(Elevation::Card))
            .child(
                content
                    .relative()
                    .size_full()
                    .rounded(px(CONTENT_RADIUS))
                    .overflow_hidden(),
            )
            .child(inner_highlight(t, CONTENT_RADIUS))
    }
}

pub(in crate::presentation) fn inner_highlight(
    theme: crate::presentation::Theme,
    radius: f32,
) -> Div {
    div()
        .absolute()
        .inset(px(1.))
        .rounded(px((radius - 1.).max(0.)))
        .border_t_1()
        .border_color(theme.inner_highlight())
}
