//! A single bounded, scoped result. No implicit retries or invented delivery.
use super::controls::Availability;
use super::{ButtonStyle, Desktop, LocalAction, Overlay, state::error_label};
use gpui::{AnyElement, Context, div, prelude::*, px};
use std::time::{Duration, Instant};
use talk_core::ui::presentation::{CallStage, ConversationRef, NativeDesktopPort, UiIntent};
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
    surface: Option<Surface>,
    expires_at: Option<Instant>,
}

#[derive(Clone, Copy, Eq, PartialEq)]
enum Surface {
    Main,
    Overlay(Overlay),
}
impl From<Option<Overlay>> for Surface {
    fn from(overlay: Option<Overlay>) -> Self {
        overlay.map_or(Self::Main, Self::Overlay)
    }
}
impl Feedback {
    pub(super) fn dismiss_theme(self, theme: super::Theme) -> super::Theme {
        let foreground = match self.tone {
            Tone::Info => theme.muted,
            Tone::Warning => theme.warning,
            Tone::Error => theme.danger,
        };
        super::Theme {
            sidebar_secondary: foreground,
            hover: gpui::Hsla {
                a: 0.10,
                ..foreground
            },
            ..theme
        }
    }
    pub fn info(text: &'static str) -> Self {
        Self {
            text,
            tone: Tone::Info,
            context: None,
            surface: None,
            expires_at: None,
        }
    }
    pub fn warning(text: &'static str) -> Self {
        Self {
            text,
            tone: Tone::Warning,
            context: None,
            surface: None,
            expires_at: None,
        }
    }
    pub fn notice(text: &'static str) -> Self {
        Self {
            expires_at: Some(Instant::now() + Duration::from_secs(5)),
            ..Self::info(text)
        }
    }
    pub fn error(text: &'static str) -> Self {
        Self {
            text,
            tone: Tone::Error,
            context: None,
            surface: None,
            expires_at: None,
        }
    }
    pub fn in_conversation(mut self, context: ConversationRef) -> Self {
        self.context = Some(context);
        self
    }
    pub fn on_surface(mut self, overlay: Option<Overlay>) -> Self {
        self.surface = Some(overlay.into());
        self
    }
    fn visible_in(self, context: Option<ConversationRef>, overlay: Option<Overlay>) -> bool {
        self.context
            .is_none_or(|expected| context == Some(expected))
            && self
                .surface
                .is_none_or(|expected| expected == overlay.into())
    }
    pub fn expired(self, now: Instant) -> bool {
        self.expires_at.is_some_and(|deadline| now >= deadline)
    }
    pub fn from_draft_error(error: SubmitError) -> Option<Self> {
        Self::from_error(error)
    }
    pub fn from_error(error: SubmitError) -> Option<Self> {
        let text = error_label(error)?;
        Some(match error {
            SubmitError::Unavailable(
                UnavailableReason::Unsupported | UnavailableReason::Unconfigured,
            ) => Self::info(text),
            SubmitError::StaleScope
            | SubmitError::Unavailable(
                UnavailableReason::PolicyUnknown | UnavailableReason::PolicyStale,
            ) => Self::warning(text),
            _ => Self::error(text),
        })
    }
}

/// Automatic observations retry through their existing bounded owner ticks.
/// A non-admission must neither overwrite an action result nor claim a send failed.
fn automatic(intent: &UiIntent) -> bool {
    use talk_core::ui::{content::ContentAction, messaging::MessageAction};
    matches!(
        intent,
        UiIntent::AcknowledgeCreation { .. }
            | UiIntent::AcknowledgeRoom { .. }
            | UiIntent::AcknowledgeMessage { .. }
            | UiIntent::AcknowledgeOrganization { .. }
            | UiIntent::AcknowledgeAccountRemoval { .. }
            | UiIntent::AcknowledgeSubmission { .. }
            | UiIntent::AcknowledgeMutation { .. }
            | UiIntent::Content {
                action: ContentAction::Load(_) | ContentAction::Acknowledge,
                ..
            }
            | UiIntent::Message {
                action: MessageAction::MarkRead(_)
                    | MessageAction::Typing(_)
                    | MessageAction::Mentions(_),
                ..
            }
    )
}

