//! Shared form geometry. Focus borders and disabled pointer behavior use the same
//! control owner as buttons; input entities retain editing and IME state.
use crate::presentation::{Desktop, input::TextInput};
use gpui::{Context, Div, Entity, Focusable, FontWeight, div, prelude::*, px};

impl Desktop {
    /// A composite field shell. Callers add their icon and retained editor;
    /// clicks on padding focus that editor only while it is interactive.
    pub(in crate::presentation) fn input_surface(
        &self,
        input: &Entity<TextInput>,
        cx: &Context<Self>,
    ) -> Div {
        let input = input.clone();
        let focus = input.focus_handle(cx);
        let cursor = if input.read(cx).interactive {
            gpui::CursorStyle::IBeam
        } else {
            gpui::CursorStyle::Arrow
        };
        div()
            .cursor(cursor)
            .relative()
            .flex()
            .items_center()
            .gap_3()
            .min_w_0()
            .min_h(px(36.))
            .px_3()
            .py(px(3.))
            .rounded(px(8.))
            .border_1()
            .border_color(self.theme.input_border)
            .bg(self.theme.subtle)
            .child(self.field_ring(focus.clone(), 8.))
            .on_mouse_down(
                gpui::MouseButton::Left,
                cx.listener(move |_, _, window, cx| {
                    if input.read(cx).interactive {
                        focus.focus(window);
                    }
                }),
            )
    }
    /// A page is a column of groups separated by space, not by cards.
    pub(in crate::presentation) fn dialog_page(&self) -> Div {
        div().flex().flex_col().gap_6().w_full()
    }
    /// A group of related rows under a short label.
    pub(in crate::presentation) fn dialog_section(&self, label: Option<&str>) -> Div {
        div()
            .flex_none()
            .flex()
            .flex_col()
            .gap_2()
            .when_some(label, |d, label| {
                d.child(
                    div()
                        .text_size(self.sp(13.))
                        .line_height(self.sp(18.))
                        .font_weight(FontWeight::SEMIBOLD)
                        .child(self.l(label).to_owned()),
                )
            })
    }
    /// Label and explanation on the left, the single control on the right.
    pub(in crate::presentation) fn dialog_row(
        &self,
        label: &str,
        detail: Option<&str>,
        control: impl IntoElement,
    ) -> Div {
        div()
            .flex_none()
            .flex()
            .items_center()
            .justify_between()
            .gap_4()
            .py(px(10.))
            .border_b_1()
            .border_color(self.theme.border)
            .child(
                div()
                    .min_w_0()
                    .flex_1()
                    .flex()
                    .flex_col()
                    .gap(px(2.))
                    .child(
                        div()
                            .text_size(self.sp(13.))
                            .line_height(self.sp(18.))
                            .font_weight(FontWeight::MEDIUM)
                            .child(self.l(label).to_owned()),
                    )
                    .when_some(detail, |d, detail| d.child(self.hint(self.l(detail)))),
            )
            .child(control)
    }
    /// Buttons keep their intrinsic width instead of stretching across the page.
    pub(in crate::presentation) fn dialog_actions(&self) -> Div {
        div().flex_none().flex().flex_wrap().items_center().gap_2()
    }
    /// Container for one exclusive choice; options keep their natural width.
    pub(in crate::presentation) fn segmented(&self) -> Div {
        div()
            .flex()
            .flex_wrap()
            .gap_1()
            .p(px(2.))
            .rounded(px(8.))
            .bg(self.theme.subtle)
    }
}
