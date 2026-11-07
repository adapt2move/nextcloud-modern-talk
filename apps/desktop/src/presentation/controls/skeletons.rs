//! Bounded placeholders for data that has not arrived yet.
use crate::presentation::Desktop;
use gpui::{Animation, AnimationExt, Div, div, prelude::*, px};

#[derive(Clone, Copy)]
pub(in crate::presentation) enum Skeleton {
    Rows,
    Messages,
}

impl Desktop {
    /// A refresh keeps confirmed rows mounted; only an initial load uses a
    /// skeleton. Its one shimmer is retired with the placeholder, and no task
    /// or request belongs to it.
    pub(in crate::presentation) fn collection_skeleton(
        &self,
        kind: Skeleton,
        id: &'static str,
    ) -> Div {
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
        let content = match kind {
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
        };
        let content = content.relative().overflow_hidden();
        if self.reduced_motion || self.theme.reduced_transparency {
            content
        } else {
            let clear = gpui::transparent_black();
            let glint = self
                .theme
                .text
                .opacity(if self.theme.dark { 0.045 } else { 0.025 });
            let shimmer = div()
                .absolute()
                .top_0()
                .bottom_0()
                .w(px(160.))
                .flex()
                .child(div().flex_1().h_full().bg(gpui::linear_gradient(
                    90.,
                    gpui::linear_color_stop(clear, 0.),
                    gpui::linear_color_stop(glint, 1.),
                )))
                .child(div().flex_1().h_full().bg(gpui::linear_gradient(
                    90.,
                    gpui::linear_color_stop(glint, 0.),
                    gpui::linear_color_stop(clear, 1.),
                )))
                .with_animation(
                    id,
                    Animation::new(crate::presentation::motion::SHIMMER).repeat(),
                    |element, progress| element.left(gpui::relative(-0.2 + 1.4 * progress)),
                );
            content.child(shimmer)
        }
    }
}
