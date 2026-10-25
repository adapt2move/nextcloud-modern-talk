//! Static, bounded placeholders for data that has not arrived yet.
use crate::presentation::Desktop;
use gpui::{Div, div, prelude::*, px};

#[derive(Clone, Copy)]
pub(in crate::presentation) enum Skeleton {
    Rows,
    Messages,
}

impl Desktop {
    /// These placeholders own no animation, timer or request. A refresh keeps
    /// confirmed rows mounted; only an initial load uses a skeleton.
    pub(in crate::presentation) fn collection_skeleton(&self, kind: Skeleton) -> Div {
        let color = self
            .theme
            .muted
            .opacity(if self.theme.dark { 0.16 } else { 0.12 });
        let bar = |width: f32, height: f32| {
            div()
                .flex_none()
                .w_full()
                .max_w(px(width))
                .h(px(height))
                .rounded(px(4.))
                .bg(color)
        };
        match kind {
            Skeleton::Rows => {
                let mut rows = div().flex_none().flex().flex_col().gap_1().py_2();
                rows = rows.child(
                    div()
                        .h(px(24.))
                        .px_2()
                        .flex()
                        .items_center()
                        .child(bar(72., 7.)),
                );
                for width in [112., 144., 96., 128., 104.] {
                    rows = rows.child(
                        div()
                            .h(px(32.))
                            .flex()
                            .items_center()
                            .gap_2()
                            .px_3()
                            .child(div().flex_none().size(px(20.)).rounded_full().bg(color))
                            .child(div().flex_1().min_w_0().child(bar(width, 8.))),
                    );
                }
                rows
            }
            Skeleton::Messages => {
                let mut messages = div()
                    .flex_1()
                    .min_h_0()
                    .overflow_hidden()
                    .flex()
                    .flex_col()
                    .gap_6()
                    .p_6();
                for (first, second) in [(280., 190.), (360., 250.), (220., 310.), (320., 160.)] {
                    messages = messages.child(
                        div()
                            .flex_none()
                            .flex()
                            .items_start()
                            .gap_3()
                            .child(div().flex_none().size(px(28.)).rounded_full().bg(color))
                            .child(
                                div()
                                    .flex_1()
                                    .min_w_0()
                                    .flex()
                                    .flex_col()
                                    .gap_2()
                                    .child(
                                        div()
                                            .h(px(16.))
                                            .flex()
                                            .items_center()
                                            .gap_3()
                                            .child(bar(92., 8.))
                                            .child(bar(30., 6.)),
                                    )
                                    .child(bar(first, 9.))
                                    .child(bar(second, 9.)),
                            ),
                    );
                }
                messages
            }
        }
    }
}
