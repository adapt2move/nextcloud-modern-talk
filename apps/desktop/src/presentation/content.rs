//! Native content surfaces over scoped observations and composition-owned images.
//! Views never resolve a URL, decode remote bytes, or report transfer success.
pub(in crate::presentation) mod gallery;
pub(in crate::presentation) mod groups;
mod window;

use super::{
    ButtonStyle, Desktop, LocalAction, Overlay,
    controls::{Availability, scoped_id},
    input::TextInput,
    messaging::composer_menu,
};
use gpui::{
    Context, Div, Entity, Focusable, ImageSource, ObjectFit, Window, div, img, prelude::*, px,
};
use talk_core::ui::content::{
    AssetState, AssetTarget, AssetView, AttachmentRef, ContentAction, DownloadState, PollDraft,
    PollView,
};
use talk_core::ui::presentation::{
    Completion, ContentState, ConversationRef, PresentationSnapshot, RequestId, UiIntent,
};
use talk_core::ui::{AccountScope, Page, Text, conversation::RemoteState};

const MAX_IMAGES: usize = 128;
const MAX_POLL_OPTIONS: usize = 8;

#[derive(Clone)]
pub(crate) enum Action {
    Menu,
    ComposerChoice(composer_menu::Target, composer_menu::Choice),
    CreatePoll,
    AddOption,
    RemoveOption,
    Multiple,
    HiddenResults,
    SubmitPoll,
    Preview(AttachmentRef),
    GalleryMove { from: AttachmentRef, offset: i8 },
    Download(AttachmentRef),
    Command(ContentAction),
}

pub(super) struct ContentUi {
    account: Option<AccountScope>,
    context: Option<ConversationRef>,
    images: Vec<(u64, ImageSource)>,
    requested: Vec<AssetTarget>,
    request: u64,
    pub menu: bool,
    pub(super) composer_menu: composer_menu::Menu,
    preview: Option<AttachmentRef>,
    gallery: Vec<AttachmentRef>,
    gallery_position: Option<(usize, usize)>,
    media_groups: Vec<groups::Span>,
    swipe: gallery::Swipe,
    return_overlay: Option<Overlay>,
    pub question: Entity<TextInput>,
    pub options: Vec<Entity<TextInput>>,
    option_count: usize,
    multiple: bool,
    hidden_results: bool,
    pending_poll: Option<(ConversationRef, RequestId)>,
    acknowledged_result: Option<RequestId>,
    observed_polls: Vec<PollView>,
}
impl ContentUi {
    pub fn new(
        theme: super::Theme,
        account: Option<AccountScope>,
        context: Option<ConversationRef>,
        cx: &mut Context<Desktop>,
    ) -> Self {
        Self {
            account,
            context,
            images: Vec::new(),
            requested: Vec::new(),
            request: 0,
            menu: false,
            composer_menu: composer_menu::Menu::new(cx),
            preview: None,
            gallery: Vec::new(),
            gallery_position: None,
            media_groups: Vec::new(),
            swipe: gallery::Swipe::default(),
            return_overlay: None,
            question: cx.new(|cx| TextInput::new("Frage der Umfrage", 4000, theme, cx)),
            options: (0..MAX_POLL_OPTIONS)
                .map(|_| cx.new(|cx| TextInput::new("Antwortoption", 1000, theme, cx)))
                .collect(),
            option_count: 2,
            multiple: false,
            hidden_results: false,
            pending_poll: None,
            acknowledged_result: None,
            observed_polls: Vec::new(),
        }
    }
    pub fn inputs(&self) -> impl Iterator<Item = &Entity<TextInput>> {
        std::iter::once(&self.question).chain(self.options.iter())
    }
}
impl Desktop {
    /// Composition supplies already decoded render images. Reject URI/path
    /// image sources; renderer caches cannot become an alternate HTTP owner.
    pub fn set_content_images(
        &mut self,
        account: Option<AccountScope>,
        images: Vec<(u64, ImageSource)>,
        cx: &mut Context<Self>,
    ) {
        if account.is_some() && account != self.snapshot.active_account {
            // An obsolete image completion cannot clear the newer account's
            // cache. Only explicit None or current-scope observations retire it.
            return;
        }
        let admitted = account.is_some()
            && account == self.snapshot.active_account
            && account == self.snapshot.content_panel.account;
        let keys: Vec<_> = self
            .snapshot
            .content_panel
            .assets
            .as_slice()
            .iter()
            .filter_map(|a| {
                if let AssetState::Ready { key } = a.state {
                    Some(key)
                } else {
                    None
                }
            })
            .chain(
                self.snapshot
                    .content_panel
                    .retained_avatar_keys
                    .as_slice()
                    .iter()
                    .copied(),
            )
            .collect();
        let mut changed = false;
        let previous_account = self.content_ui.account;
        self.content_ui.images.retain(|(key, image)| {
            let keep = admitted && previous_account == account && keys.contains(key);
            if !keep {
                if let ImageSource::Render(image) = image {
                    cx.drop_image(image.clone(), None);
                }
                changed = true;
            }
            keep
        });
        if admitted {
            for (key, image) in images.into_iter().take(MAX_IMAGES) {
                if !matches!(image, ImageSource::Render(_)) || !keys.contains(&key) {
                    continue;
                }
                if let Some(index) = self
                    .content_ui
                    .images
                    .iter()
                    .position(|(old, _)| *old == key)
                {
                    if let (Some((_, ImageSource::Render(previous))), ImageSource::Render(next)) =
                        (self.content_ui.images.get(index), &image)
                        && std::sync::Arc::ptr_eq(previous, next)
                    {
                        continue;
                    }
                    let (_, previous) = self.content_ui.images.remove(index);
                    if let ImageSource::Render(previous) = previous {
                        cx.drop_image(previous, None);
                    }
                }
                if self.content_ui.images.len() < MAX_IMAGES {
                    self.content_ui.images.push((key, image));
                    changed = true;
                }
            }
        }
        if changed {
            cx.notify();
        }
    }

