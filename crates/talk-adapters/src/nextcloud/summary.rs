//! Bind Talk summary tasks to the room and operation which created them.
//! A lost POST is never repeated automatically; only status reads are retried.
use super::*;
use nextcloud_talk_api as api;
use std::time::{Duration, Instant};
use talk_core::protocol::messaging::{MessagingCommand, MessagingResult};

enum State {
    Starting,
    Waiting {
        task: u64,
        from: u64,
        next: Option<u64>,
    },
    Ready {
        next: u64,
    },
    Done,
}

pub(super) struct Binding {
    conversation: ConversationId,
    handle: OperationId,
    expires: Instant,
    state: State,
}

impl NextcloudClient {
    pub(super) async fn summary(
        &mut self,
        context: &OperationContext,
        conversation: ConversationId,
        operation: OperationId,
        command: MessagingCommand,
    ) -> Result<MessagingResult, PortError> {
        use MessagingCommand as C;
        use MessagingResult as R;
        self.summaries
            .retain(|binding| binding.expires > Instant::now());
        match command {
            C::Summarize { from } => {
                if from.conversation != conversation {
                    return Err(self.error(ErrorKind::WrongScope));
                }
                let offset = from.sequence;
                if self.summaries.iter().any(|item| item.handle == operation) {
                    return Err(self.error(ErrorKind::Conflict));
                }
                if self.summaries.len() == 8 {
                    return Err(self.error(ErrorKind::Busy));
                }
                self.summaries.push(Binding {
                    conversation,
                    handle: operation,
                    expires: Instant::now() + Duration::from_secs(600),
                    state: State::Starting,
                });
                self.start_summary_part(context, conversation, operation, offset)
                    .await
            }
            C::SummaryContinue { handle } => {
                let binding = self
                    .summaries
                    .iter_mut()
                    .find(|item| item.handle == handle && item.conversation == conversation)
                    .ok_or_else(|| self.transport.error(ErrorKind::WrongScope))?;
                let State::Ready { next } = binding.state else {
                    return Err(self.error(ErrorKind::Conflict));
                };
                // Retire the continuation before dispatch. A lost acknowledgement
                // cannot authorize a duplicate provider request.
                binding.state = State::Starting;
                self.start_summary_part(context, conversation, handle, next)
                    .await
            }
            C::SummaryStatus { handle } => {
                let binding = self
                    .summaries
                    .iter()
                    .find(|item| item.handle == handle && item.conversation == conversation)
                    .ok_or_else(|| self.error(ErrorKind::WrongScope))?;
                let State::Waiting { task, from, next } = binding.state else {
                    return Err(self.error(ErrorKind::Conflict));
                };
                let token = self.check_room(conversation)?.to_owned();
                let api::Reply::TaskProcessing(result) = self
                    .transport
                    .api_request(context, api::Request::TaskProcessing { id: task })
                    .await?
                else {
                    return Err(self.error(ErrorKind::Malformed));
                };
                if result.id != task
                    || result.task_type != "core:text2text:summary"
                    || result.app != "spreed"
                    || result.user.as_deref() != Some(self.transport.user())
                    || result.custom_id.as_deref() != Some(format!("summary/{token}").as_str())
                {
                    return Err(self.error(ErrorKind::WrongScope));
                }
                match result.state {
                    api::task_processing::State::Pending => Ok(R::SummaryPending),
                    api::task_processing::State::Failed => {
                        if let Some(item) =
                            self.summaries.iter_mut().find(|item| item.handle == handle)
                        {
                            item.state = State::Done;
                        }
                        Err(self.error(ErrorKind::Conflict))
                    }
                    api::task_processing::State::Finished(output) => {
                        if next.is_some_and(|value| value <= from) {
                            return Err(self.error(ErrorKind::Malformed));
                        }
                        let text =
                            Text::new(&output).map_err(|_| self.error(ErrorKind::TooLarge))?;
                        let item = self
                            .summaries
                            .iter_mut()
                            .find(|item| item.handle == handle)
                            .ok_or_else(|| self.transport.error(ErrorKind::WrongScope))?;
                        item.state = next.map_or(State::Done, |next| State::Ready { next });
                        Ok(R::SummaryPart {
                            text,
                            more: next.is_some(),
                        })
                    }
                }
            }
            _ => Err(self.error(ErrorKind::Unconfigured)),
        }
    }

    async fn start_summary_part(
        &mut self,
        context: &OperationContext,
        conversation: ConversationId,
        handle: OperationId,
        from: u64,
    ) -> Result<MessagingResult, PortError> {
        let token = api::models::RoomToken::parse(self.check_room(conversation)?)
            .map_err(|kind| self.error(transport::kind(kind)))?;
        let api::Reply::Chat(api::chat::Response::Summary(task)) = self
            .transport
            .api_request(
                context,
                api::Request::Chat {
                    room: token,
                    command: api::chat::Command::Summarize { from },
                },
            )
            .await?
        else {
            return Err(self.error(ErrorKind::Malformed));
        };
        let binding = self
            .summaries
            .iter_mut()
            .find(|item| item.handle == handle && item.conversation == conversation)
            .ok_or_else(|| self.transport.error(ErrorKind::WrongScope))?;
        match task {
            Some(task) => {
                binding.state = State::Waiting {
                    task: task.task_id,
                    from,
                    next: task.next_offset,
                };
                Ok(MessagingResult::SummaryStarted { handle })
            }
            None => {
                binding.state = State::Done;
                Ok(MessagingResult::SummaryEmpty)
            }
        }
    }
}