pub(super) fn submit(
    port: &mut dyn NativeDesktopPort,
    feedback: &mut Option<Feedback>,
    intent: UiIntent,
    overlay: Option<Overlay>,
) -> bool {
    let automatic = automatic(&intent);
    // These actions open their destination after synchronous submission, even
    // when admission fails. Their recovery notice belongs to that destination.
    let overlay = match &intent {
        UiIntent::Message {
            action: talk_core::ui::messaging::MessageAction::Thread(_),
            ..
        } => Some(Overlay::Thread),
        UiIntent::Message {
            action: talk_core::ui::messaging::MessageAction::Files,
            ..
        } => Some(Overlay::Files),
        _ => overlay,
    };
    let context = match &intent {
        UiIntent::Message { context, .. }
        | UiIntent::Conversation { context, .. }
        | UiIntent::Refresh(context) => Some(*context),
        _ => None,
    };
    let unsupported = match &intent {
        UiIntent::DownloadFile { .. } => {
            Some("Herunterladen ist in diesem Client noch nicht verfügbar.")
        }
        UiIntent::PickAttachment(_) => {
            Some("Dateien anhängen ist in diesem Client noch nicht verfügbar.")
        }
        UiIntent::ReopenBrowser(_) => {
            Some("Der Browser kann hier noch nicht erneut geöffnet werden.")
        }
        _ => None,
    };
    match port.try_submit(intent) {
        Ok(()) => {
            if !automatic && feedback.is_some_and(|f| f.expires_at.is_some()) {
                *feedback = None;
            }
            true
        }
        Err(error) => {
            if !automatic {
                let result = match (unsupported, error) {
                    (
                        Some(label),
                        SubmitError::Unavailable(
                            UnavailableReason::Unsupported | UnavailableReason::Unconfigured,
                        ),
                    ) => Some(Feedback::info(label)),
                    _ => Feedback::from_error(error),
                };
                if let Some(mut result) = result {
                    result = result.on_surface(overlay);
                    result.context = context;
                    *feedback = Some(result);
                }
            }
            false
        }
    }
}
impl Desktop {
    pub(super) fn report_draft_error(&mut self, error: SubmitError, context: ConversationRef) {
        if let Some(feedback) = Feedback::from_draft_error(error) {
            self.feedback = Some(feedback.in_conversation(context).on_surface(self.overlay));
        }
    }
    pub(super) fn feedback_visible(&self) -> bool {
        self.feedback
            .is_some_and(|f| f.visible_in(self.snapshot.selected, self.overlay))
    }
    pub(super) fn incoming_visible(&self) -> bool {
        self.snapshot.call.as_ref().is_some_and(|c| {
            c.stage == CallStage::Incoming && Some(c.scope) != self.dismissed_incoming
        })
    }
    pub(super) fn local_feedback_visible(&self) -> bool {
        self.feedback.is_some_and(|f| f.context.is_none())
            && self.feedback_visible()
            && self.overlay.is_none()
            && !self.incoming_visible()
    }
    pub(super) fn feedback_view(&self, cx: &Context<Self>) -> AnyElement {
        let t = self.theme;
        let Some(f) = self
            .feedback
            .filter(|f| f.visible_in(self.snapshot.selected, self.overlay))
        else {
            return div().into_any_element();
        };
        let (bg, fg, symbol) = match f.tone {
            Tone::Info => (t.subtle, t.text, "info"),
            Tone::Warning => (t.warning_bg, t.warning, "info"),
            Tone::Error => (t.danger_bg, t.danger, "info"),
        };
        let banner = div()
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
            .child(self.reveal_region("dismiss-feedback", cx));
        self.reveal((f.text, f.tone as usize), banner)
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    use talk_core::ui::{
        AccountId, AccountScope, ConversationId, HostId, RuntimeGeneration, Text,
        content::{AssetTarget, ContentAction},
        messaging::MessageAction,
        presentation::{PresentationSnapshot, RequestId},
    };

    fn context(room: u128) -> ConversationRef {
        let account = AccountId::from_local(1);
        ConversationRef {
            account: AccountScope {
                account,
                runtime: RuntimeGeneration::from_local(1),
            },
            conversation: ConversationId {
                account,
                host: HostId::from_local(1),
                local: room,
            },
        }
    }
    struct Port {
        error: Option<SubmitError>,
        attempts: usize,
    }
    impl NativeDesktopPort for Port {
        fn try_submit(&mut self, _: UiIntent) -> Result<(), SubmitError> {
            self.attempts += 1;
            self.error.map_or(Ok(()), Err)
        }
        fn take_snapshot(&mut self) -> Option<PresentationSnapshot> {
            None
        }
    }
    fn message(action: MessageAction) -> UiIntent {
        UiIntent::Message {
            request: RequestId(1),
            context: context(1),
            action,
        }
    }

    #[test]
    fn automatic_busy_is_non_admission_and_retry_never_replaces_action_feedback() {
        let context = context(1);
        let account = context.account;
        let request = RequestId(1);
        let intents = [
            message(MessageAction::MarkRead(42)),
            message(MessageAction::Typing(true)),
            message(MessageAction::Mentions(Text::new("Mar").unwrap())),
            UiIntent::Content {
                account,
                request,
                action: ContentAction::Load(AssetTarget::SelfAvatar),
            },
            UiIntent::Content {
                account,
                request,
                action: ContentAction::Acknowledge,
            },
            UiIntent::AcknowledgeCreation { account, request },
            UiIntent::AcknowledgeRoom { context, request },
            UiIntent::AcknowledgeMessage { context, request },
            UiIntent::AcknowledgeOrganization { account, request },
            UiIntent::AcknowledgeAccountRemoval {
                account: account.account,
                request,
            },
            UiIntent::AcknowledgeSubmission { context, request },
            UiIntent::AcknowledgeMutation { context, request },
        ];
        for original in [
            None,
            Some(Feedback::warning("Status unklar").in_conversation(context)),
        ] {
            for intent in &intents {
                let mut feedback = original;
                let mut port = Port {
                    error: Some(SubmitError::Busy),
                    attempts: 0,
                };
                assert!(!submit(&mut port, &mut feedback, intent.clone(), None));
                assert!(feedback == original);
                port.error = None;
                assert!(submit(&mut port, &mut feedback, intent.clone(), None));
                assert_eq!(
                    port.attempts, 2,
                    "Busy must remain eligible for owner retry"
                );
                assert!(feedback == original);
            }
        }
    }

    #[test]
    fn rejected_user_action_is_visible_only_in_its_conversation_and_surface() {
        let mut port = Port {
            error: Some(SubmitError::Unavailable(
                UnavailableReason::PermissionDenied,
            )),
            attempts: 0,
        };
        let mut feedback = None;
        assert!(!submit(
            &mut port,
            &mut feedback,
            message(MessageAction::Delete(42)),
            Some(Overlay::Thread)
        ));
        let result = feedback.unwrap();
        assert!(result.visible_in(Some(context(1)), Some(Overlay::Thread)));
        assert!(!result.visible_in(Some(context(2)), Some(Overlay::Thread)));
        assert!(!result.visible_in(None, Some(Overlay::Thread)));
        assert!(!result.visible_in(Some(context(1)), Some(Overlay::Settings)));
        assert!(!result.visible_in(Some(context(1)), None));
        assert!(!result.text.contains("Entwurf"));
        let mut newer_runtime = context(1);
        newer_runtime.account.runtime = RuntimeGeneration::from_local(2);
        assert!(!result.visible_in(Some(newer_runtime), Some(Overlay::Thread)));
    }

    #[test]
    fn requests_that_open_dialogs_report_rejection_in_the_destination() {
        for (action, destination) in [
            (MessageAction::Thread(42), Overlay::Thread),
            (MessageAction::Files, Overlay::Files),
        ] {
            let mut port = Port {
                error: Some(SubmitError::Unavailable(
                    UnavailableReason::PermissionDenied,
                )),
                attempts: 0,
            };
            let mut feedback = None;
            assert!(!submit(&mut port, &mut feedback, message(action), None));
            assert!(
                feedback
                    .unwrap()
                    .visible_in(Some(context(1)), Some(destination))
            );
            assert!(!feedback.unwrap().visible_in(Some(context(1)), None));
        }
    }

    #[test]
    fn quiet_busy_preserves_genuine_feedback_and_does_not_claim_admission() {
        let action = message(MessageAction::React {
            message: 42,
            emoji: Text::new("👍").unwrap(),
            add: true,
        });
        for original in [
            None,
            Some(Feedback::warning("Sendestatus unklar")),
            Feedback::from_error(SubmitError::Unavailable(
                UnavailableReason::PermissionDenied,
            )),
        ] {
            let mut feedback = original;
            let mut port = Port {
                error: Some(SubmitError::Busy),
                attempts: 0,
            };
            assert!(!submit(&mut port, &mut feedback, action.clone(), None));
            assert!(feedback == original);
            assert_eq!(port.attempts, 1);
            port.error = None;
            assert!(submit(&mut port, &mut feedback, action.clone(), None));
            assert!(feedback == original);
            assert_eq!(port.attempts, 2);
        }
    }

    #[test]
    fn explicit_send_busy_preserves_draft_and_reports_non_admission() {
        let mut drafts = crate::presentation::state::Drafts::default();
        let mut port = Port {
            error: Some(SubmitError::Busy),
            attempts: 0,
        };
        let context = context(1);
        drafts.save(context, "Keep this draft").unwrap();
        let error = drafts
            .send(context, talk_core::ui::Decision::Allowed, &mut port)
            .unwrap_err();
        assert_eq!(error, SubmitError::Busy);
        assert_eq!(drafts.text(context), "Keep this draft");
        assert!(!drafts.pending(context));
        assert!(Feedback::from_draft_error(error).is_none());
        port.error = None;
        assert!(
            drafts
                .send(context, talk_core::ui::Decision::Allowed, &mut port)
                .is_ok()
        );
        assert!(drafts.pending(context));
        assert_eq!(drafts.text(context), "Keep this draft");
    }

    #[test]
    fn admission_and_unsupported_are_not_errors_and_uncertain_is_a_warning() {
        assert!(Feedback::from_error(SubmitError::Busy).is_none());
        let expired_work =
            Feedback::from_error(SubmitError::Unavailable(UnavailableReason::Busy)).unwrap();
        assert!(matches!(expired_work.tone, Tone::Error));
        assert!(expired_work.expires_at.is_none());
        assert!(matches!(
            Feedback::from_error(SubmitError::Unavailable(UnavailableReason::Unsupported))
                .unwrap()
                .tone,
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
            .unwrap()
            .tone,
            Tone::Error
        ));
    }
}
