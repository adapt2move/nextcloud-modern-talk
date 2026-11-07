//! One finite translation request; retries never create another provider write.
use super::*;
use talk_core::protocol::messaging as wire;
use talk_core::ui::messaging::MessageFeature;

struct Job {
    context: ConversationRef,
    request: RequestId,
    message: u128,
    source: Text<4096>,
    language: Text<64>,
    attempts: u8,
    due: Instant,
    deadline: Instant,
    in_flight: bool,
}

#[derive(Default)]
pub(super) struct Translations {
    job: Option<Job>,
}
impl Translations {
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
    pub(super) fn begin_translation(
        &mut self,
        request: RequestId,
        context: ConversationRef,
        message: u128,
        source: Text<4096>,
        language: Text<64>,
    ) -> Result<(), SubmitError> {
        if self.snapshot.selected != Some(context)
            || !context.is_consistent()
            || u64::try_from(message).ok().is_none_or(|id| id == 0)
        {
            return Err(SubmitError::StaleScope);
        }
        if self.translation.job.is_some() {
            return Err(SubmitError::Busy);
        }
        let panel = self
            .snapshot
            .message_panel
            .as_ref()
            .filter(|panel| panel.context == context)
            .ok_or(SubmitError::StaleScope)?;
        if !panel.availability(MessageFeature::Translation).enabled()
            || !panel.translation_targets.as_ref().is_some_and(|targets| {
                targets
                    .as_slice()
                    .iter()
                    .any(|target| target.as_str().eq_ignore_ascii_case(language.as_str()))
            })
            || !self
                .snapshot
                .messages
                .as_slice()
                .iter()
                .chain(panel.thread_root_message.iter())
                .chain(panel.thread_messages.as_slice())
                .find(|row| row.local_id == message)
                .is_some_and(|row| {
                    row.local_id == message
                        && row.body == source
                        && !row.metadata.deleted
                        && !row.metadata.system
                        && !row.metadata.truncated
                        && row.metadata.rich.as_slice().is_empty()
                })
        {
            return Err(SubmitError::StaleScope);
        }
        let now = Instant::now();
        self.translation.job = Some(Job {
            context,
            request,
            message,
            source,
            language,
            attempts: 0,
            due: now,
            deadline: now + Duration::from_secs(60),
            in_flight: false,
        });
        if let Some(panel) = self.snapshot.message_panel.as_mut() {
            panel.translation_pending = Some(message);
            panel.translation_failed = None;
        }
        self.touch();
        Ok(())
    }

    pub(super) fn tick_translations(&mut self) {
        let Some(job) = self.translation.job.as_ref() else {
            return;
        };
        if self.snapshot.selected != Some(job.context) || Instant::now() >= job.deadline {
            self.translation.job = None;
            if let Some(panel) = self.snapshot.message_panel.as_mut() {
                panel.translation_pending = None;
            }
            self.touch();
            return;
        }
        if job.in_flight || Instant::now() < job.due {
            return;
        }
        let context = job.context;
        let request = job.request;
        let message = job.message;
        let command = wire::MessagingCommand::Translate {
            message: protocol::MessageId {
                conversation: context.conversation,
                sequence: match u64::try_from(message) {
                    Ok(value) if value > 0 => value,
                    _ => {
                        self.translation.job = None;
                        return;
                    }
                },
            },
            source: job.source.clone(),
            language: job.language.clone(),
        };
        let started = self
            .start_message(
                request,
                context,
                command,
                messaging::Work::Translate(message),
            )
            .is_ok();
        if let Some(job) = self.translation.job.as_mut() {
            if started {
                job.in_flight = true;
                job.attempts = job.attempts.saturating_add(1);
            } else {
                job.due = Instant::now() + Duration::from_secs(1);
            }
        }
    }

    pub(super) fn translation_reply(
        &mut self,
        request: RequestId,
        context: ConversationRef,
        operation: OperationId,
        message: u128,
        reply: Result<RuntimeReply, IdentityError>,
    ) -> Result<RuntimeReply, IdentityError> {
        let Some(job) = self.translation.job.as_mut().filter(|job| {
            job.request == request
                && job.context == context
                && job.message == message
                && job.in_flight
        }) else {
            return Err(IdentityError::Request(ErrorKind::WrongScope));
        };
        let valid = matches!(&reply, Ok(RuntimeReply::Protocol(ProtocolReply::Messaging { conversation, operation: returned, result: wire::MessagingResult::Translation { message: id, source, language, .. } }))
            if *conversation == context.conversation && *returned == operation && id.conversation == context.conversation && u128::from(id.sequence) == message && *source == job.source && *language == job.language);
        let transient = matches!(
            &reply,
            Err(IdentityError::Request(
                ErrorKind::Transport
                    | ErrorKind::Deadline
                    | ErrorKind::RateLimited
                    | ErrorKind::ServerUnavailable
            ))
        );
        let retry = transient && job.attempts < 3 && Instant::now() < job.deadline;
        if retry {
            job.in_flight = false;
            job.due = Instant::now() + Duration::from_secs(1u64 << job.attempts);
        } else {
            self.translation.job = None;
            if let Some(panel) = self
                .snapshot
                .message_panel
                .as_mut()
                .filter(|panel| panel.context == context)
            {
                panel.translation_pending = None;
                if !valid
                    && !transient
                    && !matches!(
                        &reply,
                        Err(IdentityError::Request(
                            ErrorKind::WrongScope | ErrorKind::Cancelled
                        ))
                    )
                {
                    panel.translation_failed = Some(message);
                }
            }
            self.touch();
        }
        if valid || reply.is_err() {
            reply
        } else {
            Err(IdentityError::Request(ErrorKind::Malformed))
        }
    }
}
