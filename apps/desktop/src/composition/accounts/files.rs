//! Native selection is scoped before and after the OS dialog. File bytes stay in the adapter.
use super::*;
use std::path::PathBuf;
use talk_core::protocol::messaging::MessagingCommand;
use talk_core::ui::{content::DownloadView, messaging::MessageFeature};
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
        if self.picker.is_some() || self.picker_open || self.file_work_pending(context) {
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
        self.finish_file_selection(picker, result);
    }
    fn file_work_pending(&self, context: ConversationRef) -> bool {
        self.content_owner.transfer.is_some()
            || self.pending.iter().any(|pending| {
                matches!(pending.purpose,
                    Purpose::Message(_, current, _, messaging::Work::Action) if current == context)
            })
    }
    // Scope is checked again when the native dialog returns. Keep the old
    // transfer intact until start_message has admitted this exact request.
    fn finish_file_selection(&mut self, picker: Picker, result: Result<Option<PathBuf>, ()>) {
        let outcome = self.submit_file_selection(&picker, result);
        if let Err(error) = outcome {
            let reason = match error {
                SubmitError::Unavailable(reason) => reason,
                _ => UnavailableReason::PolicyStale,
            };
            // A picker failure belongs to that picker, never to another active
            // download's progress, cancellation handle, or message action.
            self.snapshot.content_panel.result = Some((picker.request, Completion::Failed(reason)));
            self.touch();
        }
    }
    fn submit_file_selection(
        &mut self,
        picker: &Picker,
        result: Result<Option<PathBuf>, ()>,
    ) -> Result<(), SubmitError> {
        let Some(path) =
            result.map_err(|_| SubmitError::Unavailable(UnavailableReason::Unconfigured))?
        else {
            return Ok(());
        };
        if self.file_work_pending(picker.context) {
            return Err(SubmitError::Busy);
        }
        let local = path
            .to_str()
            .and_then(|p| Text::new(p).ok())
            .ok_or(SubmitError::Unavailable(UnavailableReason::Unsupported))?;
        let mut upload_name = None;
        let mut download = None;
        let command = match &picker.kind {
            Kind::Upload => {
                let name = path
                    .file_name()
                    .and_then(|n| n.to_str())
                    .and_then(|n| Text::new(n).ok())
                    .ok_or(SubmitError::Unavailable(UnavailableReason::Unsupported))?;
                upload_name = Some(name.clone());
                MessagingCommand::UploadFile {
                    local_path: local,
                    name,
                }
            }
            Kind::Download {
                message,
                placeholder,
                name,
            } => {
                let progress = protocol::content::TransferProgress::default();
                let id = protocol::MessageId {
                    conversation: picker.context.conversation,
                    sequence: u64::try_from(*message).map_err(|_| SubmitError::StaleScope)?,
                };
                download = Some((
                    progress.clone(),
                    DownloadView {
                        request: picker.request,
                        attachment: talk_core::ui::content::AttachmentRef {
                            context: picker.context,
                            message: *message,
                            placeholder: placeholder.clone(),
                        },
                        name: name.clone(),
                        received: 0,
                        total: None,
                        state: talk_core::ui::content::DownloadState::Transferring,
                    },
                ));
                MessagingCommand::DownloadFile {
                    progress,
                    message: id,
                    placeholder: placeholder.clone(),
                    destination: local,
                }
            }
        };
        self.start_message(
            picker.request,
            picker.context,
            command,
            messaging::Work::Action,
        )?;
        if let Some(name) = upload_name
            && let Some(panel) = self.snapshot.message_panel.as_mut()
        {
            panel.upload_name = Some(name);
        }
        if let Some((progress, view)) = download {
            self.content_owner.transfer = Some(progress);
            self.snapshot.content_panel.transfer = Some(view);
        }
        self.touch();
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use talk_core::protocol::content::TransferProgress;
    use talk_core::ui::content::{AttachmentRef, DownloadState};

    fn active_download() -> (Accounts, Picker, TransferProgress) {
        let mut model = super::super::tests::model();
        let context = super::super::super::fixtures::context(1);
        let progress = TransferProgress::default();
        progress.update(12, Some(100));
        let picker = Picker {
            request: RequestId(22),
            context,
            kind: Kind::Download {
                message: 44,
                placeholder: text("file"),
                name: text("second.png"),
            },
        };
        model.content_owner.transfer = Some(progress.clone());
        model.snapshot.content_panel.transfer = Some(DownloadView {
            request: RequestId(21),
            attachment: AttachmentRef {
                context,
                message: 43,
                placeholder: text("file"),
            },
            name: text("first.png"),
            received: 12,
            total: Some(100),
            state: DownloadState::Transferring,
        });
        model.pending.push(Pending {
            id: ConnectionRequestId(7),
            purpose: Purpose::Message(
                RequestId(21),
                context,
                OperationId::from_local(8),
                messaging::Work::Action,
            ),
        });
        (model, picker, progress)
    }

    #[test]
    fn second_picker_and_failed_upload_preserve_first_download_owner() {
        for upload in [false, true] {
            let (mut model, mut picker, progress) = active_download();
            let original = model.snapshot.content_panel.transfer.clone();
            if upload {
                picker.kind = Kind::Upload;
            }
            assert!(model.file_work_pending(picker.context));
            // Exercise the same owner completion used after native scope checks.
            model.finish_file_selection(picker.clone(), Ok(Some(PathBuf::from("/tmp/second.png"))));
            assert_eq!(model.snapshot.content_panel.transfer, original);
            assert_eq!(model.content_owner.transfer.as_ref(), Some(&progress));
            assert_eq!(model.pending.len(), 1);
            assert!(matches!(
                model.snapshot.content_panel.result,
                Some((RequestId(22), Completion::Failed(_)))
            ));
            model.finish_file_selection(picker, Err(()));
            assert_eq!(model.snapshot.content_panel.transfer, original);
            assert_eq!(model.content_owner.transfer.as_ref(), Some(&progress));
            model.cancel_download();
            assert!(progress.is_cancelled());
            assert_eq!(progress.snapshot(), (12, Some(100), false));
            assert!(!progress.is_published());
        }
    }

    #[test]
    fn rejected_admission_never_installs_new_transfer_or_upload_name() {
        let (mut model, picker, _) = active_download();
        model.content_owner.transfer = None;
        model.pending.clear();
        let original = model.snapshot.content_panel.transfer.clone();
        // The test model has no account connection, so start_message rejects.
        model.finish_file_selection(picker, Ok(Some(PathBuf::from("/tmp/second.png"))));
        assert!(model.content_owner.transfer.is_none());
        assert_eq!(model.snapshot.content_panel.transfer, original);
    }
}
