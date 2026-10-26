//! Native selection is scoped before and after the OS dialog. File bytes stay in the adapter.
use super::*;
use std::path::PathBuf;
use talk_adapters::local_files::{CsvRead, CsvReadError};
use talk_core::protocol::messaging::MessagingCommand;
use talk_core::ui::conversation::{CsvFileError, CsvFileOutcome, CsvFileResult, RoomSetting};
use talk_core::ui::{content::DownloadView, messaging::MessageFeature};
#[derive(Clone)]
pub(crate) enum Kind {
    Upload,
    Csv {
        edit_generation: u64,
    },
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
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct CsvSelection {
    request: RequestId,
    context: ConversationRef,
    edit_generation: u64,
}
struct CsvWork {
    selection: CsvSelection,
    reader: CsvRead,
}
/// One CSV ticket and at most one physical read, including cancelled settlement.
#[derive(Default)]
pub(super) struct CsvOwner {
    current: Option<CsvSelection>,
    dialog: Option<CsvSelection>,
    reading: Option<CsvWork>,
}
fn csv_available(snapshot: &PresentationSnapshot, context: ConversationRef) -> bool {
    snapshot.active_account == Some(context.account)
        && snapshot.selected == Some(context)
        && snapshot.conversation_panel.as_ref().is_some_and(|panel| {
            panel.context == context
                && panel.details.as_ref().is_some_and(|details| {
                    !details.classified
                        && details.availability(RoomSetting::CsvImport).enabled()
                        && details.availability(RoomSetting::Invite).enabled()
                })
        })
}
impl Accounts {
    pub(super) fn queue_csv_picker(
        &mut self,
        request: RequestId,
        context: ConversationRef,
        edit_generation: u64,
    ) -> Result<(), SubmitError> {
        self.queue_picker(request, context, Kind::Csv { edit_generation })
    }
    /// Also acknowledges a published result, so imported personal data is not
    /// retained by the snapshot after the UI consumes or closes its local form.
    pub(super) fn cancel_csv_picker(&mut self, context: ConversationRef, pick_request: RequestId) {
        if self
            .csv_file_owner
            .current
            .is_some_and(|s| s.context == context && s.request == pick_request)
        {
            self.csv_file_owner.current = None;
        }
        if self.picker.as_ref().is_some_and(|p| {
            p.context == context && p.request == pick_request && matches!(p.kind, Kind::Csv { .. })
        }) {
            self.picker = None;
        }
        if let Some(work) = &self.csv_file_owner.reading
            && work.selection.context == context
            && work.selection.request == pick_request
        {
            work.reader.cancel();
        }
        if let Some(panel) = &mut self.snapshot.conversation_panel
            && panel.context == context
            && panel
                .csv_file
                .as_ref()
                .is_some_and(|r| r.request == pick_request)
        {
            panel.csv_file = None;
            self.touch();
        }
    }
    pub(super) fn cancel_csv_file(&mut self) {
        self.csv_file_owner.current = None;
        if self
            .picker
            .as_ref()
            .is_some_and(|picker| matches!(picker.kind, Kind::Csv { .. }))
        {
            self.picker = None;
        }
        if let Some(work) = &self.csv_file_owner.reading {
            work.reader.cancel();
        }
        if let Some(panel) = &mut self.snapshot.conversation_panel {
            panel.csv_file = None;
        }
    }
    pub(super) fn csv_file_stopped(&self) -> bool {
        self.csv_file_owner
            .reading
            .as_ref()
            .is_none_or(|w| w.reader.is_finished())
    }
    pub(super) fn poll_csv_file(&mut self) {
        let Some(mut work) = self.csv_file_owner.reading.take() else {
            return;
        };
        let current = self.csv_file_owner.current == Some(work.selection);
        if !current
            || self.current(work.selection.context.account).is_err()
            || !csv_available(&self.snapshot, work.selection.context)
        {
            work.reader.cancel();
        }
        let mut result = work.reader.try_result();
        // The worker can finish between the first empty receive and this check.
        if result.is_none() && work.reader.is_finished() {
            result = work.reader.try_result();
        }
        if let Some(result) = result {
            self.publish_csv(work.selection, csv_outcome(result));
        }
        if !work.reader.is_finished() {
            self.csv_file_owner.reading = Some(work);
        }
    }
    fn publish_csv(&mut self, selection: CsvSelection, outcome: CsvFileOutcome) {
        if self.csv_file_owner.current != Some(selection) {
            return;
        }
        self.csv_file_owner.current = None;
        if self.current(selection.context.account).is_err()
            || !csv_available(&self.snapshot, selection.context)
        {
            return;
        }
        if let Some(panel) = &mut self.snapshot.conversation_panel {
            panel.csv_file = Some(CsvFileResult {
                request: selection.request,
                edit_generation: selection.edit_generation,
                outcome,
            });
            self.touch();
        }
    }
    fn finish_csv_selection(
        &mut self,
        picker: Picker,
        edit_generation: u64,
        result: Result<Option<PathBuf>, ()>,
    ) {
        let selection = CsvSelection {
            request: picker.request,
            context: picker.context,
            edit_generation,
        };
        if self.csv_file_owner.current != Some(selection) {
            return;
        }
        if self.current(picker.context.account).is_err()
            || !csv_available(&self.snapshot, picker.context)
        {
            self.csv_file_owner.current = None;
            return;
        }
        let path = match result {
            Ok(Some(path)) => path,
            Ok(None) => {
                self.publish_csv(selection, CsvFileOutcome::Cancelled);
                return;
            }
            Err(()) => {
                self.publish_csv(
                    selection,
                    CsvFileOutcome::Failed(CsvFileError::PickerUnavailable),
                );
                return;
            }
        };
        match CsvRead::start(path) {
            Ok(reader) => self.csv_file_owner.reading = Some(CsvWork { selection, reader }),
            Err(error) => self.publish_csv(selection, csv_outcome(Err(error))),
        }
    }
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
        if self.picker.is_some()
            || self.picker_open
            || self.csv_file_owner.reading.is_some()
            || self.file_work_pending(context)
        {
            return Err(SubmitError::Busy);
        }
        let permitted = if matches!(kind, Kind::Csv { .. }) {
            csv_available(&self.snapshot, context)
        } else {
            self.snapshot.message_panel.as_ref().is_some_and(|p| {
                p.context == context && p.availability(MessageFeature::Files).enabled()
            })
        };
        if !permitted {
            return Err(SubmitError::Unavailable(
                UnavailableReason::PermissionDenied,
            ));
        }
        if let Kind::Csv { edit_generation } = &kind {
            self.csv_file_owner.current = Some(CsvSelection {
                request,
                context,
                edit_generation: *edit_generation,
            });
            if let Some(panel) = &mut self.snapshot.conversation_panel {
                panel.csv_file = None;
            }
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
        if let Kind::Csv { edit_generation } = &p.kind {
            self.csv_file_owner.dialog = Some(CsvSelection {
                request: p.request,
                context: p.context,
                edit_generation: *edit_generation,
            });
        }
        self.picker_open = true;
        Some(p)
    }
    pub(crate) fn finish_picker(&mut self, picker: Picker, result: Result<Option<PathBuf>, ()>) {
        if let Kind::Csv { edit_generation } = &picker.kind {
            let edit_generation = *edit_generation;
            let selection = CsvSelection {
                request: picker.request,
                context: picker.context,
                edit_generation,
            };
            if self.csv_file_owner.dialog != Some(selection) {
                return;
            }
            self.csv_file_owner.dialog = None;
            self.picker_open = false;
            self.finish_csv_selection(picker, edit_generation, result);
            return;
        }
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
            Kind::Csv { .. } => return Err(SubmitError::StaleScope),
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

fn csv_outcome(result: Result<Text<65536>, CsvReadError>) -> CsvFileOutcome {
    use CsvReadError as E;
    match result {
        Ok(text) => CsvFileOutcome::Loaded(text),
        Err(E::Cancelled) => CsvFileOutcome::Cancelled,
        Err(error) => CsvFileOutcome::Failed(match error {
            E::Busy => CsvFileError::Unavailable(UnavailableReason::Busy),
            E::TooLarge => CsvFileError::TooLarge,
            E::InvalidUtf8 => CsvFileError::InvalidUtf8,
            E::NotRegularFile => CsvFileError::NotRegularFile,
            E::ReadFailed => CsvFileError::ReadFailed,
            E::TimedOut => CsvFileError::TimedOut,
            E::Cancelled => return CsvFileOutcome::Cancelled,
        }),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use talk_core::protocol::content::TransferProgress;
    use talk_core::ui::content::{AttachmentRef, DownloadState};

    fn csv_model() -> (Accounts, CsvSelection) {
        let mut model = super::super::tests::model();
        model.snapshot = super::super::super::fixtures::snapshot(Some("details-ready"));
        let context = model.snapshot.selected.unwrap();
        model.snapshot.active_account = Some(context.account);
        model.snapshot.message_panel = None;
        let selection = CsvSelection {
            request: RequestId(90),
            context,
            edit_generation: 7,
        };
        (model, selection)
    }
    #[test]
    fn shutdown_retires_queued_csv_without_releasing_an_open_dialog_slot() {
        for opened in [false, true] {
            let (mut model, selection) = csv_model();
            model.csv_file_owner.current = Some(selection);
            model.picker = Some(Picker {
                request: selection.request,
                context: selection.context,
                kind: Kind::Csv {
                    edit_generation: selection.edit_generation,
                },
            });
            let dialog = opened.then(|| model.take_picker().unwrap());
            model.shutdown();
            assert!(model.csv_file_owner.current.is_none());
            assert!(
                model.take_picker().is_none(),
                "shutdown cannot launch a cancelled CSV picker"
            );
            assert_eq!(model.picker_open, opened);
            if let Some(dialog) = dialog {
                model.finish_picker(dialog, Ok(None));
                assert!(
                    !model.picker_open,
                    "only the matching OS completion retires its slot"
                );
            }
        }
    }
    #[test]
    fn csv_gate_is_separate_from_upload_and_rechecks_invitation_policy() {
        use talk_core::ui::conversation::Availability;
        let (mut model, selection) = csv_model();
        assert!(csv_available(&model.snapshot, selection.context));
        assert!(model.snapshot.message_panel.is_none());
        for setting in [RoomSetting::CsvImport, RoomSetting::Invite] {
            let mut denied = model.snapshot.clone();
            let details = denied
                .conversation_panel
                .as_mut()
                .unwrap()
                .details
                .as_mut()
                .unwrap();
            details.controls = page(
                [
                    (
                        RoomSetting::CsvImport,
                        Availability {
                            implemented: true,
                            server: FeatureSupport::Supported,
                            allowed: Decision::Allowed,
                        },
                    ),
                    (
                        RoomSetting::Invite,
                        Availability {
                            implemented: true,
                            server: FeatureSupport::Supported,
                            allowed: Decision::Allowed,
                        },
                    ),
                ]
                .map(|(key, mut value)| {
                    if key == setting {
                        value.allowed = Decision::Denied;
                    }
                    (key, value)
                }),
            );
            assert!(!csv_available(&denied, selection.context));
        }
        model.snapshot.active_account = None;
        assert!(!csv_available(&model.snapshot, selection.context));
    }
    #[test]
    fn cancellation_acknowledges_only_exact_result_and_preserves_other_ticket() {
        let (mut model, selection) = csv_model();
        model.csv_file_owner.current = Some(selection);
        model.snapshot.conversation_panel.as_mut().unwrap().csv_file = Some(CsvFileResult {
            request: selection.request,
            edit_generation: selection.edit_generation,
            outcome: CsvFileOutcome::Loaded(text("a@example.test")),
        });
        model.cancel_csv_picker(selection.context, RequestId(89));
        assert_eq!(model.csv_file_owner.current, Some(selection));
        assert!(
            model
                .snapshot
                .conversation_panel
                .as_ref()
                .unwrap()
                .csv_file
                .is_some()
        );
        model.cancel_csv_picker(selection.context, selection.request);
        assert!(model.csv_file_owner.current.is_none());
        assert!(
            model
                .snapshot
                .conversation_panel
                .as_ref()
                .unwrap()
                .csv_file
                .is_none()
        );
    }
    #[test]
    fn cancelled_dialog_keeps_os_slot_and_late_reply_cannot_overwrite_new_generation() {
        let (mut model, selection) = csv_model();
        model.csv_file_owner.current = Some(selection);
        model.picker = Some(Picker {
            request: selection.request,
            context: selection.context,
            kind: Kind::Csv {
                edit_generation: selection.edit_generation,
            },
        });
        let picker = model.take_picker().unwrap();
        model.cancel_csv_picker(selection.context, selection.request);
        assert!(
            model.picker_open,
            "cancelling a ticket does not close the OS dialog"
        );
        let newer = CsvSelection {
            edit_generation: 8,
            request: RequestId(91),
            ..selection
        };
        model.csv_file_owner.current = Some(newer);
        model.finish_picker(picker.clone(), Ok(Some(PathBuf::from("unused.csv"))));
        assert!(!model.picker_open);
        assert_eq!(model.csv_file_owner.current, Some(newer));
        assert!(model.csv_file_owner.reading.is_none());
        assert!(
            model
                .snapshot
                .conversation_panel
                .as_ref()
                .unwrap()
                .csv_file
                .is_none()
        );
        model.picker_open = true;
        model.finish_picker(picker, Err(()));
        assert!(
            model.picker_open,
            "a duplicate old callback cannot release another dialog's slot"
        );
    }
    #[test]
    fn stale_success_failure_and_scope_teardown_never_restore_file_contents() {
        let (mut model, selection) = csv_model();
        let newer = CsvSelection {
            edit_generation: 8,
            ..selection
        };
        for outcome in [
            CsvFileOutcome::Loaded(text("a@example.test")),
            CsvFileOutcome::Failed(CsvFileError::ReadFailed),
        ] {
            model.csv_file_owner.current = Some(newer);
            model.publish_csv(selection, outcome);
            assert_eq!(model.csv_file_owner.current, Some(newer));
            assert!(
                model
                    .snapshot
                    .conversation_panel
                    .as_ref()
                    .unwrap()
                    .csv_file
                    .is_none()
            );
        }
        model.clear_content();
        assert!(model.csv_file_owner.current.is_none());
        model.publish_csv(selection, CsvFileOutcome::Loaded(text("a@example.test")));
        assert!(model.snapshot.conversation_panel.is_none());
    }

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