    fn submit_content(&mut self, action: ContentAction) -> Option<RequestId> {
        let account = self.snapshot.active_account?;
        let request = RequestId(self.content_ui.request.checked_add(1)?);
        self.content_ui.request = request.0;
        self.submit(UiIntent::Content {
            request,
            account,
            action,
        })
        .then_some(request)
    }

    /// One missing asset per existing owner tick. The content owner settles
    /// enqueue success with admission, bounded Busy retry or a failed asset;
    /// no render callback starts work.
    pub(super) fn refresh_content_ui(&mut self, cx: &mut Context<Self>) {
        let account = self.snapshot.active_account;
        if self.content_ui.account != account || self.content_ui.context != self.snapshot.selected {
            self.content_ui.requested.clear();
            self.content_ui.menu = false;
            self.content_ui.preview = None;
            self.content_ui.pending_poll = None;
            self.content_ui.acknowledged_result = None;
            self.content_ui.question.update(cx, |i, _| i.reset());
            for input in &self.content_ui.options {
                input.update(cx, |i, _| i.reset());
            }
            self.content_ui.option_count = 2;
            self.content_ui.multiple = false;
            self.content_ui.hidden_results = false;
            self.content_ui.return_overlay = None;
            if matches!(
                self.overlay,
                Some(Overlay::ContentPreview | Overlay::CreatePoll)
            ) {
                self.overlay = None;
                self.pending_modal_focus = false;
            }
            if self.content_ui.account != account {
                self.set_content_images(None, Vec::new(), cx);
            }
            self.content_ui.account = account;
            self.content_ui.context = self.snapshot.selected;
        }
        self.content_ui.gallery =
            gallery::entries(&self.snapshot, self.content_ui.preview.as_ref());
        self.content_ui.gallery_position = self
            .content_ui
            .preview
            .as_ref()
            .and_then(|reference| window::global_position(&self.snapshot, reference));
        let pins: Vec<_> = self
            .snapshot
            .message_panel
            .as_ref()
            .filter(|p| Some(p.context) == self.snapshot.selected)
            .into_iter()
            .flat_map(|p| p.pinned_messages.as_slice())
            .map(|m| m.local_id)
            .collect();
        self.content_ui.media_groups =
            groups::spans(self.snapshot.messages.as_slice(), &pins, |a, b| {
                self.settings_port.same_local_day(a, b)
            });
        if self.overlay == Some(Overlay::ContentPreview)
            && (self.snapshot.content == ContentState::Denied
                || self
                    .content_ui
                    .preview
                    .as_ref()
                    .is_some_and(|r| self.attachment_object(r).is_none()))
        {
            self.close_content_overlay();
        }
        self.set_content_images(account, Vec::new(), cx);
        if self.projection_blocked || self.update_restart_pending {
            return;
        }
        if self.content_ui.observed_polls.as_slice() != self.snapshot.content_panel.polls.as_slice()
        {
            let anchor = self.message_ui.list.logical_scroll_top();
            let count = self.message_ui.list.item_count();
            self.message_ui.list.splice(0..count, count);
            self.message_ui.list.scroll_to(anchor);
            self.content_ui.observed_polls = self.snapshot.content_panel.polls.as_slice().to_vec();
        }
        if let Some((request, completion)) = self.snapshot.content_panel.result {
            if self
                .content_ui
                .pending_poll
                .is_some_and(|(context, pending)| {
                    self.snapshot.selected == Some(context) && pending == request
                })
                && completion == Completion::ServerAccepted
            {
                self.content_ui.pending_poll = None;
                self.content_ui.question.update(cx, |i, _| i.reset());
                for input in &self.content_ui.options {
                    input.update(cx, |i, _| i.reset());
                }
                if self.overlay == Some(Overlay::CreatePoll) {
                    self.overlay = self.content_ui.return_overlay.take();
                    self.pending_modal_focus = self.overlay.is_some();
                }
            }
            if self.content_ui.acknowledged_result != Some(request) {
                if let Completion::Failed(_) = completion {
                    self.feedback = Some(super::feedback::Feedback::error(
                        "Die Inhaltsaktion ist fehlgeschlagen. Erneut versuchen.",
                    ));
                }
                if self.submit_content(ContentAction::Acknowledge).is_some() {
                    self.content_ui.acknowledged_result = Some(request);
                }
            }
        }
        let Some(account) = account else { return };
        let mut targets = Vec::with_capacity(MAX_IMAGES);
        // Returning online or retiring an obsolete descriptor must not leave
        // an open viewer waiting for an original nobody has requested.
        if !self.media_offline()
            && let Some(reference) = &self.content_ui.preview
        {
            targets.push(AssetTarget::Image(reference.clone()));
        }
        targets.push(AssetTarget::SelfAvatar);
        if let Some(context) = self.snapshot.selected {
            targets.push(AssetTarget::ConversationAvatar(context));
            let rows = self.snapshot.messages.as_slice();
            let top = self.message_ui.list.logical_scroll_top().item_ix;
            let start = if top >= rows.len() {
                rows.len().saturating_sub(16)
            } else {
                top
            };
            for message in rows.iter().skip(start).take(16) {
                let avatar = AssetTarget::MessageAvatar {
                    context,
                    message: message.local_id,
                };
                // Own rows use the account's already requested SelfAvatar.
                // A verified same-actor alias also needs no per-message GET;
                // the content owner still retires it on policy/expiry changes.
                if targets.len() < MAX_IMAGES
                    && !message.own
                    && matches!(
                        message.delivery,
                        talk_core::ui::presentation::DeliveryState::Received
                            | talk_core::ui::presentation::DeliveryState::ServerAccepted
                    )
                    && avatar_alias(&self.snapshot, &avatar).is_none()
                {
                    targets.push(avatar);
                }
                for rich in message
                    .metadata
                    .rich
                    .as_slice()
                    .iter()
                    .filter(|o| o.kind.as_str() == "file")
                    .take(4)
                {
                    if targets.len() < MAX_IMAGES {
                        targets.push(AssetTarget::Thumbnail(AttachmentRef {
                            context,
                            message: message.local_id,
                            placeholder: rich.placeholder.clone(),
                        }));
                    }
                }
            }
            if self.overlay == Some(Overlay::Participants)
                && let Some(panel) = self
                    .snapshot
                    .conversation_panel
                    .as_ref()
                    .filter(|p| p.context == context)
            {
                for member in panel.members.as_slice().iter().take(16) {
                    if targets.len() < MAX_IMAGES {
                        targets.push(AssetTarget::ParticipantAvatar {
                            context,
                            participant: member.id.local,
                        });
                    }
                }
            }
            if self.overlay == Some(Overlay::Files)
                && let Some(panel) = &self.snapshot.message_panel
            {
                for message in panel.shared_files.as_slice().iter().take(16) {
                    for object in message
                        .metadata
                        .rich
                        .as_slice()
                        .iter()
                        .filter(|r| r.kind.as_str() == "file")
                        .take(2)
                    {
                        if targets.len() < MAX_IMAGES {
                            targets.push(AssetTarget::Thumbnail(AttachmentRef {
                                context,
                                message: message.local_id,
                                placeholder: object.placeholder.clone(),
                            }));
                        }
                    }
                }
            }
        }
        for row in self.snapshot.conversations.as_slice().iter().take(24) {
            if targets.len() < MAX_IMAGES {
                targets.push(AssetTarget::ConversationAvatar(row.context));
            }
        }
        if matches!(self.overlay, Some(Overlay::Create | Overlay::Search))
            && let Some(panel) = &self.snapshot.create_panel
        {
            for person in panel.people.as_slice().iter().take(16) {
                if targets.len() < MAX_IMAGES {
                    targets.push(AssetTarget::PersonAvatar(person.id));
                }
            }
        }
        // Scrolling retires UI dedup keys. The backend owns the actual byte and
        // request limits and can return a terminal unavailable observation.
        self.content_ui.requested.retain(|t| {
            targets.contains(t)
                && !self
                    .snapshot
                    .content_panel
                    .assets
                    .as_slice()
                    .iter()
                    .any(|a| &a.target == t)
        });
        let target = targets.into_iter().find(|target| {
            !self.content_ui.requested.contains(target)
                && !self
                    .snapshot
                    .content_panel
                    .assets
                    .as_slice()
                    .iter()
                    .any(|a| &a.target == target)
        });
        if let Some(target) = target
            && let Some(next) = self.content_ui.request.checked_add(1)
        {
            self.content_ui.request = next;
            if self
                .port
                .try_submit(UiIntent::Content {
                    request: RequestId(next),
                    account,
                    action: match &target {
                        AssetTarget::Image(reference) => ContentAction::Preview(reference.clone()),
                        _ => ContentAction::Load(target.clone()),
                    },
                })
                .is_ok()
            {
                self.content_ui.requested.push(target);
            }
        }
    }

