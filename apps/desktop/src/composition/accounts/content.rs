//! One admitted account lane for assets/polls; no detached HTTP client. Profile
//! avatars have a separate account-owned disk cache; attachments remain transient.
//! Every completion is checked against current account and room.
mod links;
mod thumbnails;
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
    attachment: Option<talk_adapters::identity::CachedAttachment>,
    preview_generation: Option<u64>,
}
struct AssetRetry {
    request: RequestId,
    account: AccountScope,
    target: view::AssetTarget,
    deadline: Instant,
}
#[derive(Default)]
pub(super) struct ContentOwner {
    pub(super) video: super::preview::Video,
    pub(super) preview_generation: u64,
    descriptors: Vec<(view::AssetTarget, talk_adapters::identity::CachedAttachment)>,
    jobs: Vec<Job>,
    images: Vec<ImageJob>,
    pub(super) next_key: u64,
    pub(super) retained: Vec<(u64, usize)>,
    // Equal, freshly authorized avatar bytes share one GPU image. Entries exist
    // only while a live asset references that image: 128 entries / 8 MiB, never
    // attachment bytes or an independent permission/lifetime source.
    avatars: Vec<(wire::ImageBytes, u64)>,
    // UI queue admission is not HTTP admission. Busy automatic asset loads wait
    // here, at most 128 targets for 60 seconds, with one retry per owner tick.
    // Navigation/account teardown cancels these before they can start effects.
    pending_assets: Vec<AssetRetry>,
    leases: Vec<(view::AssetTarget, Instant)>,
    pub(super) transfer: Option<wire::TransferProgress>,
    link: Option<(AccountScope, Text<2048>)>,
    message_link: Option<links::Pending>,
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
    pub cached: bool,
    pub attachment: Option<talk_adapters::identity::CachedAttachment>,
    pub preview_generation: Option<u64>,
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
fn is_avatar(target: &view::AssetTarget) -> bool {
    !matches!(
        target,
        view::AssetTarget::Thumbnail(_) | view::AssetTarget::Image(_)
    )
}
impl Accounts {
    pub(super) fn clear_content_assets(&mut self) {
        self.content_owner.preview_generation =
            self.content_owner.preview_generation.saturating_add(1);
        self.stop_video();
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
        self.content_owner.pending_assets.clear();
        self.content_owner.jobs.clear();
        self.content_owner.images.clear();
        self.content_owner.link = None;
        self.content_owner.message_link = None;
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
                        a.state != view::AssetState::Loading
                            && (local_context(&a.target).is_none()
                                || self.retained_message_avatar(&a.target))
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
                self.content_owner.preview_generation =
                    self.content_owner.preview_generation.saturating_add(1);
                self.stop_video();
                self.snapshot.content_panel.assets = page(
                    self.snapshot
                        .content_panel
                        .assets
                        .as_slice()
                        .iter()
                        .filter(|a| !matches!(a.target, view::AssetTarget::Image(_)))
                        .cloned(),
                );
                self.retire_content_keys();
                self.snapshot.content_panel.playback = None;
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
                let retry = self
                    .snapshot
                    .content_panel
                    .assets
                    .as_slice()
                    .iter()
                    .any(|a| {
                        a.target == view::AssetTarget::Image(attachment.clone())
                            && matches!(
                                a.state,
                                view::AssetState::Failed
                                    | view::AssetState::Missing
                                    | view::AssetState::Unsupported
                            )
                    });
                if self.snapshot.content_panel.preview.as_ref() != Some(&attachment) || retry {
                    self.content_owner.preview_generation = self
                        .content_owner
                        .preview_generation
                        .checked_add(1)
                        .ok_or(SubmitError::Busy)?;
                    self.stop_video();
                    self.snapshot.content_panel.playback = None;
                    self.content_owner
                        .images
                        .retain(|j| !matches!(j.target, view::AssetTarget::Image(_)));
                    self.snapshot.content_panel.assets = page(
                        self.snapshot
                            .content_panel
                            .assets
                            .as_slice()
                            .iter()
                            .filter(|a| !matches!(&a.target, view::AssetTarget::Image(_)))
                            .cloned(),
                    );
                    self.retire_content_keys();
                }
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
            A::PlayPreview { .. } | A::PausePreview { .. } | A::SeekPreview { .. } => {
                self.video_control(action)
            }
            A::Load(target) => self.request_asset(request, account, target),
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
                        attachment: None,
                        preview_generation: None,
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
        if attachment.context.account.runtime.as_local() == 0
            || attachment.context.conversation.host.as_local() == 0
        {
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
        if target.context().is_some_and(|c| {
            c.account != account
                || c.account.runtime.as_local() == 0
                || c.conversation.host.as_local() == 0
        }) || local_context(&target).is_some_and(|c| Some(c) != self.snapshot.selected)
        {
            return Err(SubmitError::StaleScope);
        }
        if self.content_owner.jobs.iter().any(|j| {
            j.target.as_ref() == Some(&target)
                && (!matches!(target, view::AssetTarget::Image(_))
                    || j.preview_generation == Some(self.content_owner.preview_generation))
        }) || self.content_owner.images.iter().any(|j| j.target == target)
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
        let attachment = match &target {
            view::AssetTarget::Image(reference) | view::AssetTarget::Thumbnail(reference) => {
                self.current_attachment_descriptor(reference)
            }
            _ => None,
        };
        let command = if let view::AssetTarget::Image(reference) = &target
            && self
                .snapshot
                .messages
                .as_slice()
                .iter()
                .chain(
                    self.snapshot
                        .message_panel
                        .as_ref()
                        .into_iter()
                        .flat_map(|p| {
                            p.shared_files
                                .as_slice()
                                .iter()
                                .chain(p.thread_messages.as_slice())
                                .chain(p.pinned_messages.as_slice())
                        }),
                )
                .find(|m| m.local_id == reference.message)
                .and_then(|m| {
                    m.metadata
                        .rich
                        .as_slice()
                        .iter()
                        .find(|o| o.placeholder == reference.placeholder)
                })
                .and_then(|o| o.mime_type.as_ref())
                .is_some_and(|m| m.as_str().starts_with("video/"))
        {
            wire::ContentCommand::Video {
                message: mid(reference)?,
                placeholder: reference.placeholder.clone(),
            }
        } else {
            wire::ContentCommand::Asset(asset_request(&target)?)
        };
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
                attachment,
                preview_generation: matches!(target, view::AssetTarget::Image(_))
                    .then_some(self.content_owner.preview_generation),
            },
            command,
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
    fn request_asset(
        &mut self,
        request: RequestId,
        account: AccountScope,
        target: view::AssetTarget,
    ) -> Result<(), SubmitError> {
        if self
            .content_owner
            .pending_assets
            .iter()
            .any(|job| job.account == account && job.target == target)
        {
            return Ok(());
        }
        match self.load_asset(request, account, target.clone()) {
            Err(SubmitError::Busy) => {
                if self.content_owner.pending_assets.len() < 128 {
                    self.content_owner.pending_assets.push(AssetRetry {
                        request,
                        account,
                        target,
                        deadline: Instant::now() + Duration::from_secs(60),
                    });
                } else {
                    self.asset_retry_failed(target);
                }
                Ok(())
            }
            Err(error) => {
                // A terminal nonadmission also releases presentation dedup:
                // otherwise enqueue success could hide this target forever.
                if error != SubmitError::StaleScope {
                    self.asset_retry_failed(target);
                }
                Err(error)
            }
            Ok(()) => Ok(()),
        }
    }
    fn asset_retry_failed(&mut self, target: view::AssetTarget) {
        self.set_asset(view::AssetView {
            target,
            state: view::AssetState::Failed,
            emoji: None,
            color: None,
            document_pages: None,
        });
        self.touch();
    }
    fn retry_content_asset(&mut self, now: Instant) {
        let pending = std::mem::take(&mut self.content_owner.pending_assets);
        for job in pending {
            if self.snapshot.active_account != Some(job.account)
                || local_context(&job.target).is_some_and(|c| Some(c) != self.snapshot.selected)
                || !self.asset_is_current(&job.target)
            {
                continue;
            }
            if self
                .snapshot
                .content_panel
                .assets
                .as_slice()
                .iter()
                .any(|a| {
                    a.target == job.target && matches!(a.state, view::AssetState::Ready { .. })
                })
                || self
                    .content_owner
                    .jobs
                    .iter()
                    .any(|j| j.target.as_ref() == Some(&job.target))
                || self
                    .content_owner
                    .images
                    .iter()
                    .any(|j| j.target == job.target)
            {
                continue;
            }
            if now >= job.deadline {
                self.asset_retry_failed(job.target);
            } else {
                self.content_owner.pending_assets.push(job);
            }
        }
        if !self.content_owner.pending_assets.is_empty() {
            let job = self.content_owner.pending_assets.remove(0);
            match self.load_asset(job.request, job.account, job.target.clone()) {
                Err(SubmitError::Busy) => self.content_owner.pending_assets.insert(0, job),
                Ok(()) => (),
                Err(_) => self.asset_retry_failed(job.target),
            }
        }
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
                attachment: None,
                preview_generation: None,
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
        if matches!(job.target, Some(view::AssetTarget::Image(_)))
            && job.preview_generation != Some(self.content_owner.preview_generation)
        {
            return;
        }
        if matches!(&job.target,Some(view::AssetTarget::Image(reference)) if self.snapshot.content_panel.preview.as_ref()!=Some(reference))
        {
            return;
        }
        if let (Some(captured), Some(target)) = (&job.attachment, &job.target)
            && let view::AssetTarget::Image(reference) | view::AssetTarget::Thumbnail(reference) =
                target
            && self.current_attachment_descriptor(reference).as_ref() != Some(captured)
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
            Ok(wire::ContentResult::Video(bytes)) => {
                if let Some(view::AssetTarget::Image(reference)) = job.target
                    && let (Some(attachment), Some(generation)) =
                        (job.attachment, job.preview_generation)
                {
                    self.stage_video(reference, bytes, attachment, generation);
                }
            }
            Ok(wire::ContentResult::Image(bytes)) => {
                if let Some(target) = job.target {
                    self.receive_image(
                        job.account,
                        target,
                        bytes,
                        job.attachment,
                        job.preview_generation,
                    );
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
    // Called only after the protocol completion's account/room checks above.
    // Reuse is based on full byte equality, never author labels or a hash alone.
    fn receive_image(
        &mut self,
        account: AccountScope,
        target: view::AssetTarget,
        bytes: wire::ImageBytes,
        attachment: Option<talk_adapters::identity::CachedAttachment>,
        preview_generation: Option<u64>,
    ) {
        if is_avatar(&target)
            && let Some(key) = self
                .content_owner
                .avatars
                .iter()
                .find_map(|(old, key)| (old == &bytes).then_some(*key))
        {
            self.set_asset(view::AssetView {
                target,
                state: view::AssetState::Ready { key },
                emoji: None,
                color: None,
                document_pages: None,
            });
        } else if self.content_owner.images.len() >= 2 {
            self.set_asset(view::AssetView {
                target,
                state: view::AssetState::Failed,
                emoji: None,
                color: None,
                document_pages: None,
            });
        } else {
            let Some(key) = self.content_owner.next_key.checked_add(1) else {
                return;
            };
            self.content_owner.next_key = key;
            self.content_owner.images.push(ImageJob {
                cached: false,
                attachment,
                preview_generation,
                account,
                target,
                key: self.content_owner.next_key,
                bytes,
            });
        }
    }
    fn content_failure(&mut self, job: &Job, error: IdentityError) {
        if job.context.is_none() && job.target.is_none() && !job.cloud {
            self.content_owner.profile = None;
            self.content_owner.profile_retry_at = Some(Instant::now() + Duration::from_secs(30));
            return;
        }
        if let Some(target) = job.target.clone() {
            if matches!(
                error,
                IdentityError::Request(
                    ErrorKind::PermissionDenied
                        | ErrorKind::NotFound
                        | ErrorKind::ReauthenticationRequired
                )
            ) && let view::AssetTarget::Image(reference)
            | view::AssetTarget::Thumbnail(reference) = &target
            {
                self.forget_thumbnail(reference);
                self.set_asset(view::AssetView {
                    target: view::AssetTarget::Thumbnail(reference.clone()),
                    state: view::AssetState::Denied,
                    emoji: None,
                    color: None,
                    document_pages: None,
                });
            }
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
            // Automatic capability/poll reads own their typed state below. They
            // must neither fabricate a user-action result nor replace an older
            // deliberate result that the presentation has not acknowledged.
            if job.mutation || job.cloud {
                self.snapshot.content_panel.result = Some((job.request, completion));
            }
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
    pub(super) fn set_asset(&mut self, asset: view::AssetView) {
        let mut assets = self.snapshot.content_panel.assets.as_slice().to_vec();
        assets.retain(|a| a.target != asset.target);
        if assets.len() >= 128 {
            // Message/preview churn must not repeatedly evict the persistent
            // footer and sidebar pictures merely because they loaded first.
            let index = assets
                .iter()
                .position(|a| !self.navigation_avatar(&a.target))
                .unwrap_or(0);
            assets.remove(index);
        }
        self.content_owner
            .leases
            .retain(|(target, _)| target != &asset.target);
        if asset.state != view::AssetState::Loading {
            self.content_owner
                .leases
                .push((asset.target.clone(), Instant::now()));
        }
        assets.push(asset);
        self.snapshot.content_panel.assets = page(assets);
        self.retire_content_keys();
    }
    fn navigation_avatar(&self, target: &view::AssetTarget) -> bool {
        match target {
            view::AssetTarget::SelfAvatar => true,
            view::AssetTarget::ConversationAvatar(context) => {
                self.snapshot.selected == Some(*context)
                    || self
                        .snapshot
                        .conversations
                        .as_slice()
                        .iter()
                        .take(24)
                        .any(|r| r.context == *context)
            }
            _ => false,
        }
    }
    fn retire_content_keys(&mut self) {
        self.content_owner.descriptors.retain(|(target, _)| {
            self.snapshot
                .content_panel
                .assets
                .as_slice()
                .iter()
                .any(|a| a.target == *target)
        });
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
        self.content_owner
            .avatars
            .retain(|(_, key)| self.content_owner.retained.iter().any(|(k, _)| k == key));
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
            || (!job.cached && self.current(job.account).is_err())
            || (job.cached && !self.cached_thumbnail_job_current(job))
        {
            return false;
        }
        if let (
            Some(captured),
            view::AssetTarget::Image(reference) | view::AssetTarget::Thumbnail(reference),
        ) = (&job.attachment, &job.target)
            && self.current_attachment_descriptor(reference).as_ref() != Some(captured)
        {
            return false;
        }
        if matches!(job.target, view::AssetTarget::Image(_))
            && job.preview_generation != Some(self.content_owner.preview_generation)
        {
            return false;
        }
        if matches!(&job.target,view::AssetTarget::Image(reference) if self.snapshot.content_panel.preview.as_ref()!=Some(reference))
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
        if let Some(bytes) = bytes.filter(|n| {
            *n <= if matches!(job.target, view::AssetTarget::Image(_)) {
                64 * 1024 * 1024
            } else {
                8 * 1024 * 1024
            }
        }) {
            while self
                .content_owner
                .retained
                .iter()
                .map(|(_, n)| *n)
                .sum::<usize>()
                + bytes
                > 96 * 1024 * 1024
            {
                if self.content_owner.retained.is_empty() {
                    break;
                }
                let index = self
                    .content_owner
                    .retained
                    .iter()
                    .position(|(key, _)| {
                        self.snapshot
                            .content_panel
                            .assets
                            .as_slice()
                            .iter()
                            .any(|a| {
                                a.state == (view::AssetState::Ready { key: *key })
                                    && !is_avatar(&a.target)
                            })
                    })
                    .unwrap_or(0);
                let (key, _) = self.content_owner.retained.remove(index);
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
            if let Some(attachment) = &job.attachment {
                self.content_owner
                    .descriptors
                    .retain(|(target, _)| target != &job.target);
                self.content_owner
                    .descriptors
                    .push((job.target.clone(), attachment.clone()));
            }
            if is_avatar(&job.target) {
                let size = job.bytes.as_slice().len();
                while !self.content_owner.avatars.is_empty()
                    && (self.content_owner.avatars.len() >= 128
                        || self
                            .content_owner
                            .avatars
                            .iter()
                            .map(|(b, _)| b.as_slice().len())
                            .sum::<usize>()
                            + size
                            > 8 * 1024 * 1024)
                {
                    self.content_owner.avatars.remove(0);
                }
                self.content_owner
                    .avatars
                    .push((job.bytes.clone(), job.key));
            }
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
    pub(super) fn asset_is_current(&self, target: &view::AssetTarget) -> bool {
        if let view::AssetTarget::Image(reference) | view::AssetTarget::Thumbnail(reference) =
            target
            && self
                .content_owner
                .descriptors
                .iter()
                .find(|(t, _)| t == target)
                .is_some_and(|(_, old)| {
                    self.current_attachment_descriptor(reference).as_ref() != Some(old)
                })
        {
            return false;
        }
        if matches!(target, view::AssetTarget::Thumbnail(_))
            && target
                .context()
                .is_some_and(|c| self.cache.is_presentation(c))
        {
            return self.cached_thumbnail_current(target);
        }
        if target.context().is_some_and(|context| {
            Some(context.account) != self.snapshot.active_account
                || !self
                    .snapshot
                    .conversations
                    .as_slice()
                    .iter()
                    .any(|r| r.context == context)
        }) {
            return false;
        }
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
            return !attachment && self.retained_message_avatar(target);
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
                && (!attachment || !m.metadata.deleted)
                && m.metadata.expires_at.is_none_or(|at| at == 0 || at > now)
        };
        // The first observation by precedence owns this identity, including
        // tombstones and removed placeholders. A stale pin cannot revive it.
        let message = self
            .snapshot
            .messages
            .as_slice()
            .iter()
            .chain(
                self.snapshot
                    .message_panel
                    .as_ref()
                    .filter(|p| p.context == context)
                    .into_iter()
                    .flat_map(|p| {
                        p.shared_files
                            .as_slice()
                            .iter()
                            .chain(p.thread_messages.as_slice())
                            .chain(p.pinned_messages.as_slice())
                    }),
            )
            .find(|m| m.local_id == id);
        message.is_some_and(|m| {
            valid(&m)
                && match target {
                    view::AssetTarget::Image(a) | view::AssetTarget::Thumbnail(a) => m
                        .metadata
                        .rich
                        .as_slice()
                        .iter()
                        .any(|r| r.kind.as_str() == "file" && r.placeholder == a.placeholder),
                    _ => true,
                }
        })
    }
    /// Navigation keeps pixels only alongside an unrestricted, retained history
    /// row. Removing that history, observing restrictions, expiry or account
    /// retirement removes this reference and the decoded image it kept alive.
    fn retained_message_avatar(&self, target: &view::AssetTarget) -> bool {
        let view::AssetTarget::MessageAvatar { context, message } = target else {
            return false;
        };
        Some(context.account) == self.snapshot.active_account
            && self
                .snapshot
                .conversations
                .as_slice()
                .iter()
                .any(|r| r.context == *context && r.draft_clipboard == Decision::Allowed)
            && self.history.get(*context).is_some_and(|history| {
                let now = history::now_unix();
                history.messages.as_slice().iter().any(|m| {
                    m.local_id == *message
                        && m.metadata.expires_at.is_none_or(|at| at == 0 || at > now)
                })
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
                            .is_none_or(|(_, at)| {
                                // The one open original is owned by its preview
                                // lifetime. Time-based tile eviction must not
                                // redownload it or restart a paused video.
                                if matches!(&asset.target, view::AssetTarget::Image(reference)
                                    if self.snapshot.content_panel.preview.as_ref() == Some(reference))
                                {
                                    return true;
                                }
                                let ttl = if asset.state == view::AssetState::Failed {
                                    // A transient failure must not permanently
                                    // suppress the UI's one missing-asset intent.
                                    30
                                } else {
                                    match asset.target {
                                        view::AssetTarget::Thumbnail(_)
                                        | view::AssetTarget::Image(_) => 60,
                                        _ => 24 * 60 * 60,
                                    }
                                };
                                at.elapsed() < Duration::from_secs(ttl)
                            })
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
        self.hydrate_thumbnails();
        self.tick_video();
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
                attachment: None,
                preview_generation: None,
            };
            if self
                .start_content(job, wire::ContentCommand::SelfProfile)
                .is_ok()
            {
                self.content_owner.profile = Some(account);
            }
        }
        self.retry_content_asset(Instant::now());
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

    fn attachment_model() -> (Accounts, view::AttachmentRef) {
        let mut model = super::super::tests::model();
        model.snapshot = super::super::super::fixtures::snapshot(Some("chat"));
        let context = model.snapshot.selected.unwrap();
        let mut row = model.snapshot.messages.as_slice()[0].clone();
        row.local_id = 7;
        row.metadata.deleted = false;
        row.metadata.expires_at = None;
        row.metadata.rich = page([talk_core::ui::messaging::RichObject {
            placeholder: text("file"),
            kind: text("file"),
            name: text("synthetic.png"),
            mime_type: Some(text("image/png")),
            size: Some(32),
            id: Some(text("7")),
            link: None,
        }]);
        model.snapshot.messages = page([row]);
        model.snapshot.conversations = page(
            model
                .snapshot
                .conversations
                .as_slice()
                .iter()
                .cloned()
                .map(|mut room| {
                    room.draft_clipboard = Decision::Allowed;
                    room
                }),
        );
        (
            model,
            view::AttachmentRef {
                context,
                message: 7,
                placeholder: text("file"),
            },
        )
    }

    #[test]
    fn open_original_outlives_tile_lease_but_closing_releases_expired_pixels() {
        let (mut model, reference) = attachment_model();
        let target = view::AssetTarget::Image(reference.clone());
        model.snapshot.content_panel.preview = Some(reference);
        model.set_asset(asset(target.clone(), 9));
        model.content_owner.retained = vec![(9, 16)];
        model.content_owner.leases =
            vec![(target.clone(), Instant::now() - Duration::from_secs(61))];
        model.reconcile_content_assets();
        assert_eq!(
            model.snapshot.content_panel.assets.as_slice(),
            &[asset(target.clone(), 9)]
        );
        model.snapshot.content_panel.preview = None;
        model.reconcile_content_assets();
        assert!(model.snapshot.content_panel.assets.as_slice().is_empty());
        assert!(model.content_owner.retained.is_empty());
    }

    #[test]
    fn main_tombstone_or_removed_file_dominates_a_stale_pin_with_the_same_message_id() {
        let (mut model, reference) = attachment_model();
        let original = model.snapshot.messages.as_slice()[0].clone();
        let mut panel = talk_core::ui::messaging::MessagePanel::new(reference.context);
        panel.pinned_messages = page([original.clone()]);
        model.snapshot.message_panel = Some(panel);
        let target = view::AssetTarget::Image(reference);
        assert!(model.asset_is_current(&target));
        for removed_placeholder in [false, true] {
            let mut current = original.clone();
            if removed_placeholder {
                current.metadata.rich = Page::empty();
            } else {
                current.metadata.deleted = true;
            }
            model.snapshot.messages = page([current]);
            assert!(
                !model.asset_is_current(&target),
                "secondary copies cannot revive a retired file"
            );
        }
    }

    #[test]
    fn automatic_poll_failures_keep_typed_state_without_creating_or_overwriting_action_results() {
        for poll in [None, Some(7)] {
            for (kind, reason) in [
                (ErrorKind::Unsupported, UnavailableReason::Unsupported),
                (ErrorKind::NotFound, UnavailableReason::Unconfigured),
            ] {
                let mut model = super::super::tests::model();
                model.snapshot = super::super::super::fixtures::snapshot(Some("chat"));
                let context = model.snapshot.selected.unwrap();
                let job = Job {
                    request: RequestId(0),
                    account: context.account,
                    context: Some(context),
                    operation: OperationId::from_local(1),
                    target: None,
                    poll,
                    mutation: false,
                    cloud: false,
                    attachment: None,
                    preview_generation: None,
                };
                model.snapshot.content_panel.result = None;
                model.content_failure(&job, IdentityError::Request(kind));
                assert_eq!(model.snapshot.content_panel.result, None);
                assert_eq!(
                    model.snapshot.content_panel.poll_state,
                    RemoteState::Failed(RequestId(0), reason)
                );
                if poll.is_none() {
                    assert_eq!(
                        model.snapshot.content_panel.poll_create,
                        Availability::UNKNOWN
                    );
                    assert!(model.content_owner.capabilities_at.is_some());
                }
                let deliberate = Some((RequestId(77), Completion::ServerAccepted));
                model.snapshot.content_panel.result = deliberate;
                model.content_failure(&job, IdentityError::Request(kind));
                assert_eq!(
                    model.snapshot.content_panel.result, deliberate,
                    "a background read cannot erase an unacknowledged user action"
                );
            }
        }
    }

    #[test]
    fn deliberate_cloud_and_poll_mutation_failures_keep_correlated_action_results() {
        for cloud in [false, true] {
            for (kind, completion) in [
                (
                    ErrorKind::Unsupported,
                    Completion::Failed(UnavailableReason::Unsupported),
                ),
                (ErrorKind::Uncertain, Completion::Uncertain),
            ] {
                let mut model = super::super::tests::model();
                model.snapshot = super::super::super::fixtures::snapshot(Some("chat"));
                model.snapshot.content_panel.poll_state = RemoteState::Idle;
                let context = model.snapshot.selected.unwrap();
                let job = Job {
                    request: RequestId(88),
                    account: context.account,
                    context: Some(context),
                    operation: OperationId::from_local(2),
                    target: None,
                    poll: (!cloud).then_some(7),
                    mutation: !cloud,
                    cloud,
                    attachment: None,
                    preview_generation: None,
                };
                model.content_failure(&job, IdentityError::Request(kind));
                assert_eq!(
                    model.snapshot.content_panel.result,
                    Some((RequestId(88), completion))
                );
                if cloud {
                    assert_eq!(model.snapshot.content_panel.poll_state, RemoteState::Idle);
                } else {
                    assert_eq!(model.content_owner.refresh_poll, Some((context, 7)));
                    assert_eq!(
                        model.snapshot.content_panel.poll_state,
                        if kind == ErrorKind::Uncertain {
                            RemoteState::Uncertain(RequestId(88))
                        } else {
                            RemoteState::Failed(RequestId(88), UnavailableReason::Unsupported)
                        }
                    );
                }
            }
        }
    }

    fn asset(target: view::AssetTarget, key: u64) -> view::AssetView {
        view::AssetView {
            target,
            state: view::AssetState::Ready { key },
            emoji: None,
            color: None,
            document_pages: None,
        }
    }

    fn retain_history(model: &mut Accounts, context: ConversationRef, id: u64) {
        retain_message(
            model,
            context,
            id,
            talk_core::protocol::MessageKind::Comment,
            None,
        );
    }

    fn retain_message(
        model: &mut Accounts,
        context: ConversationRef,
        id: u64,
        kind: talk_core::protocol::MessageKind,
        expires_at: Option<u64>,
    ) {
        use talk_core::protocol::{Message, MessageId, messaging::*};
        let operation = OperationId::from_local(1);
        model.history.open(context);
        model.history.admitted(
            context,
            operation,
            messaging::Work::History,
            RequestId(1),
            1,
        );
        let result = MessagingResult::Messages {
            messages: page([MessageDetails {
                message: Message {
                    id: MessageId {
                        conversation: context.conversation,
                        sequence: id,
                    },
                    author: text("Synthetic user"),
                    text: text("Retained conversation"),
                    kind,
                    timestamp: 1,
                    expires_at,
                    edited_at: None,
                    own_operation: None,
                },
                author_key: None,
                notify: Decision::Denied,
                own: false,
                replyable: Some(true),
                parent: None,
                thread: None,
                reactions: Page::empty(),
                rich: Page::empty(),
                system: None,
                edit: Decision::Unknown,
                delete: Decision::Unknown,
            }]),
            cursor: None,
            common_read: None,
            capabilities: None,
        };
        assert!(model.history.complete(
            context,
            operation,
            messaging::Work::History,
            RequestId(1),
            &Ok(result),
            false
        ));
    }

    #[test]
    fn deleted_message_keeps_its_author_avatar_but_retires_attachment_pixels() {
        let mut model = super::super::tests::model();
        model.snapshot = super::super::super::fixtures::snapshot(Some("chat"));
        let context = model.snapshot.selected.unwrap();
        retain_message(
            &mut model,
            context,
            7,
            talk_core::protocol::MessageKind::Deleted,
            None,
        );
        model.snapshot.messages = model.history.get(context).unwrap().messages.clone();
        assert!(model.snapshot.messages.as_slice()[0].metadata.deleted);
        let avatar = view::AssetTarget::MessageAvatar {
            context,
            message: 7,
        };
        let attachment = view::AssetTarget::Thumbnail(view::AttachmentRef {
            context,
            message: 7,
            placeholder: text("file"),
        });
        model.set_asset(asset(avatar.clone(), 2));
        model.set_asset(asset(attachment.clone(), 3));
        model.content_owner.retained = vec![(2, 16), (3, 16)];
        assert!(model.asset_is_current(&avatar));
        assert!(!model.asset_is_current(&attachment));
        model.reconcile_content_assets();
        assert!(
            model
                .snapshot
                .content_panel
                .assets
                .as_slice()
                .contains(&asset(avatar.clone(), 2))
        );
        assert!(
            !model
                .snapshot
                .content_panel
                .assets
                .as_slice()
                .iter()
                .any(|a| a.target == attachment)
        );
        assert_eq!(model.content_owner.retained, vec![(2, 16)]);

        model.snapshot.selected = None;
        model.snapshot.messages = Page::empty();
        assert!(
            model.asset_is_current(&avatar),
            "a retained tombstone retains its author pixels"
        );
        model.history.remove(context);
        retain_message(
            &mut model,
            context,
            7,
            talk_core::protocol::MessageKind::Deleted,
            Some(1),
        );
        assert!(
            !model.asset_is_current(&avatar),
            "message expiry still retires its avatar"
        );
        model.reconcile_content_assets();
        assert!(
            !model
                .snapshot
                .content_panel
                .assets
                .as_slice()
                .iter()
                .any(|a| a.target == avatar)
        );
        assert!(model.content_owner.retained.is_empty());
    }

    #[test]
    fn cached_navigation_and_repeated_snapshots_keep_avatar_keys_until_history_or_policy_retires() {
        let mut model = super::super::tests::model();
        model.snapshot = super::super::super::fixtures::snapshot(Some("chat"));
        let context = model.snapshot.selected.unwrap();
        retain_history(&mut model, context, 7);
        model.snapshot.messages = model.history.get(context).unwrap().messages.clone();
        model.snapshot.content_panel.account = Some(context.account);
        let target = view::AssetTarget::MessageAvatar {
            context,
            message: 7,
        };
        model.set_asset(asset(view::AssetTarget::SelfAvatar, 1));
        model.set_asset(asset(target.clone(), 2));
        model.content_owner.retained = vec![(1, 16), (2, 16)];
        model.clear_content_assets();
        model.snapshot.selected = None;
        model.snapshot.messages = Page::empty();
        for _ in 0..100 {
            model.reconcile_content_assets();
        }
        assert!(
            model
                .snapshot
                .content_panel
                .assets
                .as_slice()
                .contains(&asset(target.clone(), 2))
        );
        assert!(
            model.content_owner.images.is_empty(),
            "no repeated decode admission"
        );
        model.snapshot.selected = Some(context);
        model.snapshot.messages = model.history.get(context).unwrap().messages.clone();
        model.reconcile_content_assets();
        assert!(
            model
                .snapshot
                .content_panel
                .assets
                .as_slice()
                .contains(&asset(target.clone(), 2))
        );

        model.snapshot.selected = None;
        model.snapshot.conversations = page(
            model
                .snapshot
                .conversations
                .as_slice()
                .iter()
                .cloned()
                .map(|mut r| {
                    if r.context == context {
                        r.draft_clipboard = Decision::Unknown;
                    }
                    r
                }),
        );
        model.reconcile_content_assets();
        assert!(
            !model
                .snapshot
                .content_panel
                .assets
                .as_slice()
                .iter()
                .any(|a| a.target == target)
        );
        assert_eq!(model.content_owner.retained, vec![(1, 16)]);
        // Independently, removing a retained history retires its avatar, even
        // when the room remains visible and unrestricted in navigation.
        model.snapshot.conversations = page(
            model
                .snapshot
                .conversations
                .as_slice()
                .iter()
                .cloned()
                .map(|mut r| {
                    r.draft_clipboard = Decision::Allowed;
                    r
                }),
        );
        model.set_asset(asset(target.clone(), 3));
        model.history.remove(context);
        model.reconcile_content_assets();
        assert!(
            !model
                .snapshot
                .content_panel
                .assets
                .as_slice()
                .iter()
                .any(|a| a.target == target)
        );
    }

    #[test]
    fn message_asset_capacity_does_not_cycle_footer_and_sidebar_images() {
        let mut model = super::super::tests::model();
        model.snapshot = super::super::super::fixtures::snapshot(Some("chat"));
        let context = model.snapshot.selected.unwrap();
        model.set_asset(asset(view::AssetTarget::SelfAvatar, 1));
        model.set_asset(asset(view::AssetTarget::ConversationAvatar(context), 2));
        for message in 1..=300 {
            model.set_asset(asset(
                view::AssetTarget::MessageAvatar { context, message },
                3,
            ));
        }
        let assets = model.snapshot.content_panel.assets.as_slice();
        assert_eq!(assets.len(), 128);
        assert!(assets.contains(&asset(view::AssetTarget::SelfAvatar, 1)));
        assert!(assets.contains(&asset(view::AssetTarget::ConversationAvatar(context), 2)));
    }

    #[test]
    fn equal_validated_avatar_results_share_pixels_but_attachment_images_do_not() {
        let mut model = super::super::tests::model();
        model.snapshot = super::super::super::fixtures::snapshot(Some("chat"));
        let context = model.snapshot.selected.unwrap();
        let bytes = wire::ImageBytes::new(b"synthetic decoded avatar source".to_vec()).unwrap();
        model.set_asset(asset(view::AssetTarget::SelfAvatar, 1));
        model.content_owner.retained = vec![(1, 16)];
        model.content_owner.avatars.push((bytes.clone(), 1));
        let target = view::AssetTarget::MessageAvatar {
            context,
            message: 7,
        };
        model.receive_image(context.account, target.clone(), bytes.clone(), None, None);
        assert!(model.content_owner.images.is_empty());
        assert!(
            model
                .snapshot
                .content_panel
                .assets
                .as_slice()
                .contains(&asset(target.clone(), 1))
        );
        let attachment = view::AssetTarget::Thumbnail(view::AttachmentRef {
            context,
            message: 7,
            placeholder: text("file"),
        });
        model.receive_image(context.account, attachment.clone(), bytes, None, None);
        assert_eq!(model.take_content_image().unwrap().target, attachment);
        model.snapshot.content_panel.assets = page([asset(target, 1)]);
        model.retire_content_keys();
        assert_eq!(
            model.content_owner.avatars.len(),
            1,
            "a remaining authorized alias retains the pixels"
        );
        model.snapshot.content_panel.assets = Page::empty();
        model.retire_content_keys();
        assert!(model.content_owner.avatars.is_empty());
        assert!(model.content_owner.retained.is_empty());
    }

    fn busy_content_jobs(model: &mut Accounts, account: AccountScope) {
        for serial in 1..=2 {
            model.content_owner.jobs.push(Job {
                request: RequestId(serial),
                account,
                context: None,
                operation: OperationId::from_local(u128::from(serial)),
                target: None,
                poll: None,
                mutation: false,
                cloud: false,
                attachment: None,
                preview_generation: None,
            });
        }
    }

    #[test]
    fn busy_asset_admission_is_bounded_deduplicated_and_settled_without_removing_ready_pixels() {
        let mut model = super::super::tests::model();
        model.snapshot = super::super::super::fixtures::snapshot(Some("chat"));
        let context = model.snapshot.selected.unwrap();
        model.snapshot.content_panel.account = Some(context.account);
        busy_content_jobs(&mut model, context.account);
        for _ in 0..10 {
            assert_eq!(
                model.request_asset(RequestId(7), context.account, view::AssetTarget::SelfAvatar),
                Ok(())
            );
            model.retry_content_asset(Instant::now());
        }
        assert_eq!(model.content_owner.pending_assets.len(), 1);
        assert_eq!(
            model.content_owner.jobs.len(),
            2,
            "no extra work while the lane is full"
        );
        assert_eq!(model.content_owner.pending_assets[0].request, RequestId(7));
        model.set_asset(asset(view::AssetTarget::SelfAvatar, 5));
        model.retry_content_asset(Instant::now() + Duration::from_secs(61));
        assert!(model.content_owner.pending_assets.is_empty());
        assert!(
            model
                .snapshot
                .content_panel
                .assets
                .as_slice()
                .contains(&asset(view::AssetTarget::SelfAvatar, 5)),
            "a queued timeout cannot overwrite a ready image"
        );

        model.snapshot.content_panel.assets = Page::empty();
        for message in 1..=129 {
            let target = view::AssetTarget::MessageAvatar { context, message };
            assert_eq!(
                model.request_asset(RequestId(message as u64), context.account, target),
                Ok(())
            );
        }
        assert_eq!(model.content_owner.pending_assets.len(), 128);
        assert!(
            model
                .snapshot
                .content_panel
                .assets
                .as_slice()
                .iter()
                .any(|a| a.target
                    == (view::AssetTarget::MessageAvatar {
                        context,
                        message: 129
                    })
                    && a.state == view::AssetState::Failed),
            "overload is terminal, not silently deduplicated forever"
        );
        model.clear_content_assets();
        assert!(
            model.content_owner.pending_assets.is_empty(),
            "navigation cancels work before any request starts"
        );
    }

    #[test]
    fn asset_retry_deadline_and_stale_scope_prevent_late_effects() {
        let mut model = super::super::tests::model();
        model.snapshot = super::super::super::fixtures::snapshot(Some("chat"));
        let context = model.snapshot.selected.unwrap();
        model.snapshot.content_panel.account = Some(context.account);
        busy_content_jobs(&mut model, context.account);
        model
            .request_asset(RequestId(7), context.account, view::AssetTarget::SelfAvatar)
            .unwrap();
        model.retry_content_asset(Instant::now() + Duration::from_secs(61));
        assert!(model.content_owner.pending_assets.is_empty());
        assert!(
            model
                .snapshot
                .content_panel
                .assets
                .as_slice()
                .iter()
                .any(|a| a.target == view::AssetTarget::SelfAvatar
                    && a.state == view::AssetState::Failed)
        );
        model
            .request_asset(RequestId(8), context.account, view::AssetTarget::SelfAvatar)
            .unwrap();
        model.snapshot.active_account = None;
        model.retry_content_asset(Instant::now());
        assert!(model.content_owner.pending_assets.is_empty());
        assert_eq!(
            model.content_owner.jobs.len(),
            2,
            "obsolete queued work starts no effects"
        );
    }

    #[test]
    fn transient_avatar_failure_backoff_expires_without_dropping_ready_navigation_pixels() {
        let mut model = super::super::tests::model();
        model.snapshot = super::super::super::fixtures::snapshot(Some("chat"));
        let context = model.snapshot.selected.unwrap();
        model.snapshot.content_panel.account = Some(context.account);
        model.set_asset(asset(view::AssetTarget::ConversationAvatar(context), 5));
        model.asset_retry_failed(view::AssetTarget::SelfAvatar);
        model.reconcile_content_assets();
        assert!(
            model
                .snapshot
                .content_panel
                .assets
                .as_slice()
                .iter()
                .any(|a| a.target == view::AssetTarget::SelfAvatar)
        );
        for (target, at) in &mut model.content_owner.leases {
            if *target == view::AssetTarget::SelfAvatar {
                *at = Instant::now() - Duration::from_secs(31);
            }
        }
        model.reconcile_content_assets();
        assert!(
            !model
                .snapshot
                .content_panel
                .assets
                .as_slice()
                .iter()
                .any(|a| a.target == view::AssetTarget::SelfAvatar),
            "the automatic loader can admit the target again after bounded backoff"
        );
        assert!(
            model
                .snapshot
                .content_panel
                .assets
                .as_slice()
                .contains(&asset(view::AssetTarget::ConversationAvatar(context), 5))
        );
    }

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
