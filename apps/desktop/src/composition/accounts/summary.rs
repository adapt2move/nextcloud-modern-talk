//! One explicit summary request, with finite status polling and scoped results.
use super::*;
use talk_core::protocol::messaging as wire;
use talk_core::ui::messaging::{MessageFeature, Summary};

#[derive(Clone, Copy, Eq, PartialEq)]
enum Phase {
    Start(u64),
    Status(OperationId),
    Continue(OperationId),
}
struct Job {
    context: ConversationRef,
    request: RequestId,
    phase: Phase,
    parts: Page<Text<4096>, 4>,
    in_flight: bool,
    failures: u8,
    due: Instant,
    deadline: Instant,
}
#[derive(Default)]
pub(super) struct Summaries {
    job: Option<Job>,
}
impl Summaries {
    pub(super) fn retain_accounts(&mut self, scopes: &[AccountScope]) {
        if self
            .job
            .as_ref()
            .is_some_and(|job| !scopes.contains(&job.context.account))
        {
            self.job = None;
        }
    }
}
impl Accounts {
    pub(super) fn begin_summary(
        &mut self,
        request: RequestId,
        context: ConversationRef,
    ) -> Result<(), SubmitError> {
        if self.snapshot.selected != Some(context) || self.summary.job.is_some() {
            return Err(SubmitError::Busy);
        }
        let panel = self
            .snapshot
            .message_panel
            .as_ref()
            .filter(|panel| panel.context == context)
            .ok_or(SubmitError::StaleScope)?;
        let Some(Summary::Offer { from, .. }) = panel.summary.as_ref() else {
            return Err(SubmitError::StaleScope);
        };
        if !panel.availability(MessageFeature::Summary).enabled() {
            return Err(SubmitError::StaleScope);
        }
        let from = u64::try_from(*from).map_err(|_| SubmitError::StaleScope)?;
        let now = Instant::now();
        self.summary.job = Some(Job {
            context,
            request,
            phase: Phase::Start(from),
            parts: Page::empty(),
            in_flight: false,
            failures: 0,
            due: now,
            deadline: now + Duration::from_secs(300),
        });
        self.history.set_summary(
            context,
            Summary::Pending {
                parts: Page::empty(),
            },
        );
        self.project_history(context);
        self.touch();
        Ok(())
    }

    pub(super) fn dismiss_summary(&mut self, context: ConversationRef) -> Result<(), SubmitError> {
        if self.snapshot.selected != Some(context) {
            return Err(SubmitError::StaleScope);
        }
        if self
            .summary
            .job
            .as_ref()
            .is_some_and(|job| job.context == context)
        {
            self.summary.job = None;
        }
        self.history.dismiss_summary(context);
        self.project_history(context);
        self.touch();
        Ok(())
    }

    pub(super) fn tick_summary(&mut self) {
        let Some(job) = self.summary.job.as_ref() else {
            return;
        };
        if self.snapshot.selected != Some(job.context) || Instant::now() >= job.deadline {
            let context = job.context;
            self.summary.job = None;
            self.history.set_summary(context, Summary::Failed);
            if self.snapshot.selected == Some(context) {
                self.project_history(context);
                self.touch();
            }
            return;
        }
        if job.in_flight || Instant::now() < job.due {
            return;
        }
        let context = job.context;
        let request = job.request;
        let (command, work) = match job.phase {
            Phase::Start(from) => (
                wire::MessagingCommand::Summarize {
                    from: protocol::MessageId {
                        conversation: context.conversation,
                        sequence: from,
                    },
                },
                messaging::Work::SummaryStart,
            ),
            Phase::Status(handle) => (
                wire::MessagingCommand::SummaryStatus { handle },
                messaging::Work::SummaryStatus,
            ),
            Phase::Continue(handle) => (
                wire::MessagingCommand::SummaryContinue { handle },
                messaging::Work::SummaryContinue,
            ),
        };
        let started = self.start_message(request, context, command, work).is_ok();
        if let Some(job) = self.summary.job.as_mut() {
            if started {
                job.in_flight = true;
            } else {
                job.due = Instant::now() + Duration::from_secs(1);
            }
        }
    }