    fn asset(&self, target: &AssetTarget) -> Option<&talk_core::ui::content::AssetView> {
        (self.snapshot.content_panel.account == self.snapshot.active_account
            && self.snapshot.active_account.is_some())
        .then(|| {
            self.snapshot
                .content_panel
                .assets
                .as_slice()
                .iter()
                .find(|a| &a.target == target)
        })
        .flatten()
    }
    pub(super) fn content_scope_current(&self) -> bool {
        self.snapshot.active_account.is_some()
            && self.snapshot.content_panel.account == self.snapshot.active_account
            && self.snapshot.selected.is_some()
            && self.snapshot.content_panel.context == self.snapshot.selected
    }
    fn poll_submission_blocked(&self) -> bool {
        self.snapshot.content_panel.poll_state.pending()
    }
    fn asset_image(&self, target: &AssetTarget) -> Option<ImageSource> {
        let AssetState::Ready { key } = self.asset(target)?.state else {
            return None;
        };
        self.content_ui
            .images
            .iter()
            .find(|(id, _)| *id == key)
            .map(|(_, image)| image.clone())
    }
    pub(super) fn content_avatar(&self, target: AssetTarget, initials: &str, size: f32) -> Div {
        if let AssetTarget::MessageAvatar { context, message } = &target
            && self.snapshot.active_account == Some(context.account)
            && self.snapshot.selected == Some(*context)
            && self
                .snapshot
                .messages
                .as_slice()
                .iter()
                .any(|m| m.local_id == *message && m.own)
            && let Some(avatar) = self.projected_avatar(AssetTarget::SelfAvatar, size)
        {
            return avatar;
        }
        if let Some(avatar) = self.projected_avatar(target, size) {
            avatar
        } else {
            self.person_avatar(initials, size)
        }
    }
    pub(super) fn projected_avatar(&self, target: AssetTarget, size: f32) -> Option<Div> {
        let extent = px(size * (f32::from(self.scale) / 100.).sqrt());
        let base = div()
            .flex_none()
            .size(extent)
            .rounded(extent / 3.)
            .overflow_hidden()
            .flex()
            .items_center()
            .justify_center();
        let asset = self
            .asset(&target)
            .filter(|asset| matches!(asset.state, AssetState::Ready { .. }))
            .or_else(|| avatar_alias(&self.snapshot, &target))?;
        let image = if let AssetState::Ready { key } = asset.state {
            self.content_ui
                .images
                .iter()
                .find(|(id, _)| *id == key)
                .map(|(_, image)| image.clone())
        } else {
            None
        };
        if let Some(image) = image {
            // GPUI's parent overflow mask is rectangular. The image primitive
            // needs its own radius; avatar decode supplies an undistorted square.
            return Some(
                base.child(
                    img(image)
                        .size_full()
                        .rounded(extent / 3.)
                        .object_fit(ObjectFit::Cover),
                ),
            );
        }
        let emoji = asset.emoji.as_ref()?;
        let bg = asset
            .color
            .as_ref()
            .and_then(|c| c.as_str().strip_prefix('#'))
            .filter(|c| c.len() == 6)
            .and_then(|c| u32::from_str_radix(c, 16).ok())
            .map(|c| gpui::rgb(c).into())
            .unwrap_or(self.theme.selected);
        Some(
            base.bg(bg)
                .text_size(extent * 0.56)
                .child(emoji.as_str().to_owned()),
        )
    }

