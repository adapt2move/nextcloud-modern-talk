//! Shared elevation and geometry for content surfaces and context menus.
use crate::presentation::{Desktop, Overlay};
use gpui::{BoxShadow, Div, div, point, prelude::*, px, rgb};

pub(in crate::presentation) const CONTENT_RADIUS: f32 = 16.;

impl Desktop {
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

    /// Common opaque dialog panel; callers own geometry, contents and focus.
    pub(in crate::presentation) fn dialog_shell_surface(&self) -> Div {
        self.style_dialog_shell(div())
    }
    pub(in crate::presentation) fn style_dialog_shell<T: Styled>(&self, surface: T) -> T {
        surface
            .bg(self.theme.content_material())
            .rounded(px(16.))
            .border_1()
            .border_color(self.theme.border)
            .shadow_lg()
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
            .border_t_1()
            .border_color(self.theme.border)
    }
    pub(in crate::presentation) fn context_menu_surface(&self) -> Div {
        div()
            .flex()
            .flex_col()
            .gap(px(4.))
            .p_1()
            .rounded(px(12.))
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
            rgb(0x281e0a).into()
        };
        div()
            .relative()
            .size_full()
            .rounded(px(CONTENT_RADIUS))
            .bg(self.theme.content_material())
            .border_1()
            .border_color(if self.theme.dark {
                gpui::rgba(0xffffff10)
            } else {
                gpui::rgba(0x46371e14)
            })
            .shadow(vec![
                BoxShadow {
                    color: color.opacity(if self.theme.dark { 0.18 } else { 0.05 }),
                    offset: point(px(0.), px(1.)),
                    blur_radius: px(2.),
                    spread_radius: px(0.),
                },
                BoxShadow {
                    color: color.opacity(if self.theme.dark { 0.35 } else { 0.07 }),
                    offset: point(px(0.), px(8.)),
                    blur_radius: px(24.),
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
