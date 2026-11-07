//! Debug-only input admission to prepared render observations. These timings
//! exclude rejected work and never claim compositor/display presentation.
use super::*;
use talk_core::ui::messaging::MessageAction;

pub(super) struct Start {
    at: Instant,
    context: ConversationRef,
    stable: Option<talk_core::preferences::navigation::NavigationRoomKey>,
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
                    MessageAction::MarkUnread => DiagnosticOperation::FeedbackRead,
                    MessageAction::MarkRead(_) => DiagnosticOperation::FeedbackRead,
                    _ => return None,
                },
            ),
            _ => return None,
        };
        Some(Self {
            at: Instant::now(),
            context,
            stable: snapshot
                .conversations
                .as_slice()
                .iter()
                .find(|r| r.context == context)
                .and_then(|r| r.navigation_key.clone()),
            metric,
            signature: signature(snapshot, context, metric),
        })
    }
}
pub(super) struct Metrics {
    enabled: bool,
    startup: Option<Instant>,
    startup_messages: Option<Instant>,
    pending: Vec<Start>,
    visible: Vec<(ConversationRef, u128, u128)>,
    selected: Option<(ConversationRef, u128, u128)>,
}
impl Default for Metrics {
    fn default() -> Self {
        Self {
            enabled: false,
            startup: Some(Instant::now()),
            startup_messages: Some(Instant::now()),
            pending: Vec::with_capacity(16),
            visible: Vec::new(),
            selected: None,
        }
    }
}
fn emit(metric: DiagnosticOperation, at: Instant, outcome: DiagnosticOutcome) {
    let operation = match metric {
        DiagnosticOperation::FeedbackStartup => "startup",
        DiagnosticOperation::FeedbackStartupMessages => "startup-messages",
        DiagnosticOperation::FeedbackSwitch => "switch",
        DiagnosticOperation::FeedbackSend => "send",
        DiagnosticOperation::FeedbackReaction => "reaction",
        DiagnosticOperation::FeedbackRead => "read",
        DiagnosticOperation::FeedbackFavorite => "favorite",
        _ => "other",
    };
    println!(
        "DEV_METRIC {operation} {} {}",
        at.elapsed().as_micros(),
        if matches!(outcome, DiagnosticOutcome::Ready) {
            "ready"
        } else {
            "incomplete"
        }
    );
    diagnostics::record(Diagnostic {
        count: at.elapsed().as_micros().min(u128::from(u64::MAX)) as u64,
        ..Diagnostic::new(metric, outcome)
    });
}
impl Metrics {
    pub(super) fn enable(&mut self) {
        if !self.enabled {
            println!("DEV_PROBE enabled");
        }
        self.enabled = true;
    }
    pub(super) fn intent(
        &self,
        intent: &UiIntent,
        snapshot: &PresentationSnapshot,
    ) -> Option<Start> {
        if !self.enabled {
            return None;
        }
        let mut start = Start::intent(intent, snapshot)?;
        // Compare against what was actually painted. Canonical room state can
        // differ from the newest optimistic overlay the user just toggled.
        if let Some((_, favorite, read)) = self.visible.iter().find(|(c, _, _)| *c == start.context)
        {
            match start.metric {
                DiagnosticOperation::FeedbackFavorite => start.signature = *favorite,
                DiagnosticOperation::FeedbackRead => start.signature = *read,
                _ => (),
            }
        }
        if matches!(
            intent,
            UiIntent::Message {
                action: MessageAction::MarkRead(_),
                ..
            }
        ) && start.signature == 0
        {
            return None;
        }
        if let Some((context, messages, reactions)) = self.selected
            && context == start.context
        {
            match start.metric {
                DiagnosticOperation::FeedbackSend => start.signature = messages,
                DiagnosticOperation::FeedbackReaction => start.signature = reactions,
                _ => (),
            }
        }
        Some(start)
    }
    pub(super) fn begin(&mut self, start: Option<Start>) {
        if !self.enabled {
            return;
        }
        if let Some(start) = start {
            if self.pending.len() == 16 {
                let old = self.pending.remove(0);
                emit(old.metric, old.at, DiagnosticOutcome::Dropped);
            }
            self.pending.push(start);
        }
    }
    pub(super) fn rendered(&mut self, snapshot: &PresentationSnapshot) {
        if !self.enabled {
            return;
        }
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
        if !snapshot.messages.as_slice().is_empty()
            && let Some(at) = self.startup_messages.take()
        {
            emit(
                DiagnosticOperation::FeedbackStartupMessages,
                at,
                DiagnosticOutcome::Ready,
            );
        }
        self.pending.retain(|p| {
            let context = p
                .stable
                .as_ref()
                .and_then(|key| {
                    snapshot.conversations.as_slice().iter().find(|r| {
                        r.context.account.account == p.context.account.account
                            && r.navigation_key.as_ref() == Some(key)
                    })
                })
                .map_or(p.context, |r| r.context);
            let visible = snapshot.selected == Some(context);
            let ready = match p.metric {
                DiagnosticOperation::FeedbackSwitch => {
                    visible && !snapshot.messages.as_slice().is_empty()
                }
                DiagnosticOperation::FeedbackFavorite | DiagnosticOperation::FeedbackRead => {
                    signature(snapshot, context, p.metric) != p.signature
                }
                _ => visible && signature(snapshot, context, p.metric) != p.signature,
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
        self.visible = snapshot
            .conversations
            .as_slice()
            .iter()
            .take(256)
            .map(|r| {
                (
                    r.context,
                    signature(snapshot, r.context, DiagnosticOperation::FeedbackFavorite),
                    signature(snapshot, r.context, DiagnosticOperation::FeedbackRead),
                )
            })
            .collect();
        self.selected = snapshot.selected.map(|context| {
            (
                context,
                signature(snapshot, context, DiagnosticOperation::FeedbackSend),
                signature(snapshot, context, DiagnosticOperation::FeedbackReaction),
            )
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
        DiagnosticOperation::FeedbackRead => {
            s.conversations
                .as_slice()
                .iter()
                .find(|r| r.context == context)
                .map_or(0, |r| u128::from(r.unread) << 1)
                | u128::from(
                    s.message_panel
                        .as_ref()
                        .is_some_and(|p| p.context == context && p.unread_queued),
                )
        }
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

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn toggling_an_optimistic_favorite_measures_against_painted_state_not_its_old_canonical_value()
    {
        let mut canonical = crate::composition::fixtures::snapshot(Some("chat"));
        let context = canonical.selected.unwrap();
        canonical.conversations = page(canonical.conversations.as_slice().iter().cloned().map(
            |mut r| {
                if r.context == context {
                    r.favorite = Some(false);
                }
                r
            },
        ));
        let mut painted = canonical.clone();
        painted.conversations = page(painted.conversations.as_slice().iter().cloned().map(
            |mut r| {
                if r.context == context {
                    r.favorite = Some(true);
                }
                r
            },
        ));
        let mut metrics = Metrics::default();
        metrics.enable();
        metrics.rendered(&painted);
        let start = metrics.intent(
            &UiIntent::SetFavorite {
                request: RequestId(1),
                context,
                favorite: false,
            },
            &canonical,
        );
        metrics.begin(start);
        metrics.rendered(&canonical);
        assert!(metrics.pending.is_empty());
    }
}