    pub(super) fn content_action(
        &mut self,
        action: Action,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        match action {
            Action::Menu => self.toggle_composer_menu(window, cx),
            Action::ComposerChoice(target, choice) => {
                self.choose_composer_menu(target, choice, window, cx);
            }
            Action::CreatePoll => {
                self.content_ui.return_overlay = self.overlay;
                self.content_ui.menu = false;
                self.handle(LocalAction::Open(Overlay::CreatePoll), window, cx);
                self.content_ui.question.focus_handle(cx).focus(window);
            }
            Action::AddOption => {
                self.content_ui.option_count =
                    (self.content_ui.option_count + 1).min(MAX_POLL_OPTIONS)
            }
            Action::RemoveOption => {
                self.content_ui.option_count = self.content_ui.option_count.saturating_sub(1).max(2)
            }
            Action::Multiple => self.content_ui.multiple = !self.content_ui.multiple,
            Action::HiddenResults => {
                self.content_ui.hidden_results = !self.content_ui.hidden_results
            }
            Action::SubmitPoll => {
                if let Some(draft) = self.poll_draft(cx)
                    && self.content_scope_current()
                    && self.snapshot.content_panel.poll_create.enabled()
                    && !self.poll_submission_blocked()
                    && let Some(context) = self.snapshot.selected
                    && let Some(request) = self.submit_content(ContentAction::CreatePoll(draft))
                {
                    self.content_ui.pending_poll = Some((context, request));
                }
            }
            Action::Preview(reference) => self.open_media_preview(reference, window, cx),
            Action::GalleryMove { from, offset } => {
                if self.content_ui.preview.as_ref() == Some(&from)
                    && let Some(next) = gallery::adjacent(&self.content_ui.gallery, &from, offset)
                {
                    self.open_media_preview(next, window, cx);
                }
            }
            Action::Download(reference) => {
                if let Some(object) = self.attachment_object(&reference) {
                    let name = Text::new(object.name.as_str())
                        .unwrap_or_else(|_| Text::new("download").expect("constant"));
                    self.submit_message(talk_core::ui::messaging::MessageAction::Download {
                        message: reference.message,
                        placeholder: reference.placeholder,
                        name,
                    });
                }
            }
            Action::Command(command) => {
                self.submit_content(command);
            }
        }
        cx.notify();
    }

    pub(super) fn close_content_overlay(&mut self) -> bool {
        if !matches!(
            self.overlay,
            Some(Overlay::ContentPreview | Overlay::CreatePoll)
        ) {
            return false;
        }
        if self.overlay == Some(Overlay::ContentPreview) {
            if !self.snapshot.selected.is_some_and(|c| {
                c.account.runtime.as_local() == 0 || c.conversation.host.as_local() == 0
            }) {
                self.submit_content(ContentAction::ClosePreview);
            }
            self.content_ui.preview = None;
        }
        self.overlay = self.content_ui.return_overlay.take();
        self.pending_modal_focus = self.overlay.is_some();
        true
    }

