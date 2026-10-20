//! Native selection is scoped before and after the OS dialog. File bytes stay in the adapter.
use super::*;
use std::path::PathBuf;
use talk_core::protocol::messaging::MessagingCommand;
use talk_core::ui::{conversation::RemoteState, messaging::MessageFeature};
#[derive(Clone)]
pub(crate) enum Kind {
    Upload,
    Download {
        message: u128,
        placeholder: Text<128>,
        name: Text<255>,
    },
}
#[derive(Clone)]
pub(crate) struct Picker {
    pub request: RequestId,
    pub context: ConversationRef,
    pub kind: Kind,
}
impl Accounts {
    pub(super) fn queue_picker(
        &mut self,
        request: RequestId,
        context: ConversationRef,
        kind: Kind,
    ) -> Result<(), SubmitError> {
        self.current(context.account)?;
        if self.snapshot.selected != Some(context) {
            return Err(SubmitError::StaleScope);
        }
        if self.picker.is_some() || self.picker_open {
            return Err(SubmitError::Busy);
        }
        if !self.snapshot.message_panel.as_ref().is_some_and(|p| {
            p.context == context && p.availability(MessageFeature::Files).enabled()
        }) {
            return Err(SubmitError::Unavailable(
                UnavailableReason::PermissionDenied,
            ));
        }
        self.picker = Some(Picker {
            request,
            context,
            kind,
        });
        Ok(())
    }
    pub(crate) fn take_picker(&mut self) -> Option<Picker> {
        let p = self.picker.take()?;
        self.picker_open = true;
        Some(p)
    }
    pub(crate) fn finish_picker(&mut self, picker: Picker, result: Result<Option<PathBuf>, ()>) {
        self.picker_open = false;
        if self.snapshot.selected != Some(picker.context)
            || self.current(picker.context.account).is_err()
        {
            return;
        }
        let command = (|| {
            let Some(path) =
                result.map_err(|_| SubmitError::Unavailable(UnavailableReason::Unconfigured))?
            else {
                return Ok(None);
            };
            let local = path
                .to_str()
                .and_then(|p| Text::new(p).ok())
                .ok_or(SubmitError::Unavailable(UnavailableReason::Unsupported))?;
            Ok(Some(match picker.kind {
                Kind::Upload => {
                    let name = path
                        .file_name()
                        .and_then(|n| n.to_str())
                        .and_then(|n| Text::new(n).ok())
                        .ok_or(SubmitError::Unavailable(UnavailableReason::Unsupported))?;
                    if let Some(panel) = self.snapshot.message_panel.as_mut() {
                        panel.upload_name = Some(name.clone());
                    }
                    MessagingCommand::UploadFile {
                        local_path: local,
                        name,
                    }
                }
                Kind::Download {
                    message,
                    placeholder,
                    ..
                } => MessagingCommand::DownloadFile {
                    message: protocol::MessageId {
                        conversation: picker.context.conversation,
                        sequence: u64::try_from(message).map_err(|_| SubmitError::StaleScope)?,
                    },
                    placeholder,
                    destination: local,
                },
            }))
        })();
        let outcome = match command {
            Ok(Some(command)) => self.start_message(
                picker.request,
                picker.context,
                command,
                messaging::Work::Action,
            ),
            Ok(None) => return,
            Err(error) => Err(error),
        };
        if let Err(error) = outcome
            && let Some(panel) = self.snapshot.message_panel.as_mut()
        {
            let reason = match error {
                SubmitError::Unavailable(reason) => reason,
                _ => UnavailableReason::Unconfigured,
            };
            panel.state = RemoteState::Failed(picker.request, reason);
            self.touch();
        }
    }
}
