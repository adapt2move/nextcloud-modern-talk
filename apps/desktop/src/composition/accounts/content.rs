//! One admitted account lane for assets/polls. No detached HTTP client or persistent
//! content cache. Every completion is checked against current account and room.
use super::*;
use talk_core::protocol::content as wire;
use talk_core::ui::content as view;
use talk_core::ui::conversation::{Availability, RemoteState};

#[derive(Clone)]
struct Job {
    request: RequestId,
    account: AccountScope,
    context: Option<ConversationRef>,
    operation: OperationId,
    target: Option<view::AssetTarget>,
    poll: Option<u64>,
    mutation: bool,
    cloud: bool,
}
#[derive(Default)]
pub(super) struct ContentOwner {
    jobs: Vec<Job>,
    images: Vec<ImageJob>,
    next_key: u64,
    retained: Vec<(u64, usize)>,
    leases: Vec<(view::AssetTarget, Instant)>,
    pub(super) transfer: Option<wire::TransferProgress>,
    link: Option<(AccountScope, Text<2048>)>,
    refresh_poll: Option<(ConversationRef, u64)>,
    // One deliberate preview may wait behind the two admitted content jobs.
    preview_request: Option<(RequestId, view::AttachmentRef)>,
    profile: Option<AccountScope>,
    profile_retry_at: Option<Instant>,
    poll_at: Option<Instant>,
    capabilities_at: Option<Instant>,
    poll_index: usize,
}
#[derive(Clone)]
pub(crate) struct ImageJob {
    pub account: AccountScope,
    pub target: view::AssetTarget,
    pub key: u64,
    pub bytes: wire::ImageBytes,
}
fn local_context(target: &view::AssetTarget) -> Option<ConversationRef> {
    match target {
        view::AssetTarget::ConversationAvatar(_) => None,
        _ => target.context(),
    }
}
impl Accounts {
    pub(super) fn clear_content_assets(&mut self) {
        if let Some(progress) = self.content_owner.transfer.take() {
            progress.cancel();
        }
        if let Ok(connection) = &mut self.connection {
            for pending in &self.pending {
                if matches!(pending.purpose, Purpose::Content(_))
                    || matches!(
                        pending.purpose,
                        Purpose::Message(_, _, _, messaging::Work::Action)
                    )
                {
                    let _ = connection.cancel_request(pending.id);
                }
            }
        }
        self.content_owner.preview_request = None;
        self.content_owner.jobs.clear();
        self.content_owner.images.clear();
        self.content_owner.link = None;
        self.content_owner.refresh_poll = None;
        self.content_owner.capabilities_at = None;
        let account = self.snapshot.active_account;
        let old = &self.snapshot.content_panel;
        let assets = if old.account == account {
            page(
                old.assets
                    .as_slice()
                    .iter()
                    .filter(|a| {
                        local_context(&a.target).is_none() && a.state != view::AssetState::Loading
                    })
                    .cloned(),
            )
        } else {
            Page::empty()
        };
        let (self_name, organization) = if old.account == account {
            (old.self_name.clone(), old.organization.clone())
        } else {
            (None, None)
        };
        if old.account != account {
            self.content_owner.profile = None;
            self.content_owner.profile_retry_at = None;
        }
        self.snapshot.content_panel = view::ContentPanel {
            account,
            assets,
            self_name,
            organization,
            ..Default::default()
        };
        self.retire_content_keys();
    }
    pub(super) fn content_action(
        &mut self,
        request: RequestId,
        account: AccountScope,
        action: view::ContentAction,
    ) -> Result<(), SubmitError> {
        self.current(account)?;
        if self.snapshot.active_account != Some(account) {
            return Err(SubmitError::StaleScope);
        }
        use view::ContentAction as A;
        match action {
            A::Acknowledge => {
                self.snapshot.content_panel.result = None;
                self.touch();
                Ok(())
            }
            A::ClosePreview => {
                self.content_owner.preview_request = None;
                self.snapshot.content_panel.preview = None;
                self.touch();
                Ok(())
            }
            A::CancelDownload => {
                self.cancel_download();
                Ok(())
            }
            A::Preview(attachment) => {
                self.require_attachment(&attachment)?;
                match self.load_asset(
                    request,
                    account,
                    view::AssetTarget::Image(attachment.clone()),
                ) {
                    Ok(()) => self.content_owner.preview_request = None,
                    Err(SubmitError::Busy) => {
                        self.content_owner.preview_request = Some((request, attachment.clone()))
                    }
                    Err(error) => return Err(error),
                }
                self.snapshot.content_panel.preview = Some(attachment);
                self.touch();
                Ok(())
            }
            A::Load(target) => self.load_asset(request, account, target),
            A::OpenCloud(attachment) => {
                self.require_attachment(&attachment)?;
                let message = mid(&attachment)?;
                self.start_content(
                    Job {
                        request,
                        account,
                        context: Some(attachment.context),
                        operation: OperationId::from_local(0),
                        target: None,
                        poll: None,
                        mutation: false,
                        cloud: true,
                    },
                    wire::ContentCommand::OpenCloud {
                        message,
                        placeholder: attachment.placeholder,
                    },
                )
            }
            A::RefreshPoll(poll) => self.start_poll(request, wire::PollCommand::Read(poll)),
            A::CreatePoll(draft) => self.start_poll(
                request,
                wire::PollCommand::Create(wire::PollDraft {
                    question: draft.question,
                    options: draft.options,
                    multiple: draft.multiple,
                    hidden_results: draft.hidden_results,
                }),
            ),
            A::Vote { poll, options } => {
                self.start_poll(request, wire::PollCommand::Vote { poll, options })
            }
            A::ClosePoll(poll) => self.start_poll(request, wire::PollCommand::Close(poll)),
        }
    }
    pub(super) fn cancel_download(&mut self) {
        if let Some(progress) = &self.content_owner.transfer {
            progress.cancel();
        }
        if let (Ok(connection), Some(transfer)) =
            (&mut self.connection, &self.snapshot.content_panel.transfer)
        {
            for pending in &self.pending {
                if matches!(pending.purpose,Purpose::Message(request,context,_,messaging::Work::Action) if request==transfer.request&&context==transfer.attachment.context)
                {
                    let _ = connection.cancel_request(pending.id);
                }
            }
        }
        self.touch();
    }
    fn require_attachment(&self, attachment: &view::AttachmentRef) -> Result<(), SubmitError> {
        if self.snapshot.selected != Some(attachment.context) {
            return Err(SubmitError::StaleScope);
        }
        self.current(attachment.context.account)
    }
    fn load_asset(
        &mut self,
        request: RequestId,
        account: AccountScope,
        target: view::AssetTarget,
    ) -> Result<(), SubmitError> {
        if target.context().is_some_and(|c| c.account != account)
            || local_context(&target).is_some_and(|c| Some(c) != self.snapshot.selected)
        {
            return Err(SubmitError::StaleScope);
        }
        if self
            .content_owner
            .jobs
            .iter()
            .any(|j| j.target.as_ref() == Some(&target))
            || self.content_owner.images.iter().any(|j| j.target == target)
        {
            return Ok(());
        }
        if self
            .snapshot
            .content_panel
            .assets
            .as_slice()
            .iter()
            .any(|a| a.target == target && matches!(a.state, view::AssetState::Ready { .. }))
        {
            return Ok(());
        }
        let command = asset_request(&target)?;
        self.start_content(
            Job {
                request,
                account,
                context: local_context(&target),
                operation: OperationId::from_local(0),
                target: Some(target.clone()),
                poll: None,
                mutation: false,
                cloud: false,
            },
            wire::ContentCommand::Asset(command),
        )?;
        self.set_asset(view::AssetView {
            target,
            state: view::AssetState::Loading,
            emoji: None,
            color: None,
            document_pages: None,
        });
        self.touch();
        Ok(())
    }
    pub(super) fn start_poll(
        &mut self,
        request: RequestId,
        command: wire::PollCommand,
    ) -> Result<(), SubmitError> {
        if self.snapshot.content == ContentState::Denied {
            return Err(SubmitError::Unavailable(
                UnavailableReason::PermissionDenied,
            ));
        }
        let context = self.snapshot.selected.ok_or(SubmitError::StaleScope)?;
        let poll = match command {
            wire::PollCommand::Read(id)
            | wire::PollCommand::Close(id)
            | wire::PollCommand::Vote { poll: id, .. } => Some(id),
            _ => None,
        };
        let mutation = matches!(
            command,
            wire::PollCommand::Create(_)
                | wire::PollCommand::Vote { .. }
                | wire::PollCommand::Close(_)
        );
        if self
            .content_owner
            .jobs
            .iter()
            .any(|j| j.target.is_none() && !j.cloud)
        {
            return Err(SubmitError::Busy);
        }
        self.start_content(
            Job {
                request,
                account: context.account,
                context: Some(context),
                operation: OperationId::from_local(0),
                target: None,
                poll,
                mutation,
                cloud: false,
            },
            wire::ContentCommand::Poll {
                conversation: context.conversation,
                command,
            },
        )?;
        self.snapshot.content_panel.context = Some(context);
        self.snapshot.content_panel.poll_state = if mutation {
            RemoteState::Saving(request)
        } else {
            RemoteState::Loading(request)
        };
        self.touch();
        Ok(())
    }
    fn start_content(
        &mut self,
        mut job: Job,
        command: wire::ContentCommand,
    ) -> Result<(), SubmitError> {
        if self.content_owner.jobs.len() >= 2 || self.content_owner.images.len() >= 2 {
            return Err(SubmitError::Busy);
        }
        let operation = self
            .connection
            .as_ref()
            .map_err(|e| submit_error(*e))?
            .new_operation_id(job.account)
            .map_err(submit_error)?;
        job.operation = operation;
        self.request(
            job.account,
            ProtocolRequest::Content { operation, command },
            Purpose::Content(operation),
        )?;
        self.content_owner.jobs.push(job);
        Ok(())
    }
    pub(super) fn complete_content(
        &mut self,
        operation: OperationId,
        result: Result<RuntimeReply, IdentityError>,
    ) {
        let Some(index) = self
            .content_owner
            .jobs
            .iter()
            .position(|j| j.operation == operation)
        else {
            return;
        };
        let job = self.content_owner.jobs.remove(index);
        if self.snapshot.active_account != Some(job.account)
            || job
                .context
                .is_some_and(|c| Some(c) != self.snapshot.selected)
            || self.current(job.account).is_err()
        {
            return;
        }
        if job.context.is_some() && self.snapshot.content == ContentState::Denied
            || job
                .target
                .as_ref()
                .is_some_and(|target| !self.asset_is_current(target))
        {
            return;
        }
        let result = match result {
            Ok(RuntimeReply::Protocol(ProtocolReply::Content {
                operation: o,
                result,
            })) if o == operation => Ok(result),
            Ok(_) => Err(IdentityError::Request(ErrorKind::Malformed)),
            Err(e) => Err(e),
        };
        match result {
            Ok(wire::ContentResult::SelfProfile { name, organization }) => {
                self.snapshot.content_panel.self_name = name;
                self.snapshot.content_panel.organization = organization;
            }
            Ok(wire::ContentResult::Image(bytes)) => {
                if let Some(target) = job.target {
                    if self.content_owner.images.len() >= 2 {
                        self.set_asset(view::AssetView {
                            target,
                            state: view::AssetState::Failed,
                            emoji: None,
                            color: None,
                            document_pages: None,
                        });
                    } else {
                        self.content_owner.next_key = self.content_owner.next_key.saturating_add(1);
                        self.content_owner.images.push(ImageJob {
                            account: job.account,
                            target,
                            key: self.content_owner.next_key,
                            bytes,
                        });
                    }
                }
            }
            Ok(wire::ContentResult::Emoji { text, color }) => {
                if let Some(target) = job.target {
                    self.set_asset(view::AssetView {
                        target,
                        state: view::AssetState::Ready { key: 0 },
                        emoji: Some(text),
                        color,
                        document_pages: None,
                    });
                }
            }
            Ok(wire::ContentResult::CloudLink(link)) if job.cloud => {
                self.content_owner.link = Some((job.account, link));
                self.snapshot.content_panel.result =
                    Some((job.request, Completion::ServerAccepted));
            }
            Ok(wire::ContentResult::PollCapabilities(availability)) => {
                self.snapshot.content_panel.poll_create = availability_view(availability);
                self.snapshot.content_panel.poll_state = RemoteState::Idle;
                self.content_owner.capabilities_at = Some(Instant::now() + Duration::from_secs(30));
            }
            Ok(wire::ContentResult::Poll(poll)) => {
                let mut values = self.snapshot.content_panel.polls.as_slice().to_vec();
                values.retain(|p| p.id != poll.id);
                if values.len() >= 20 {
                    values.remove(0);
                }
                values.push(poll_view(*poll));
                self.snapshot.content_panel.polls = page(values);
                self.snapshot.content_panel.poll_state = RemoteState::Idle;
                if job.mutation {
                    self.snapshot.content_panel.result =
                        Some((job.request, Completion::ServerAccepted));
                    self.message_poll_at = Instant::now();
                }
            }
            Ok(_) => self.content_failure(&job, IdentityError::Request(ErrorKind::Malformed)),
            Err(error) => self.content_failure(&job, error),
        }
        self.touch();
    }
    fn content_failure(&mut self, job: &Job, error: IdentityError) {
        if job.context.is_none() && job.target.is_none() && !job.cloud {
            self.content_owner.profile = None;
            self.content_owner.profile_retry_at = Some(Instant::now() + Duration::from_secs(30));
            return;
        }
        if let Some(target) = job.target.clone() {
            let state = match error {
                IdentityError::Request(
                    ErrorKind::PermissionDenied
                    | ErrorKind::WrongScope
                    | ErrorKind::ReauthenticationRequired
                    | ErrorKind::Cancelled,
                ) => view::AssetState::Denied,
                IdentityError::Request(ErrorKind::NotFound) => view::AssetState::Missing,
                IdentityError::Request(ErrorKind::Unsupported | ErrorKind::TooLarge) => {
                    view::AssetState::Unsupported
                }
                _ => view::AssetState::Failed,
            };
            self.set_asset(view::AssetView {
                target,
                state,
                emoji: None,
                color: None,
                document_pages: None,
            });
        } else {
            let completion = completion_error(error);
            self.snapshot.content_panel.result = Some((job.request, completion));
            if job.cloud {
                return;
            }
            if job.poll.is_none() && !job.mutation {
                self.snapshot.content_panel.poll_create = Availability::UNKNOWN;
                self.content_owner.capabilities_at = Some(Instant::now() + Duration::from_secs(30));
            }
            self.snapshot.content_panel.poll_state = if completion == Completion::Uncertain {
                RemoteState::Uncertain(job.request)
            } else {
                RemoteState::Failed(job.request, error_reason(error))
            };
            if job.mutation
                && let (Some(context), Some(poll)) = (job.context, job.poll)
            {
                self.content_owner.refresh_poll = Some((context, poll));
            }
        }
    }
    fn set_asset(&mut self, asset: view::AssetView) {
        let mut assets = self.snapshot.content_panel.assets.as_slice().to_vec();
        assets.retain(|a| a.target != asset.target);
        if assets.len() >= 128 {
            assets.remove(0);
        }
        self.content_owner
            .leases
            .retain(|(target, _)| target != &asset.target);
        if matches!(asset.state, view::AssetState::Ready { .. }) {
            self.content_owner
                .leases
                .push((asset.target.clone(), Instant::now()));
        }
        assets.push(asset);
        self.snapshot.content_panel.assets = page(assets);
        self.retire_content_keys();
    }
    fn retire_content_keys(&mut self) {
        let assets = &self.snapshot.content_panel.assets;
        self.content_owner.retained.retain(|(key, _)| {
            assets
                .as_slice()
                .iter()
                .any(|a| a.state == (view::AssetState::Ready { key: *key }))
        });
        self.content_owner
            .leases
            .retain(|(target, _)| assets.as_slice().iter().any(|a| &a.target == target));
    }
    pub(crate) fn take_content_image(&mut self) -> Option<ImageJob> {
        if self.content_owner.images.is_empty() {
            None
        } else {
            Some(self.content_owner.images.remove(0))
        }
    }
    pub(crate) fn finish_content_image(&mut self, job: &ImageJob, bytes: Option<usize>) -> bool {
        if self.snapshot.active_account != Some(job.account)
            || local_context(&job.target).is_some_and(|c| Some(c) != self.snapshot.selected)
            || self.current(job.account).is_err()
        {
            return false;
        }
        if !self.asset_is_current(&job.target)
            || !self
                .snapshot
                .content_panel
                .assets
                .as_slice()
                .iter()
                .any(|a| a.target == job.target && a.state == view::AssetState::Loading)
        {
            return false;
        }
        let mut admitted = false;
        if let Some(bytes) = bytes.filter(|n| *n <= 8 * 1024 * 1024) {
            while self
                .content_owner
                .retained
                .iter()
                .map(|(_, n)| *n)
                .sum::<usize>()
                + bytes
                > 32 * 1024 * 1024
            {
                if self.content_owner.retained.is_empty() {
                    break;
                }
                let (key, _) = self.content_owner.retained.remove(0);
                self.snapshot.content_panel.assets = page(
                    self.snapshot
                        .content_panel
                        .assets
                        .as_slice()
                        .iter()
                        .filter(|a| a.state != (view::AssetState::Ready { key }))
                        .cloned(),
                );
            }
            self.content_owner.retained.push((job.key, bytes));
            admitted = true;
        }
        self.set_asset(view::AssetView {
            target: job.target.clone(),
            state: if admitted {
                view::AssetState::Ready { key: job.key }
            } else {
                view::AssetState::Unsupported
            },
            emoji: None,
            color: None,
            document_pages: if admitted {
                job.bytes.document_pages()
            } else {
                None
            },
        });
        self.touch();
        admitted
    }
    pub(crate) fn take_cloud_link(&mut self) -> Option<Text<2048>> {
        let (scope, link) = self.content_owner.link.take()?;
        (self.snapshot.active_account == Some(scope) && self.current(scope).is_ok()).then_some(link)
    }
    fn asset_is_current(&self, target: &view::AssetTarget) -> bool {
        if self.snapshot.content == ContentState::Denied
            && target
                .context()
                .is_some_and(|context| Some(context) == self.snapshot.selected)
        {
            return false;
        }
        let (context, id, attachment) = match target {
            view::AssetTarget::Thumbnail(a) | view::AssetTarget::Image(a) => {
                (a.context, a.message, true)
            }
            view::AssetTarget::MessageAvatar { context, message } => (*context, *message, false),
            _ => return true,
        };
        if self.snapshot.selected != Some(context) {
            return false;
        }
        if attachment
            && !self
                .snapshot
                .conversations
                .as_slice()
                .iter()
                .any(|r| r.context == context && r.draft_clipboard == Decision::Allowed)
        {
            return false;
        }
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_secs())
            .unwrap_or(u64::MAX);
        let valid = |m: &&talk_core::ui::presentation::MessageView| {
            m.local_id == id
                && !m.metadata.deleted
                && m.metadata.expires_at.is_none_or(|at| at == 0 || at > now)
        };
        self.snapshot
            .messages
            .as_slice()
            .iter()
            .find(valid)
            .is_some()
            || self.snapshot.message_panel.as_ref().is_some_and(|p| {
                p.shared_files.as_slice().iter().find(valid).is_some()
                    || p.thread_messages.as_slice().iter().find(valid).is_some()
                    || p.pinned_messages.as_slice().iter().find(valid).is_some()
            })
    }
    pub(super) fn reconcile_content_assets(&mut self) {
        if self
            .snapshot
            .content_panel
            .preview
            .as_ref()
            .is_some_and(|attachment| {
                !self.asset_is_current(&view::AssetTarget::Image(attachment.clone()))
            })
        {
            self.snapshot.content_panel.preview = None;
            self.touch();
        }
        if self.content_owner.transfer.is_some()
            && self
                .snapshot
                .content_panel
                .transfer
                .as_ref()
                .is_some_and(|transfer| {
                    !self.asset_is_current(&view::AssetTarget::Image(transfer.attachment.clone()))
                })
            && let Some(account) = self.snapshot.active_account
        {
            let _ = self.content_action(RequestId(0), account, view::ContentAction::CancelDownload);
        }
        let previous = self.snapshot.content_panel.assets.as_slice().len();
        self.snapshot.content_panel.assets = page(
            self.snapshot
                .content_panel
                .assets
                .as_slice()
                .iter()
                .filter(|asset| {
                    self.asset_is_current(&asset.target)
                        && self
                            .content_owner
                            .leases
                            .iter()
                            .find(|(target, _)| target == &asset.target)
                            .is_none_or(|(_, at)| at.elapsed() < Duration::from_secs(60))
                })
                .cloned(),
        );
        self.retire_content_keys();
        if previous != self.snapshot.content_panel.assets.as_slice().len() {
            self.touch();
        }
        if self.snapshot.content == ContentState::Denied {
            self.snapshot.content_panel.polls = Page::empty();
            self.snapshot.content_panel.poll_create = Availability::UNKNOWN;
        }
    }
    pub(super) fn tick_content(&mut self) {
        self.reconcile_content_assets();
        if let Some((request, attachment)) = self.content_owner.preview_request.clone() {
            if self.snapshot.content_panel.preview.as_ref() != Some(&attachment) {
                self.content_owner.preview_request = None;
            } else {
                match self.load_asset(
                    request,
                    attachment.context.account,
                    view::AssetTarget::Image(attachment.clone()),
                ) {
                    Err(SubmitError::Busy) => {}
                    Ok(()) => self.content_owner.preview_request = None,
                    Err(_) => {
                        self.content_owner.preview_request = None;
                        self.set_asset(view::AssetView {
                            target: view::AssetTarget::Image(attachment),
                            state: view::AssetState::Failed,
                            emoji: None,
                            color: None,
                            document_pages: None,
                        });
                        self.touch();
                    }
                }
            }
        }
        if let Some(account) = self.snapshot.active_account
            && self.content_owner.profile != Some(account)
            && self
                .content_owner
                .profile_retry_at
                .is_none_or(|at| at <= Instant::now())
        {
            let job = Job {
                request: RequestId(0),
                account,
                context: None,
                operation: OperationId::from_local(0),
                target: None,
                poll: None,
                mutation: false,
                cloud: false,
            };
            if self
                .start_content(job, wire::ContentCommand::SelfProfile)
                .is_ok()
            {
                self.content_owner.profile = Some(account);
            }
        }
        if self.snapshot.selected.is_some()
            && self
                .content_owner
                .capabilities_at
                .is_none_or(|at| at <= Instant::now())
            && matches!(
                self.snapshot.content_panel.poll_state,
                RemoteState::Idle | RemoteState::Failed(..)
            )
        {
            let _ = self.start_poll(RequestId(0), wire::PollCommand::Capabilities);
        }
        if self
            .content_owner
            .poll_at
            .is_none_or(|at| at <= Instant::now())
        {
            self.content_owner.poll_at = Some(Instant::now() + Duration::from_secs(5));
            let polls = self.snapshot.content_panel.polls.as_slice();
            if !polls.is_empty() {
                let id = polls[self.content_owner.poll_index % polls.len()].id;
                self.content_owner.poll_index = self.content_owner.poll_index.wrapping_add(1);
                let _ = self.start_poll(RequestId(0), wire::PollCommand::Read(id));
            }
        }
        if let Some(progress) = &self.content_owner.transfer {
            let (received, total, committing) = progress.snapshot();
            if let Some(transfer) = self.snapshot.content_panel.transfer.as_mut() {
                let state = if committing {
                    view::DownloadState::Committing
                } else {
                    view::DownloadState::Transferring
                };
                if transfer.received != received
                    || transfer.total != total
                    || transfer.state != state
                {
                    transfer.received = received;
                    transfer.total = total;
                    transfer.state = state;
                    self.touch();
                }
            }
        }
        if let Some((context, poll)) = self.content_owner.refresh_poll
            && (self.snapshot.selected != Some(context)
                || self
                    .start_poll(RequestId(0), wire::PollCommand::Read(poll))
                    .is_ok())
        {
            self.content_owner.refresh_poll = None;
        }
    }
}
fn mid(a: &view::AttachmentRef) -> Result<protocol::MessageId, SubmitError> {
    Ok(protocol::MessageId {
        conversation: a.context.conversation,
        sequence: u64::try_from(a.message).map_err(|_| SubmitError::StaleScope)?,
    })
}
fn asset_request(target: &view::AssetTarget) -> Result<wire::AssetRequest, SubmitError> {
    use view::AssetTarget as V;
    use wire::AssetRequest as W;
    Ok(match target {
        V::SelfAvatar => W::SelfAvatar,
        V::ConversationAvatar(c) => W::ConversationAvatar(c.conversation),
        V::MessageAvatar { context, message } => W::MessageAvatar(protocol::MessageId {
            conversation: context.conversation,
            sequence: u64::try_from(*message).map_err(|_| SubmitError::StaleScope)?,
        }),
        V::ParticipantAvatar {
            context,
            participant,
        } => W::ParticipantAvatar(protocol::conversation::MemberId {
            conversation: context.conversation,
            local: *participant,
        }),
        V::PersonAvatar(person) => W::PersonAvatar(protocol::conversation::RecipientId {
            scope: person.account,
            local: person.local,
        }),
        V::Thumbnail(a) | V::Image(a) => W::Attachment {
            message: mid(a)?,
            placeholder: a.placeholder.clone(),
            large: matches!(target, V::Image(_)),
        },
    })
}
fn availability_view(a: protocol::conversation::ActionAvailability) -> Availability {
    Availability {
        implemented: a.implemented,
        server: support(a.server),
        allowed: a.allowed,
    }
}
fn poll_view(p: wire::Poll) -> view::PollView {
    view::PollView {
        id: p.id,
        question: p.question,
        options: page(p.options.as_slice().iter().map(|o| view::PollOption {
            id: o.id,
            label: o.label.clone(),
            votes: o.votes,
            own: o.own,
        })),
        closed: p.closed,
        hidden_results: p.hidden_results,
        max_votes: p.max_votes,
        voters: p.voters,
        details: page(p.details.as_slice().iter().map(|v| view::PollVoter {
            name: v.name.clone(),
            option: v.option,
        })),
        details_complete: p.details_complete,
        vote: availability_view(p.vote),
        close: availability_view(p.close),
    }
}
fn error_reason(error: IdentityError) -> UnavailableReason {
    match submit_error(error) {
        SubmitError::Unavailable(reason) => reason,
        _ => UnavailableReason::PolicyStale,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn revoked_room_retires_preview_polls_and_room_pixels() {
        let mut model = super::super::tests::model();
        model.snapshot = super::super::super::fixtures::snapshot(Some("chat-content"));
        let target = model
            .snapshot
            .content_panel
            .assets
            .as_slice()
            .iter()
            .find_map(|asset| {
                if let view::AssetTarget::Image(attachment) = &asset.target {
                    Some(attachment.clone())
                } else {
                    None
                }
            })
            .unwrap();
        model.snapshot.content_panel.preview = Some(target);
        model.content_owner.retained = vec![(1, 16), (2, 32), (3, 32)];
        model.snapshot.content = ContentState::Denied;
        model.reconcile_content_assets();
        assert!(model.snapshot.content_panel.preview.is_none());
        assert!(model.snapshot.content_panel.polls.as_slice().is_empty());
        assert!(
            model
                .snapshot
                .content_panel
                .assets
                .as_slice()
                .iter()
                .all(|asset| asset.target == view::AssetTarget::SelfAvatar)
        );
        assert_eq!(model.content_owner.retained, vec![(1, 16)]);
    }

    #[test]
    fn account_change_cancels_transfer_and_drops_private_content() {
        let mut model = super::super::tests::model();
        model.snapshot = super::super::super::fixtures::snapshot(Some("chat-content"));
        model.content_owner.retained = vec![(1, 16), (2, 32)];
        let progress = wire::TransferProgress::default();
        model.content_owner.transfer = Some(progress.clone());
        let mut next = model.snapshot.active_account.unwrap();
        next.account = AccountId::from_local(22);
        model.snapshot.active_account = Some(next);
        model.clear_content_assets();
        assert!(progress.is_cancelled());
        assert_eq!(model.snapshot.content_panel.account, Some(next));
        assert!(model.snapshot.content_panel.assets.as_slice().is_empty());
        assert!(model.snapshot.content_panel.self_name.is_none());
        assert!(model.content_owner.retained.is_empty());
    }
}