    fn attachment_object(
        &self,
        reference: &AttachmentRef,
    ) -> Option<&talk_core::ui::messaging::RichObject> {
        if self.snapshot.selected != Some(reference.context) {
            return None;
        }
        self.snapshot
            .messages
            .as_slice()
            .iter()
            .chain(
                self.snapshot
                    .message_panel
                    .as_ref()
                    .filter(|p| p.context == reference.context)
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
            .filter(|m| !m.metadata.deleted)?
            .metadata
            .rich
            .as_slice()
            .iter()
            .find(|o| o.placeholder == reference.placeholder && o.kind.as_str() == "file")
    }
    pub(super) fn download_available(&self) -> Availability {
        if self
            .snapshot
            .content_panel
            .transfer
            .as_ref()
            .is_some_and(|transfer| {
                matches!(
                    transfer.state,
                    DownloadState::Transferring | DownloadState::Committing
                )
            })
        {
            Availability::pending()
        } else {
            self.message_available(talk_core::ui::messaging::MessageFeature::Files)
        }
    }
    pub(super) fn download_status(&self, cx: &Context<Self>) -> Option<Div> {
        let transfer = self.snapshot.content_panel.transfer.as_ref()?;
        if Some(transfer.attachment.context) != self.snapshot.selected {
            return None;
        }
        let label = match transfer.state {
            DownloadState::Transferring => "Datei wird heruntergeladen …",
            DownloadState::Committing => "Datei wird gespeichert …",
            DownloadState::Saved => "Datei gespeichert.",
            DownloadState::Cancelled => "Download abgebrochen.",
            DownloadState::Failed(_) => "Download fehlgeschlagen. Erneut versuchen.",
        };
        let active = matches!(
            transfer.state,
            DownloadState::Transferring | DownloadState::Committing
        );
        let fraction = transfer
            .total
            .filter(|total| *total > 0)
            .map(|total| (transfer.received.min(total) as f64 / total as f64) as f32);
        let status = div()
            .flex()
            .flex_col()
            .gap_2()
            .child(self.hint(transfer.name.as_str()))
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap_3()
                    .child(self.hint(self.l(label)))
                    .when(active, |d| {
                        d.child(self.button(
                            "cancel-content-download",
                            "Abbrechen",
                            ButtonStyle::Quiet,
                            true,
                            LocalAction::Content(Action::Command(ContentAction::CancelDownload)),
                            cx,
                        ))
                    }),
            )
            .when(active, |d| {
                d.child(self.hint(&match transfer.total {
                    Some(total) => format!("{} / {} KiB", transfer.received / 1024, total / 1024),
                    None => format!("{} KiB", transfer.received / 1024),
                }))
            })
            .when_some(fraction.filter(|_| active), |d, fraction| {
                d.child(
                    div()
                        .h(px(4.))
                        .rounded_full()
                        .overflow_hidden()
                        .bg(self.theme.border)
                        .child(
                            div()
                                .h_full()
                                .w(gpui::relative(fraction))
                                .bg(self.theme.accent),
                        ),
                )
            });
        Some(status)
    }

