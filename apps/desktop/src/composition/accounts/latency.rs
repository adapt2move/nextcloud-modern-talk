//! Debug-only input admission to prepared render observations. These timings
//! exclude rejected work and never claim compositor/display presentation.
use super::*;
use talk_core::ui::messaging::MessageAction;

pub(super) struct Start {
    at: Instant,
    context: ConversationRef,
    metric: DiagnosticOperation,
    signature: u128,
}
impl Start {
    pub(super) fn intent(intent: &UiIntent, snapshot: &PresentationSnapshot) -> Option<Self> {
        let (context, metric) = match intent {
            UiIntent::OpenConversation(c) => (*c, DiagnosticOperation::FeedbackSwitch),
            UiIntent::SetFavorite { context, .. } => {
                (*context, DiagnosticOperation::FeedbackFavorite)
            }
            UiIntent::SendDraft { context, .. } => (*context, DiagnosticOperation::FeedbackSend),
            UiIntent::Message {
                context, action, ..
            } => (
                *context,
                match action {
                    MessageAction::Send { .. } => DiagnosticOperation::FeedbackSend,
                    MessageAction::React { .. } => DiagnosticOperation::FeedbackReaction,
                    MessageAction::MarkRead(_) => DiagnosticOperation::FeedbackRead,
                    _ => return None,
                },
            ),
            _ => return None,
        };
        Some(Self {
            at: Instant::now(),
            context,
            metric,
            signature: signature(snapshot, context, metric),
        })
    }
}
pub(super) struct Metrics {
    startup: Option<Instant>,
    pending: Vec<Start>,
}
impl Default for Metrics {
    fn default() -> Self {
        Self {
            startup: Some(Instant::now()),
            pending: Vec::with_capacity(16),
        }
    }
}
fn emit(metric: DiagnosticOperation, at: Instant, outcome: DiagnosticOutcome) {
    diagnostics::record(Diagnostic {
        count: at.elapsed().as_micros().min(u128::from(u64::MAX)) as u64,
        ..Diagnostic::new(metric, outcome)
    });
}
impl Metrics {
    pub(super) fn begin(&mut self, start: Option<Start>) {
        if let Some(start) = start {
            if self.pending.len() == 16 {
                let old = self.pending.remove(0);
                emit(old.metric, old.at, DiagnosticOutcome::Dropped);
            }
            self.pending.push(start);
        }
    }
    pub(super) fn rendered(&mut self, snapshot: &PresentationSnapshot) {
        if (!snapshot.messages.as_slice().is_empty()
            || !snapshot.conversations.as_slice().is_empty())
            && let Some(at) = self.startup.take()
        {
            emit(
                DiagnosticOperation::FeedbackStartup,
                at,
                DiagnosticOutcome::Ready,
            );
        }
        self.pending.retain(|p| {
            let visible = snapshot.selected == Some(p.context);
            let ready = match p.metric {
                DiagnosticOperation::FeedbackSwitch => {
                    visible && !snapshot.messages.as_slice().is_empty()
                }
                DiagnosticOperation::FeedbackFavorite | DiagnosticOperation::FeedbackRead => {
                    signature(snapshot, p.context, p.metric) != p.signature
                }
                _ => visible && signature(snapshot, p.context, p.metric) != p.signature,
            };
            if ready {
                emit(p.metric, p.at, DiagnosticOutcome::Ready);
                false
            } else if p.at.elapsed() >= Duration::from_secs(30) {
                emit(p.metric, p.at, DiagnosticOutcome::Failed);
                false
            } else {
                true
            }
        });
    }
}
fn signature(
    s: &PresentationSnapshot,
    context: ConversationRef,
    metric: DiagnosticOperation,
) -> u128 {
    match metric {
        DiagnosticOperation::FeedbackFavorite => s
            .conversations
            .as_slice()
            .iter()
            .find(|r| r.context == context)
            .map_or(0, |r| match r.favorite {
                None => 0,
                Some(false) => 1,
                Some(true) => 2,
            }),
        DiagnosticOperation::FeedbackRead => s
            .conversations
            .as_slice()
            .iter()
            .find(|r| r.context == context)
            .map_or(0, |r| u128::from(r.unread)),
        DiagnosticOperation::FeedbackReaction => s
            .messages
            .as_slice()
            .iter()
            .flat_map(|m| m.metadata.reactions.as_slice())
            .fold(0u128, |n, r| {
                n.wrapping_mul(31)
                    .wrapping_add(u128::from(r.count) * 2 + u128::from(r.own))
            }),
        _ => s
            .messages
            .as_slice()
            .iter()
            .fold(0u128, |n, m| n.wrapping_mul(31).wrapping_add(m.local_id)),
    }
}