    pub(super) fn summary_reply(
        &mut self,
        request: RequestId,
        context: ConversationRef,
        operation: OperationId,
        work: messaging::Work,
        reply: Result<RuntimeReply, IdentityError>,
    ) -> Result<RuntimeReply, IdentityError> {
        if self.current(context.account).is_err() || self.snapshot.selected != Some(context) {
            self.summary.job = None;
            return Err(IdentityError::Request(ErrorKind::WrongScope));
        }
        let Some(job) = self
            .summary
            .job
            .as_mut()
            .filter(|job| job.request == request && job.context == context && job.in_flight)
        else {
            return Err(IdentityError::Request(ErrorKind::WrongScope));
        };
        let result = match &reply {
            Ok(RuntimeReply::Protocol(ProtocolReply::Messaging {
                conversation,
                operation: returned,
                result,
            })) if *conversation == context.conversation && *returned == operation => Some(result),
            _ => None,
        };
        let mut finish = None;
        let mut valid = true;
        match (job.phase, work, result) {
            (
                Phase::Start(_),
                messaging::Work::SummaryStart,
                Some(wire::MessagingResult::SummaryStarted { handle }),
            ) if *handle == operation => {
                job.phase = Phase::Status(*handle);
                job.failures = 0;
                job.due = Instant::now() + Duration::from_secs(2);
            }
            (
                Phase::Continue(expected),
                messaging::Work::SummaryContinue,
                Some(wire::MessagingResult::SummaryStarted { handle }),
            ) if *handle == expected => {
                job.phase = Phase::Status(*handle);
                job.failures = 0;
                job.due = Instant::now() + Duration::from_secs(2);
            }
            (
                Phase::Status(_),
                messaging::Work::SummaryStatus,
                Some(wire::MessagingResult::SummaryPending),
            ) => {
                job.failures = 0;
                job.due = Instant::now() + Duration::from_secs(2);
            }
            (
                Phase::Status(handle),
                messaging::Work::SummaryStatus,
                Some(wire::MessagingResult::SummaryPart { text, more }),
            ) => {
                if job.parts.try_push(text.clone()).is_err() {
                    valid = false;
                } else if *more && job.parts.as_slice().len() < 4 {
                    job.phase = Phase::Continue(handle);
                    job.due = Instant::now();
                } else {
                    finish = Some(Summary::Ready {
                        parts: job.parts.clone(),
                        partial: *more,
                    });
                }
            }
            (
                Phase::Start(_) | Phase::Continue(_),
                _,
                Some(wire::MessagingResult::SummaryEmpty),
            ) => {
                finish = Some(if job.parts.as_slice().is_empty() {
                    Summary::Empty
                } else {
                    Summary::Ready {
                        parts: job.parts.clone(),
                        partial: false,
                    }
                });
            }
            (Phase::Status(_), messaging::Work::SummaryStatus, None)
                if matches!(
                    &reply,
                    Err(IdentityError::Request(
                        ErrorKind::Transport
                            | ErrorKind::Deadline
                            | ErrorKind::RateLimited
                            | ErrorKind::ServerUnavailable
                    ))
                ) && Instant::now() < job.deadline
                    && job.failures < 5 =>
            {
                job.failures += 1;
                job.due = Instant::now() + Duration::from_secs((1u64 << job.failures).min(30));
            }
            _ => {
                valid = false;
            }
        }
        job.in_flight = false;
        if !valid {
            finish = Some(Summary::Failed);
        } else if finish.is_none() {
            self.history.set_summary(
                context,
                Summary::Pending {
                    parts: job.parts.clone(),
                },
            );
        }
        if let Some(value) = finish {
            self.summary.job = None;
            self.history.set_summary(context, value);
        }
        self.touch();
        if result.is_some() || reply.is_err() {
            reply
        } else {
            Err(IdentityError::Request(ErrorKind::Malformed))
        }
    }
}