    fn poll_draft(&self, cx: &Context<Self>) -> Option<PollDraft> {
        poll_draft_values(
            self.content_ui.question.read(cx).content.as_ref(),
            self.content_ui
                .options
                .iter()
                .take(self.content_ui.option_count)
                .map(|input| input.read(cx).content.as_ref()),
            self.content_ui.multiple,
            self.content_ui.hidden_results,
        )
    }
    pub(super) fn poll_creation_body(&self, cx: &Context<Self>) -> Div {
        let busy = self.poll_submission_blocked();
        let available =
            self.content_scope_current() && self.snapshot.content_panel.poll_create.enabled();
        let mut body = div()
            .flex()
            .flex_col()
            .gap_3()
            .child(self.title(self.l("Frage der Umfrage")))
            .child(
                self.input_surface(&self.content_ui.question, cx)
                    .child(self.content_ui.question.clone()),
            );
        for (index, input) in self
            .content_ui
            .options
            .iter()
            .take(self.content_ui.option_count)
            .enumerate()
        {
            body = body.child(
                div()
                    .flex()
                    .items_center()
                    .gap_2()
                    .child(self.hint(&(index + 1).to_string()))
                    .child(
                        self.input_surface(input, cx)
                            .flex_1()
                            .min_w_0()
                            .child(input.clone()),
                    ),
            );
        }
        body = body
            .child(
                div()
                    .flex()
                    .flex_wrap()
                    .gap_2()
                    .child(self.button(
                        "poll-add-option",
                        "Option hinzufügen",
                        ButtonStyle::Quiet,
                        !busy && self.content_ui.option_count < MAX_POLL_OPTIONS,
                        LocalAction::Content(Action::AddOption),
                        cx,
                    ))
                    .child(self.button(
                        "poll-remove-option",
                        "Letzte Option entfernen",
                        ButtonStyle::Quiet,
                        !busy && self.content_ui.option_count > 2,
                        LocalAction::Content(Action::RemoveOption),
                        cx,
                    )),
            )
            .child(self.button_with_icon(
                "poll-multiple",
                (
                    "Mehrere Antworten erlauben",
                    if self.content_ui.multiple {
                        "check"
                    } else {
                        "plus"
                    },
                ),
                ButtonStyle::Quiet,
                !busy,
                LocalAction::Content(Action::Multiple),
                cx,
            ))
            .child(self.button_with_icon(
                "poll-hidden",
                (
                    "Ergebnisse bis zum Abschluss verbergen",
                    if self.content_ui.hidden_results {
                        "check"
                    } else {
                        "plus"
                    },
                ),
                ButtonStyle::Quiet,
                !busy,
                LocalAction::Content(Action::HiddenResults),
                cx,
            ))
            .child(self.hint(self.l("Mindestens zwei unterschiedliche Antwortoptionen eingeben.")));
        if !available {
            body = body.child(self.hint(self.l(super::conversation_dialog::unavailable(
                self.snapshot.content_panel.poll_create,
            ))));
        }
        body = body.child(
            self.button(
                "poll-submit",
                "Umfrage veröffentlichen",
                ButtonStyle::Primary,
                if poll_creation_pending(
                    self.content_ui.pending_poll,
                    self.snapshot
                        .selected
                        .filter(|_| self.content_scope_current()),
                    self.snapshot.content_panel.poll_state,
                ) {
                    Availability::pending()
                } else {
                    (available && !busy && self.poll_draft(cx).is_some()).into()
                },
                LocalAction::Content(Action::SubmitPoll),
                cx,
            ),
        );
        let status = match self.snapshot.content_panel.poll_state {
            RemoteState::Failed(_, _) => {
                Some("Die Umfrage konnte nicht gespeichert werden. Dein Entwurf bleibt erhalten.")
            }
            RemoteState::Uncertain(_) => {
                Some("Ergebnis unklar. Prüfe das Gespräch vor erneutem Veröffentlichen.")
            }
            RemoteState::Idle | RemoteState::Loading(_) | RemoteState::Saving(_) => None,
        };
        body.when_some(status, |d, s| d.child(self.hint(self.l(s))))
    }
    pub(super) fn poll_card(&self, id: u64, cx: &Context<Self>) -> Div {
        let Some(poll) = self
            .snapshot
            .content_panel
            .polls
            .as_slice()
            .iter()
            .find(|p| self.content_scope_current() && p.id == id)
        else {
            return div().child(self.button(
                ("load-poll", id as usize),
                "Umfrage öffnen",
                ButtonStyle::Secondary,
                true,
                LocalAction::Content(Action::Command(ContentAction::RefreshPoll(id))),
                cx,
            ));
        };
        self.loaded_poll(poll, cx)
    }
    fn loaded_poll(&self, poll: &PollView, cx: &Context<Self>) -> Div {
        let mut body = div()
            .flex()
            .flex_col()
            .gap_2()
            .p_3()
            .bg(self.theme.subtle)
            .rounded(px(8.))
            .child(self.title(poll.question.as_str()));
        for option in poll.options.as_slice() {
            let choices = toggled_poll_votes(
                poll.max_votes,
                poll.options
                    .as_slice()
                    .iter()
                    .filter(|o| o.own)
                    .map(|o| o.id),
                option.id,
            );
            let enabled = poll.vote.enabled()
                && !poll.closed
                && !self.poll_submission_blocked()
                && choices.is_some();
            body = body.child(
                self.button(
                    (
                        "poll-option",
                        ((poll.id as usize) << 8) ^ usize::from(option.id),
                    ),
                    "",
                    ButtonStyle::Quiet,
                    enabled,
                    LocalAction::Content(Action::Command(ContentAction::Vote {
                        poll: poll.id,
                        options: choices.unwrap_or_else(Page::empty),
                    })),
                    cx,
                )
                .w_full()
                .justify_start()
                .child(self.scaled_icon(
                    if option.own { "check-circle" } else { "plus" },
                    16.,
                    self.theme.accent,
                ))
                .child(div().flex_1().child(option.label.as_str().to_owned()))
                .when_some(option.votes, |d, votes| {
                    d.child(self.hint(&votes.to_string()))
                }),
            );
        }
        if poll.closed {
            body = body.child(self.hint(self.l("Umfrage beendet.")));
        } else if poll.hidden_results {
            body = body.child(self.hint(self.l("Ergebnisse werden nach Abschluss angezeigt.")));
        }
        if poll.closed && !poll.hidden_results {
            for voter in poll.details.as_slice() {
                if let Some(option) = poll
                    .options
                    .as_slice()
                    .iter()
                    .find(|o| o.id == voter.option)
                {
                    body = body.child(self.hint(&format!(
                        "{} · {}",
                        voter.name.as_str(),
                        option.label.as_str()
                    )));
                }
            }
        }
        body.when(!poll.closed && poll.close.enabled(), |d| {
            d.child(self.button(
                ("close-poll", poll.id as usize),
                "Umfrage beenden",
                ButtonStyle::Quiet,
                !self.poll_submission_blocked(),
                LocalAction::Content(Action::Command(ContentAction::ClosePoll(poll.id))),
                cx,
            ))
        })
    }
}

fn poll_creation_pending(
    pending: Option<(ConversationRef, RequestId)>,
    current: Option<ConversationRef>,
    state: RemoteState,
) -> bool {
    pending.is_some_and(|(context, request)| {
        Some(context) == current && state == RemoteState::Saving(request)
    })
}

