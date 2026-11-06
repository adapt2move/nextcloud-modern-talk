//! One selected-room batch (eight paths, thirty minutes) and one native export.
//! Upload/share writes are sequential; uncertain writes are never replayed.
mod previews;
mod projection;
#[cfg(test)]
mod tests;
use super::*;
use talk_core::protocol::messaging::{MessagingCommand as Command, MessagingResult as Reply};
use talk_core::ui::content::{AttachmentRef, DownloadState, FileExportView};
use talk_core::ui::messaging::{ComposeTarget, RichObject, UploadFailure, UploadState, UploadView};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(in crate::composition::accounts) enum Step {
    Upload,
    Share,
    Export,
}
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(in crate::composition::accounts) struct Work {
    pub context: ConversationRef,
    pub operation: OperationId,
    pub file: u64,
    pub step: Step,
}
struct File {
    local: Text<16384>,
    remote: Option<Text<16384>>,
    existing: bool,
    view: UploadView,
    progress: protocol::content::TransferProgress,
    outgoing: Option<Outgoing>,
    preview_queued: bool,
}
struct Outgoing {
    request: RequestId,
    caption: Text<4096>,
    created: u64,
    operation: Option<OperationId>,
}
struct Batch {
    context: ConversationRef,
    files: Vec<File>,
    captions: Vec<(RequestId, Text<4096>)>,
    running: bool,
    created: Instant,
}
impl Batch {
    fn next(&self) -> Option<(u64, Step, Command)> {
        if !self.running {
            return None;
        }
        let file = self
            .files
            .iter()
            .find(|file| file.view.submitted && file.view.state == UploadState::Selected)?;
        let (step, command) = if let Some(path) = &file.remote {
            let caption = file
                .outgoing
                .as_ref()
                .and_then(|outgoing| {
                    self.captions
                        .iter()
                        .find(|(request, _)| *request == outgoing.request)
                })
                .map_or_else(|| text(""), |(_, caption)| caption.clone());
            (
                Step::Share,
                if file.existing {
                    Command::ShareExistingFile {
                        path: path.clone(),
                        caption,
                        options: Default::default(),
                    }
                } else {
                    Command::ShareFile {
                        path: path.clone(),
                        caption,
                    }
                },
            )
        } else {
            (
                Step::Upload,
                Command::UploadFile {
                    local_path: file.local.clone(),
                    name: file.view.name.clone(),
                    progress: file.progress.clone(),
                },
            )
        };
        Some((file.view.id, step, command))
    }
}
struct Export {
    work: Work,
    progress: protocol::content::TransferProgress,
    saved: Option<bool>,
    deadline: Instant,
}
#[derive(Default)]
pub(in crate::composition::accounts) struct Owner {
    next: u64,
    batch: Option<Batch>,
    flight: Option<Work>,
    export: Option<Export>,
}
fn allowed(snapshot: &PresentationSnapshot, context: ConversationRef) -> bool {
    snapshot.selected == Some(context)
        && snapshot.active_account == Some(context.account)
        && snapshot.content == ContentState::Ready
        && snapshot.message_panel.as_ref().is_some_and(|panel| {
            panel.context == context && panel.availability(MessageFeature::Files).enabled()
        })
}
fn file_source<'a>(
    snapshot: &'a PresentationSnapshot,
    reference: &AttachmentRef,
) -> Option<&'a RichObject> {
    let valid = |message: &&talk_core::ui::presentation::MessageView| {
        message.local_id == reference.message
            && !message.metadata.deleted
            && message
                .metadata
                .expires_at
                .is_none_or(|at| at == 0 || at > history::now_unix())
    };
    let main = snapshot
        .messages
        .as_slice()
        .iter()
        .find(|message| message.local_id == reference.message);
    let message = if let Some(main) = main {
        valid(&main).then_some(main)
    } else {
        snapshot
            .message_panel
            .as_ref()
            .filter(|panel| panel.context == reference.context)
            .into_iter()
            .flat_map(|panel| {
                panel
                    .shared_files
                    .as_slice()
                    .iter()
                    .chain(panel.thread_messages.as_slice())
                    .chain(panel.pinned_messages.as_slice())
            })
            .find(valid)
    }?;
    message
        .metadata
        .rich
        .as_slice()
        .iter()
        .find(|file| file.kind.as_str() == "file" && file.placeholder == reference.placeholder)
}
fn failure(error: IdentityError) -> UploadState {
    UploadState::Failed(match error {
        IdentityError::Request(
            ErrorKind::Uncertain
            | ErrorKind::Transport
            | ErrorKind::Deadline
            | ErrorKind::ServerUnavailable
            | ErrorKind::Malformed
            | ErrorKind::Cancelled,
        )
        | IdentityError::WorkerUnavailable => return UploadState::Uncertain,
        IdentityError::Request(ErrorKind::NotFound) => UploadFailure::Missing,
        IdentityError::Request(ErrorKind::TooLarge) => UploadFailure::TooLarge,
        IdentityError::Request(
            ErrorKind::PermissionDenied
            | ErrorKind::ReauthenticationRequired
            | ErrorKind::WrongScope,
        ) => UploadFailure::Denied,
        IdentityError::Request(ErrorKind::Unsupported) => UploadFailure::Unsupported,
        _ => UploadFailure::Connection,
    })
}
impl Accounts {
    pub(in crate::composition::accounts) fn stage_files(
        &mut self,
        context: ConversationRef,
        paths: Page<Text<16384>, 8>,
    ) -> Result<(), SubmitError> {
        if !allowed(&self.snapshot, context) {
            return Err(SubmitError::Unavailable(
                UnavailableReason::PermissionDenied,
            ));
        }
        if self
            .attachment_owner
            .batch
            .as_ref()
            .is_some_and(|batch| batch.context != context || batch.running)
        {
            return Err(SubmitError::Busy);
        }
        let mut additions = Vec::new();
        for path in paths.as_slice() {
            let native = PathBuf::from(path.as_str());
            let name = native
                .file_name()
                .and_then(|name| name.to_str())
                .and_then(|name| Text::<255>::new(name).ok())
                .filter(|name| !name.as_str().is_empty());
            let Some(name) = name.filter(|_| native.is_absolute()) else {
                return Err(SubmitError::Unavailable(UnavailableReason::Unsupported));
            };
            if additions.iter().any(|(prior, _)| prior == path)
                || self.attachment_owner.batch.as_ref().is_some_and(|batch| {
                    batch
                        .files
                        .iter()
                        .any(|file| file.local == *path && file.view.state != UploadState::Shared)
                })
            {
                continue;
            }
            additions.push((path.clone(), name));
        }
        let old = self.attachment_owner.batch.as_ref().map_or(0, |batch| {
            batch
                .files
                .iter()
                .filter(|f| f.view.state != UploadState::Shared)
                .count()
        });
        if old + additions.len() > 8
            || self
                .attachment_owner
                .next
                .checked_add(additions.len() as u64)
                .is_none()
        {
            if let Some(panel) = self.snapshot.message_panel.as_mut() {
                panel.file_error = Some(UploadFailure::TooMany);
            }
            self.touch();
            return Err(SubmitError::Busy);
        }
        if additions.is_empty() {
            return Ok(());
        }
        let batch = self.attachment_owner.batch.get_or_insert_with(|| Batch {
            context,
            files: Vec::new(),
            captions: Vec::new(),
            running: false,
            created: Instant::now(),
        });
        // Keep confirmed local pixels while the server thumbnail is pending.
        // Reclaim only the oldest confirmed fallback needed for this selection.
        while batch.files.len() + additions.len() > 8 {
            let index = batch
                .files
                .iter()
                .position(|f| f.view.state == UploadState::Shared)
                .expect("non-confirmed capacity was checked before admission");
            batch.files.remove(index).progress.cancel();
        }
        for (local, name) in additions {
            self.attachment_owner.next += 1;
            batch.files.push(File {
                local,
                remote: None,
                existing: false,
                view: UploadView {
                    id: self.attachment_owner.next,
                    name,
                    bytes: None,
                    state: UploadState::Selected,
                    submitted: false,
                    transferred: 0,
                    uploaded: false,
                    server_message: None,
                },
                progress: protocol::content::TransferProgress::default(),
                outgoing: None,
                preview_queued: false,
            });
        }
        if let Some(panel) = self.snapshot.message_panel.as_mut() {
            panel.file_error = None;
        }
        self.project_uploads();
        self.queue_upload_previews();
        Ok(())
    }
    pub(in crate::composition::accounts) fn stage_cloud_file(
        &mut self,
        context: ConversationRef,
        directory: Text<16384>,
        name: Text<255>,
    ) -> Result<(), SubmitError> {
        if !allowed(&self.snapshot, context) {
            return Err(SubmitError::Unavailable(
                UnavailableReason::PermissionDenied,
            ));
        }
        let browser = self
            .snapshot
            .content_panel
            .file_browser
            .as_ref()
            .filter(|browser| browser.account == context.account && browser.path == directory)
            .ok_or(SubmitError::StaleScope)?;
        let entry = browser
            .entries
            .as_slice()
            .iter()
            .find(|entry| entry.name == name && !entry.directory)
            .ok_or(SubmitError::StaleScope)?;
        let bytes = entry.size;
        let path = if directory.as_str() == "/" {
            format!("/{}", name.as_str())
        } else {
            format!("{}/{}", directory.as_str(), name.as_str())
        };
        let path = Text::new(&path)
            .map_err(|_| SubmitError::Unavailable(UnavailableReason::Unsupported))?;
        if self
            .attachment_owner
            .batch
            .as_ref()
            .is_some_and(|batch| batch.context != context || batch.running)
        {
            return Err(SubmitError::Busy);
        }
        let batch = self.attachment_owner.batch.get_or_insert_with(|| Batch {
            context,
            files: Vec::new(),
            captions: Vec::new(),
            running: false,
            created: Instant::now(),
        });
        if batch.files.iter().any(|file| {
            file.existing
                && file.remote.as_ref() == Some(&path)
                && file.view.state != UploadState::Shared
        }) {
            return Ok(());
        }
        if batch
            .files
            .iter()
            .filter(|file| file.view.state != UploadState::Shared)
            .count()
            >= 8
        {
            return Err(SubmitError::Busy);
        }
        while batch.files.len() >= 8 {
            let Some(index) = batch
                .files
                .iter()
                .position(|file| file.view.state == UploadState::Shared)
            else {
                return Err(SubmitError::Busy);
            };
            batch.files.remove(index).progress.cancel();
        }
        let Some(next) = self.attachment_owner.next.checked_add(1) else {
            return Err(SubmitError::Busy);
        };
        self.attachment_owner.next = next;
        batch.files.push(File {
            local: text(""),
            remote: Some(path),
            existing: true,
            view: UploadView {
                id: next,
                name,
                bytes,
                state: UploadState::Selected,
                submitted: false,
                transferred: 0,
                uploaded: true,
                server_message: None,
            },
            progress: protocol::content::TransferProgress::default(),
            outgoing: None,
            preview_queued: true,
        });
        self.project_uploads();
        Ok(())
    }
    pub(in crate::composition::accounts) fn send_files(
        &mut self,
        request: RequestId,
        context: ConversationRef,
        caption: Text<4096>,
        target: ComposeTarget,
    ) -> Result<(), SubmitError> {
        if !allowed(&self.snapshot, context) {
            return Err(SubmitError::Unavailable(
                UnavailableReason::PermissionDenied,
            ));
        }
        if target != ComposeTarget::default() {
            return Err(SubmitError::Unavailable(UnavailableReason::Unsupported));
        }
        if self.content_owner.transfer.is_some()
            || self.attachment_owner.flight.is_some()
            || self.file_export_pending()
        {
            return Err(SubmitError::Busy);
        }
        let batch = self
            .attachment_owner
            .batch
            .as_mut()
            .filter(|batch| batch.context == context)
            .ok_or(SubmitError::StaleScope)?;
        if batch.running
            || batch
                .files
                .iter()
                .any(|file| file.view.state == UploadState::Uncertain)
        {
            return Err(SubmitError::Busy);
        }
        if !batch
            .files
            .iter()
            .any(|file| !file.view.submitted && file.view.state == UploadState::Selected)
        {
            return Err(SubmitError::StaleScope);
        }
        let mut first = true;
        for file in batch
            .files
            .iter_mut()
            .filter(|file| !file.view.submitted && file.view.state == UploadState::Selected)
        {
            file.view.submitted = true;
            file.outgoing = Some(Outgoing {
                request,
                caption: if first { caption.clone() } else { text("") },
                created: history::now_unix(),
                operation: None,
            });
            first = false;
        }
        batch.captions.push((request, caption));
        batch.running = true;
        if let Some(panel) = self
            .snapshot
            .message_panel
            .as_mut()
            .filter(|p| p.context == context)
        {
            panel.files_queued = Some(request);
        }
        self.project_uploads();
        Ok(())
    }
    pub(in crate::composition::accounts) fn remove_file(
        &mut self,
        context: ConversationRef,
        id: u64,
    ) -> Result<(), SubmitError> {
        let batch = self
            .attachment_owner
            .batch
            .as_mut()
            .filter(|batch| batch.context == context)
            .ok_or(SubmitError::StaleScope)?;
        if batch.running {
            return Err(SubmitError::Busy);
        }
        batch.files.retain(|file| {
            if file.view.id == id {
                file.progress.cancel();
                false
            } else {
                true
            }
        });
        batch.captions.retain(|(request, _)| {
            batch.files.iter().any(|file| {
                file.outgoing
                    .as_ref()
                    .is_some_and(|outgoing| outgoing.request == *request)
            })
        });
        self.project_uploads();
        Ok(())
    }
    pub(in crate::composition::accounts) fn retry_file(
        &mut self,
        context: ConversationRef,
        id: u64,
    ) -> Result<(), SubmitError> {
        if !allowed(&self.snapshot, context) {
            return Err(SubmitError::Unavailable(
                UnavailableReason::PermissionDenied,
            ));
        }
        let batch = self
            .attachment_owner
            .batch
            .as_mut()
            .filter(|batch| batch.context == context)
            .ok_or(SubmitError::StaleScope)?;
        if batch.running
            || batch
                .files
                .iter()
                .any(|file| file.view.state == UploadState::Uncertain && file.remote.is_some())
        {
            return Err(SubmitError::Busy);
        }
        let file = batch
            .files
            .iter_mut()
            .find(|file| file.view.id == id)
            .ok_or(SubmitError::StaleScope)?;
        if !matches!(file.view.state, UploadState::Failed(_))
            && !(file.view.state == UploadState::Uncertain && file.remote.is_none())
        {
            return Err(SubmitError::StaleScope);
        }
        file.view.state = UploadState::Selected;
        file.progress.cancel();
        file.progress = protocol::content::TransferProgress::default();
        file.preview_queued = false;
        if let Some(outgoing) = file.outgoing.as_mut() {
            outgoing.operation = None;
        }
        batch.running = true;
        self.project_uploads();
        Ok(())
    }
    pub(in crate::composition::accounts) fn clear_attachment_work(&mut self) {
        if let Some(batch) = self.attachment_owner.batch.as_ref() {
            for file in &batch.files {
                file.progress.cancel();
            }
        }
        if let Ok(connection) = &mut self.connection {
            for pending in &self.pending {
                if matches!(pending.purpose, Purpose::Files(_)) {
                    let _ = connection.cancel_request(pending.id);
                }
            }
        }
        self.settle_file_export();
        self.attachment_owner.flight = None;
        self.attachment_owner.batch = None;
    }
    fn project_file_export(&mut self) {
        let observation = self
            .attachment_owner
            .export
            .as_ref()
            .map(|export| FileExportView {
                request: RequestId(export.work.file),
                context: export.work.context,
                saved: export.saved,
            });
        if self.snapshot.file_export != observation {
            self.snapshot.file_export = observation;
            self.touch();
        }
    }
    fn settle_file_export(&mut self) {
        if let Some(export) = self.attachment_owner.export.as_mut()
            && export.saved.is_none()
        {
            export.progress.cancel();
            export.saved = Some(export.progress.is_published());
            if let Some(view) = self
                .snapshot
                .content_panel
                .transfer
                .as_mut()
                .filter(|view| {
                    view.request == RequestId(export.work.file)
                        && view.attachment.context == export.work.context
                })
            {
                let (received, total, _) = export.progress.snapshot();
                view.received = received;
                view.total = total;
                if view.state != DownloadState::Cancelled {
                    view.state = if export.saved == Some(true) {
                        DownloadState::Saved
                    } else {
                        DownloadState::Failed(UnavailableReason::PolicyStale)
                    };
                }
            }
            self.content_owner.transfer = None;
        }
        self.project_file_export();
    }
    pub(in crate::composition::accounts) fn cancel_file_export(&mut self) -> bool {
        let Some(export) = self
            .attachment_owner
            .export
            .as_ref()
            .filter(|export| export.saved.is_none())
        else {
            return false;
        };
        let work = export.work;
        export.progress.cancel();
        if let Ok(connection) = &mut self.connection {
            for pending in &self.pending {
                if matches!(pending.purpose, Purpose::Files(pending_work) if pending_work == work) {
                    let _ = connection.cancel_request(pending.id);
                }
            }
        }
        if self.attachment_owner.flight == Some(work) {
            self.attachment_owner.flight = None;
        }
        if let Some(view) = self
            .snapshot
            .content_panel
            .transfer
            .as_mut()
            .filter(|view| {
                view.request == RequestId(work.file) && view.attachment.context == work.context
            })
        {
            view.state = if export.progress.is_published() {
                DownloadState::Saved
            } else {
                DownloadState::Cancelled
            };
        }
        self.settle_file_export();
        true
    }
    pub(in crate::composition::accounts) fn finish_file_export(
        &mut self,
        request: RequestId,
        context: ConversationRef,
    ) {
        let Some(export) = self
            .attachment_owner
            .export
            .as_ref()
            .filter(|export| export.work.file == request.0 && export.work.context == context)
        else {
            return;
        };
        let work = export.work;
        export.progress.cancel();
        if let Ok(connection) = &mut self.connection {
            for pending in &self.pending {
                if matches!(pending.purpose, Purpose::Files(pending_work) if pending_work == work) {
                    let _ = connection.cancel_request(pending.id);
                }
            }
        }
        if self.attachment_owner.flight == Some(work) {
            self.attachment_owner.flight = None;
        }
        self.content_owner.transfer = None;
        if let Some(view) = self
            .snapshot
            .content_panel
            .transfer
            .as_mut()
            .filter(|view| view.request == request && view.attachment.context == context)
            && matches!(
                view.state,
                DownloadState::Transferring | DownloadState::Committing
            )
        {
            view.state = if export.progress.is_published() {
                DownloadState::Saved
            } else {
                DownloadState::Cancelled
            };
        }
        self.attachment_owner.export = None;
        self.project_file_export();
    }
    pub(in crate::composition::accounts) fn file_export_pending(&self) -> bool {
        self.attachment_owner.export.is_some()
    }
    pub(in crate::composition::accounts) fn tick_file_work(&mut self) {
        if let Some(export) = self.attachment_owner.export.as_ref() {
            if Instant::now() >= export.deadline + Duration::from_secs(5) {
                self.finish_file_export(RequestId(export.work.file), export.work.context);
            } else if Instant::now() >= export.deadline {
                let work = export.work;
                self.settle_file_export();
                if self.attachment_owner.flight == Some(work) {
                    self.attachment_owner.flight = None;
                }
            }
        }
        if self.attachment_owner.batch.as_ref().is_some_and(|batch| {
            self.snapshot.selected != Some(batch.context)
                || batch.created.elapsed() >= Duration::from_secs(30 * 60)
        }) {
            self.clear_attachment_work();
            self.project_uploads();
        }
        self.project_uploads();
        self.queue_upload_previews();
        if self.attachment_owner.flight.is_some() || self.file_export_pending() {
            return;
        }
        let next = self.attachment_owner.batch.as_ref().and_then(|batch| {
            batch
                .next()
                .map(|(id, step, command)| (batch.context, id, step, command))
        });
        let Some((context, id, step, command)) = next else {
            if let Some(batch) = self.attachment_owner.batch.as_mut() {
                batch.running = false;
            }
            self.project_uploads();
            return;
        };
        let result = self.operation(context).and_then(|operation| {
            let work = Work {
                context,
                operation,
                file: id,
                step,
            };
            self.request(
                context.account,
                ProtocolRequest::Messaging {
                    conversation: context.conversation,
                    operation,
                    command,
                },
                Purpose::Files(work),
            )?;
            self.attachment_owner.flight = Some(work);
            if step == Step::Share
                && let Some(batch) = self.attachment_owner.batch.as_mut()
                && let Some(index) = batch.files.iter().position(|f| f.view.id == id)
                && let Some(outgoing) = batch.files[index].outgoing.as_ref()
            {
                let request = outgoing.request;
                let caption = batch
                    .captions
                    .iter()
                    .find(|(r, _)| *r == request)
                    .map(|(_, c)| c.clone());
                for file in &mut batch.files {
                    if let Some(outgoing) = file.outgoing.as_mut().filter(|o| o.request == request)
                    {
                        outgoing.caption = if file.view.id == id {
                            caption.clone().unwrap_or_else(|| text(""))
                        } else {
                            text("")
                        };
                        if file.view.id == id {
                            outgoing.operation = Some(operation);
                        }
                    }
                }
            }
            Ok(())
        });
        if result == Err(SubmitError::Busy) {
            return;
        }
        if let Some(batch) = self.attachment_owner.batch.as_mut()
            && let Some(file) = batch.files.iter_mut().find(|file| file.view.id == id)
        {
            file.view.state = match result {
                Ok(()) => {
                    if step == Step::Share {
                        UploadState::Sharing
                    } else {
                        UploadState::Uploading
                    }
                }
                Err(
                    SubmitError::StaleScope
                    | SubmitError::Unavailable(UnavailableReason::PermissionDenied),
                ) => UploadState::Failed(UploadFailure::Denied),
                Err(_) => UploadState::Failed(UploadFailure::Connection),
            };
        }
        self.project_uploads();
    }
    pub(in crate::composition::accounts) fn complete_file_work(
        &mut self,
        work: Work,
        result: Result<RuntimeReply, IdentityError>,
    ) {
        if self.attachment_owner.flight != Some(work) {
            return;
        }
        self.attachment_owner.flight = None;
        let result = result.and_then(|reply| match reply {
            RuntimeReply::Protocol(ProtocolReply::Messaging {
                conversation,
                operation,
                result,
            }) if conversation == work.context.conversation && operation == work.operation => {
                Ok(result)
            }
            _ => Err(IdentityError::Request(ErrorKind::Malformed)),
        });
        if work.step == Step::Export {
            self.settle_file_export();
            self.touch();
            return;
        }
        let Some(batch) = self
            .attachment_owner
            .batch
            .as_mut()
            .filter(|batch| batch.context == work.context)
        else {
            return;
        };
        let Some(index) = batch
            .files
            .iter()
            .position(|file| file.view.id == work.file)
        else {
            return;
        };
        match result {
            Ok(Reply::Uploaded { path, bytes }) if work.step == Step::Upload => {
                batch.files[index].remote = Some(path);
                batch.files[index].view.bytes = Some(u64::from(bytes));
                batch.files[index].view.state = UploadState::Selected;
                batch.files[index].view.uploaded = true;
            }
            Ok(Reply::Changed) if work.step == Step::Share => {
                batch.files[index].view.state = UploadState::Confirming;
                if let Some(outgoing) = batch.files[index].outgoing.as_ref()
                    && let Some(panel) = self
                        .snapshot
                        .message_panel
                        .as_mut()
                        .filter(|panel| panel.context == work.context)
                {
                    panel.files_sent = Some(outgoing.request);
                    batch
                        .captions
                        .retain(|(request, _)| *request != outgoing.request);
                }
                // The next ordinary poll discovers the canonical shared message.
                self.message_poll_at = Instant::now();
            }
            other => {
                let state = failure(
                    other
                        .err()
                        .unwrap_or(IdentityError::Request(ErrorKind::Malformed)),
                );
                batch.files[index].view.state = state;
                if matches!(
                    state,
                    UploadState::Uncertain | UploadState::Failed(UploadFailure::Denied)
                ) {
                    batch.running = false;
                }
            }
        }
        if !batch
            .files
            .iter()
            .any(|f| f.view.submitted && f.view.state == UploadState::Selected)
        {
            batch.running = false;
        }
        self.project_uploads();
    }
    pub(in crate::composition::accounts) fn export_file(
        &mut self,
        request: RequestId,
        context: ConversationRef,
        attachment: AttachmentRef,
        source: &RichObject,
        destination: Text<16384>,
    ) -> Result<(), SubmitError> {
        let admission = (|| {
            if attachment.context != context
                || !allowed(&self.snapshot, context)
                || file_source(&self.snapshot, &attachment) != Some(source)
            {
                return Err(SubmitError::StaleScope);
            }
            if self.attachment_owner.flight.is_some()
                || self.content_owner.transfer.is_some()
                || self.picker_open
                || self.picker.is_some()
                || self.file_export_pending()
            {
                return Err(SubmitError::Busy);
            }
            let path = PathBuf::from(destination.as_str());
            if !path.is_absolute() || path.file_name().is_none() {
                return Err(SubmitError::Unavailable(UnavailableReason::Unsupported));
            }
            let operation = self.operation(context)?;
            let work = Work {
                context,
                operation,
                file: request.0,
                step: Step::Export,
            };
            let progress = protocol::content::TransferProgress::default();
            self.request(
                context.account,
                ProtocolRequest::Messaging {
                    conversation: context.conversation,
                    operation,
                    command: Command::DownloadFile {
                        message: protocol::MessageId {
                            conversation: context.conversation,
                            sequence: u64::try_from(attachment.message)
                                .map_err(|_| SubmitError::StaleScope)?,
                        },
                        placeholder: attachment.placeholder.clone(),
                        destination,
                        progress: progress.clone(),
                    },
                },
                Purpose::Files(work),
            )?;
            self.attachment_owner.flight = Some(work);
            self.attachment_owner.export = Some(Export {
                work,
                saved: None,
                progress: progress.clone(),
                deadline: Instant::now() + Duration::from_secs(120),
            });
            self.content_owner.transfer = Some(progress);
            self.snapshot.content_panel.transfer = Some(DownloadView {
                request,
                attachment,
                name: Text::new(source.name.as_str()).unwrap_or_else(|_| text("download")),
                received: 0,
                total: source.size,
                state: DownloadState::Transferring,
            });
            self.touch();
            Ok(())
        })();
        self.project_file_export();
        admission
    }
}
