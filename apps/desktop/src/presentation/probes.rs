//! Explicit development scenarios. No production navigation or backend capability.
use super::{ButtonStyle, Desktop, LocalAction, controls::Availability};
use gpui::{Context, Div, div, prelude::*, px};

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Probe {
    #[default]
    None,
    Controls,
    Focus,
    Accounts,
    Messages,
}
impl Desktop {
    pub(super) fn control_probe(&self, cx: &Context<Self>) -> gpui::Stateful<Div> {
        let mut body = div()
            .flex()
            .flex_col()
            .gap_6()
            .p_8()
            .child(self.title("Control-Zustände · UI-Entwicklung"))
            .child(self.hint("Fiktive Verfügbarkeit. Keine Netzwerk- oder Dateiaktion."));
        for (i, (name, style)) in [
            ("Primary", ButtonStyle::Primary),
            ("Danger", ButtonStyle::Danger),
            ("Secondary", ButtonStyle::Secondary),
            ("Quiet", ButtonStyle::Quiet),
        ]
        .into_iter()
        .enumerate()
        {
            body = body.child(
                div()
                    .flex()
                    .items_center()
                    .gap_6()
                    .min_h(px(80.))
                    .child(div().w(px(100.)).child(name))
                    .child(
                        self.button(
                            ("probe-disabled", i),
                            "Fortfahren",
                            style,
                            Availability::when(false, "In diesem UI-Beispiel nicht verfügbar."),
                            LocalAction::Intent(talk_core::ui::presentation::UiIntent::Refresh(
                                self.snapshot
                                    .selected
                                    .expect("explicit fixture conversation"),
                            )),
                            cx,
                        ),
                    )
                    .child(
                        self.icon_button(
                            ("probe-icon-disabled", i),
                            ("arrow-up", "Fortfahren"),
                            style,
                            Availability::when(false, "In diesem UI-Beispiel nicht verfügbar."),
                            LocalAction::Intent(talk_core::ui::presentation::UiIntent::Refresh(
                                self.snapshot
                                    .selected
                                    .expect("explicit fixture conversation"),
                            )),
                            cx,
                        ),
                    )
                    .child(
                        self.button(
                            ("probe-enabled", i),
                            "Fortfahren",
                            style,
                            true,
                            LocalAction::Intent(talk_core::ui::presentation::UiIntent::Refresh(
                                self.snapshot
                                    .selected
                                    .expect("explicit fixture conversation"),
                            )),
                            cx,
                        ),
                    )
                    .child(
                        self.icon_button(
                            ("probe-icon-enabled", i),
                            ("arrow-up", "Fortfahren"),
                            style,
                            Availability::available(),
                            LocalAction::Intent(talk_core::ui::presentation::UiIntent::Refresh(
                                self.snapshot
                                    .selected
                                    .expect("explicit fixture conversation"),
                            )),
                            cx,
                        ),
                    ),
            );
        }
        div()
            .id("control-probe")
            .size_full()
            .overflow_y_scroll()
            .bg(self.theme.surface)
            .child(body)
    }
}