fn placeholder_key(value: &str) -> u128 {
    value.bytes().fold(0u128, |hash, byte| {
        hash.wrapping_mul(131).wrapping_add(u128::from(byte))
    })
}

/// A new message may reuse already visible pixels while its own fresh content
/// request runs. Only an exact canonical actor key in this same authorized room
/// qualifies; missing/unknown identities and terminal refusals keep the fallback.
/// This projection starts no work and cannot skip protocol revalidation.
fn avatar_alias<'a>(
    snapshot: &'a PresentationSnapshot,
    target: &AssetTarget,
) -> Option<&'a AssetView> {
    let AssetTarget::MessageAvatar { context, message } = target else {
        return None;
    };
    if snapshot.active_account != Some(context.account)
        || snapshot.content_panel.account != snapshot.active_account
        || snapshot.selected != Some(*context)
        || snapshot.content_panel.context != Some(*context)
        || !matches!(
            snapshot.content,
            ContentState::Ready | ContentState::Offline
        )
        || !snapshot
            .conversations
            .as_slice()
            .iter()
            .any(|r| r.context == *context && r.draft_clipboard == talk_core::ui::Decision::Allowed)
    {
        return None;
    }
    message_avatar_alias(
        snapshot.content_panel.assets.as_slice(),
        snapshot.messages.as_slice(),
        *context,
        *message,
    )
}

fn message_avatar_alias<'a>(
    assets: &'a [AssetView],
    rows: &[talk_core::ui::presentation::MessageView],
    context: ConversationRef,
    message: u128,
) -> Option<&'a AssetView> {
    let target = AssetTarget::MessageAvatar { context, message };
    if assets
        .iter()
        .any(|a| a.target == target && a.state != AssetState::Loading)
    {
        return None;
    }
    let actor = rows
        .iter()
        .find(|m| m.local_id == message)?
        .metadata
        .author_key
        .as_ref()?;
    assets.iter().find(|a| {
        let AssetTarget::MessageAvatar {
            context: source_context,
            message: source,
        } = &a.target
        else {
            return false;
        };
        *source_context == context
            && matches!(a.state, AssetState::Ready { .. })
            && rows
                .iter()
                .any(|m| m.local_id == *source && m.metadata.author_key.as_ref() == Some(actor))
    })
}

fn toggled_poll_votes(
    max_votes: u16,
    own: impl IntoIterator<Item = u16>,
    id: u16,
) -> Option<Page<u16, 100>> {
    // Nextcloud uses zero for unlimited votes, one for a single choice.
    let mut choices = Page::empty();
    let mut selected = false;
    for previous in own.into_iter().take(100) {
        if previous == id {
            selected = true;
        } else if max_votes != 1 {
            choices.try_push(previous).ok()?;
        }
    }
    if !selected {
        choices.try_push(id).ok()?;
    }
    (max_votes == 0 || choices.as_slice().len() <= usize::from(max_votes)).then_some(choices)
}

fn poll_draft_values<'a>(
    question: &str,
    values: impl IntoIterator<Item = &'a str>,
    multiple: bool,
    hidden_results: bool,
) -> Option<PollDraft> {
    let question = question.trim();
    if question.is_empty() {
        return None;
    }
    let mut options = Page::empty();
    for value in values {
        let value = value.trim();
        if options.as_slice().len() == MAX_POLL_OPTIONS
            || value.is_empty()
            || options
                .as_slice()
                .iter()
                .any(|o: &Text<1000>| o.as_str() == value)
        {
            return None;
        }
        options.try_push(Text::new(value).ok()?).ok()?;
    }
    if options.as_slice().len() < 2 {
        return None;
    }
    Some(PollDraft {
        question: Text::new(question).ok()?,
        options,
        multiple,
        hidden_results,
    })
}

#[cfg(test)]
mod tests {
    use super::{
        message_avatar_alias, poll_creation_pending, poll_draft_values, toggled_poll_votes,
    };
    use talk_core::ui::content::{AssetState, AssetTarget, AssetView};
    use talk_core::ui::conversation::RemoteState;
    use talk_core::ui::presentation::{ConversationRef, DeliveryState, MessageView, RequestId};
    use talk_core::ui::{
        AccountId, AccountScope, ConversationId, HostId, Page, RuntimeGeneration, Text,
    };

    fn avatar_message(id: u128, key: Option<&str>) -> MessageView {
        let metadata = talk_core::ui::messaging::MessageMetadata {
            author_key: key.map(|key| Text::new(key).unwrap()),
            ..Default::default()
        };
        MessageView {
            local_id: id,
            author: Text::new("Same display name").unwrap(),
            initials: Text::new("SD").unwrap(),
            time_label: Text::new("09:41").unwrap(),
            body: Text::new("Synthetic message").unwrap(),
            own: false,
            delivery: DeliveryState::Received,
            attachments: Page::empty(),
            reply_count: 0,
            metadata,
        }
    }

