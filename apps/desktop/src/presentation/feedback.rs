//! A single bounded, scoped result. No implicit retries or invented delivery.
use super::controls::Availability;
use super::{ButtonStyle, Desktop, LocalAction, state::error_label};
use gpui::{Context, Div, div, prelude::*, px};
use talk_core::ui::presentation::{CallStage, ConversationRef};
use talk_core::ui::{SubmitError, UnavailableReason};

#[derive(Clone, Copy, Eq, PartialEq)]
enum Tone {
    Info,
    Warning,
    Error,
}
#[derive(Clone, Copy, Eq, PartialEq)]
pub(super) struct Feedback {
    text: &'static str,
    tone: Tone,
    pub context: Option<ConversationRef>,
}
impl Feedback {
    pub fn info(text: &'static str) -> Self {
        Self {
            text,
            tone: Tone::Info,
            context: None,
        }
    }
    pub fn warning(text: &'static str) -> Self {
        Self {
            text,
            tone: Tone::Warning,
            context: None,
        }
    }
    pub fn error(text: &'static str) -> Self {
        Self {
            text,
            tone: Tone::Error,
            context: None,
        }
    }
    pub fn in_conversation(mut self, context: ConversationRef) -> Self {
        self.context = Some(context);
        self
    }
    pub fn from_error(error: SubmitError) -> Self {
        let text = error_label(error);
        match error {
            SubmitError::Busy
            | SubmitError::Unavailable(
                UnavailableReason::Unsupported | UnavailableReason::Unconfigured,
            ) => Self::info(text),
            SubmitError::StaleScope
            | SubmitError::Unavailable(
                UnavailableReason::PolicyUnknown | UnavailableReason::PolicyStale,
            ) => Self::warning(text),
            _ => Self::error(text),
        }
    }
}
impl Desktop {
    pub(super) fn incoming_visible(&self) -> bool {
        self.snapshot.call.as_ref().is_some_and(|c| {
            c.stage == CallStage::Incoming && Some(c.scope) != self.dismissed_incoming
        })
    }
    pub(super) fn local_feedback_visible(&self) -> bool {
        self.feedback.is_some_and(|f| f.context.is_none())
            && self.overlay.is_none()
            && !self.incoming_visible()
    }
    pub(super) fn feedback_view(&self, cx: &Context<Self>) -> Div {
        let t = self.theme;
        let Some(f) = self.feedback else {
            return div();
        };
        let (bg, fg, symbol) = match f.tone {
            Tone::Info => (t.subtle, t.text, "info"),
            Tone::Warning => (t.warning_bg, t.warning, "info"),
            Tone::Error => (t.danger_bg, t.danger, "info"),
        };
        div()
            .relative()
            .occlude()
            .rounded(px(8.))
            .bg(bg)
            .text_color(fg)
            .p_3()
            .flex()
            .items_start()
            .gap_3()
            .child(self.scaled_icon(symbol, 18., fg))
            .child(
                div()
                    .min_w_0()
                    .flex_1()
                    .text_size(self.sp(12.))
                    .line_height(self.sp(18.))
                    .child(self.l(f.text)),
            )
            .child(self.icon_button(
                "dismiss-feedback",
                ("x", "Meldung schließen"),
                ButtonStyle::Quiet,
                Availability::available(),
                LocalAction::DismissFeedback,
                cx,
            ))
            .child(self.reveal_region("dismiss-feedback", cx))
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn admission_and_unsupported_are_not_errors_and_uncertain_is_a_warning() {
        assert!(matches!(
            Feedback::from_error(SubmitError::Busy).tone,
            Tone::Info
        ));
        assert!(matches!(
            Feedback::from_error(SubmitError::Unavailable(UnavailableReason::Unsupported)).tone,
            Tone::Info
        ));
        assert!(matches!(
            Feedback::warning("Status unklar").tone,
            Tone::Warning
        ));
        assert!(matches!(
            Feedback::from_error(SubmitError::Unavailable(
                UnavailableReason::PermissionDenied
            ))
            .tone,
            Tone::Error
        ));
    }
}