    #[test]
    fn current_room_avatar_alias_uses_canonical_identity_and_never_overrides_refusal() {
        let account = AccountId::from_local(1);
        let context = ConversationRef {
            account: AccountScope {
                account,
                runtime: RuntimeGeneration::from_local(1),
            },
            conversation: ConversationId {
                account,
                host: HostId::from_local(1),
                local: 1,
            },
        };
        let source = AssetView {
            target: AssetTarget::MessageAvatar {
                context,
                message: 1,
            },
            state: AssetState::Ready { key: 7 },
            emoji: None,
            color: None,
            document_pages: None,
        };
        let mut rows = [
            avatar_message(1, Some("5:users:synthetic")),
            avatar_message(2, Some("5:users:synthetic")),
        ];
        assert_eq!(
            message_avatar_alias(std::slice::from_ref(&source), &rows, context, 2),
            Some(&source)
        );
        rows[1].metadata.author_key = None;
        assert!(message_avatar_alias(std::slice::from_ref(&source), &rows, context, 2).is_none());
        rows[1].metadata.author_key = Some(Text::new("6:guests:synthetic").unwrap());
        assert!(
            message_avatar_alias(std::slice::from_ref(&source), &rows, context, 2).is_none(),
            "equal labels or IDs in another actor namespace are not identity"
        );
        rows[1].metadata.author_key = rows[0].metadata.author_key.clone();
        let mut other = context;
        other.account.runtime = RuntimeGeneration::from_local(2);
        assert!(message_avatar_alias(std::slice::from_ref(&source), &rows, other, 2).is_none());
        for state in [
            AssetState::Denied,
            AssetState::Unsupported,
            AssetState::Missing,
            AssetState::Failed,
        ] {
            let target = AssetView {
                target: AssetTarget::MessageAvatar {
                    context,
                    message: 2,
                },
                state,
                emoji: None,
                color: None,
                document_pages: None,
            };
            assert!(message_avatar_alias(&[source.clone(), target], &rows, context, 2).is_none());
        }
        rows[0].metadata.deleted = true;
        rows[1].metadata.deleted = true;
        assert_eq!(
            message_avatar_alias(std::slice::from_ref(&source), &rows, context, 2),
            Some(&source),
            "tombstones retain a known actor's already admitted image without restoring message content"
        );
    }

    #[test]
    fn poll_votes_preserve_unlimited_selections_and_enforce_single_or_limited_modes() {
        assert_eq!(
            toggled_poll_votes(0, [0, 2], 1).unwrap().as_slice(),
            &[0, 2, 1]
        );
        assert_eq!(toggled_poll_votes(1, [0], 1).unwrap().as_slice(), &[1]);
        assert!(toggled_poll_votes(1, [1], 1).unwrap().as_slice().is_empty());
        assert!(toggled_poll_votes(2, [0, 2], 1).is_none());
        assert_eq!(toggled_poll_votes(2, [0, 2], 2).unwrap().as_slice(), &[0]);
    }

    #[test]
    fn poll_draft_requires_distinct_bounded_options_and_preserves_modes() {
        let draft = poll_draft_values("  ¿Cuándo?  ", ["  Hoy", "Mañana  "], true, true).unwrap();
        assert_eq!(draft.question.as_str(), "¿Cuándo?");
        assert_eq!(draft.options.as_slice()[0].as_str(), "Hoy");
        assert_eq!(draft.options.as_slice()[1].as_str(), "Mañana");
        assert!(draft.multiple && draft.hidden_results);
        assert!(poll_draft_values("Q", ["Same", " Same "], false, false).is_none());
        assert!(poll_draft_values("Q", ["One"], false, false).is_none());
        assert!(poll_draft_values(" ", ["One", "Two"], false, false).is_none());
        assert!(poll_draft_values("Q", ["One", "  "], false, false).is_none());
        let oversized = "a".repeat(1001);
        assert!(poll_draft_values("Q", [oversized.as_str(), "Two"], false, false).is_none());
        assert!(
            poll_draft_values(
                "Q",
                ["1", "2", "3", "4", "5", "6", "7", "8", "9"],
                false,
                false
            )
            .is_none()
        );
    }

    #[test]
    fn poll_creation_progress_requires_its_admitted_save_in_the_current_scope() {
        let account = AccountId::from_local(1);
        let context = ConversationRef {
            account: AccountScope {
                account,
                runtime: RuntimeGeneration::from_local(1),
            },
            conversation: ConversationId {
                account,
                host: HostId::from_local(1),
                local: 1,
            },
        };
        let request = RequestId(7);
        let pending = Some((context, request));
        assert!(poll_creation_pending(
            pending,
            Some(context),
            RemoteState::Saving(request),
        ));
        for state in [
            RemoteState::Idle,
            RemoteState::Loading(request),
            RemoteState::Saving(RequestId(8)),
            RemoteState::Failed(request, talk_core::ui::UnavailableReason::PermissionDenied),
            RemoteState::Uncertain(request),
        ] {
            assert!(!poll_creation_pending(pending, Some(context), state));
        }
        assert!(!poll_creation_pending(
            None,
            Some(context),
            RemoteState::Saving(request),
        ));
        assert!(!poll_creation_pending(
            pending,
            None,
            RemoteState::Saving(request)
        ));
        let mut replaced = context;
        replaced.account.runtime = RuntimeGeneration::from_local(2);
        assert!(!poll_creation_pending(
            pending,
            Some(replaced),
            RemoteState::Saving(request),
        ));
        replaced = context;
        replaced.conversation.local = 2;
        assert!(!poll_creation_pending(
            pending,
            Some(replaced),
            RemoteState::Saving(request),
        ));
    }
}
