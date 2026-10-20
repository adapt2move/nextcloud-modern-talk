//! Native content surfaces over scoped observations and composition-owned images.
//! Views never resolve a URL, decode remote bytes, or report transfer success.
use super::{
    ButtonStyle, Desktop, LocalAction, Overlay,
    controls::{Availability, scoped_id},
    input::TextInput,
};
use gpui::{
    Context, Div, Entity, Focusable, ImageSource, ObjectFit, Window, div, img, prelude::*, px,
};
use talk_core::ui::content::{
    AssetState, AssetTarget, AttachmentRef, ContentAction, DownloadState, PollDraft, PollView,
};
use talk_core::ui::presentation::{Completion, ConversationRef, RequestId, UiIntent};
use talk_core::ui::{AccountScope, Page, Text, conversation::RemoteState};

const MAX_IMAGES: usize = 128;
const MAX_POLL_OPTIONS: usize = 8;

#[derive(Clone)]
pub(crate) enum Action {
    Menu,
    CreatePoll,
    AddOption,
    RemoveOption,
    Multiple,
    HiddenResults,
    SubmitPoll,
    Preview(AttachmentRef),
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
    preview: Option<AttachmentRef>,
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
            preview: None,
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

    /// One missing asset per existing owner tick. The owner may reject Busy;
    /// only admitted requests are retained, and no render callback starts work.
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
                if targets.len() < MAX_IMAGES {
                    targets.push(AssetTarget::MessageAvatar {
                        context,
                        message: message.local_id,
                    });
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
                    action: ContentAction::Load(target.clone()),
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
    fn content_scope_current(&self) -> bool {
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
        if let Some(image) = self.asset_image(&target) {
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
        let asset = self.asset(&target)?;
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
            Action::Menu => self.content_ui.menu = !self.content_ui.menu,
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
            Action::Preview(reference) => {
                if self.snapshot.selected == Some(reference.context)
                    && self
                        .submit_content(ContentAction::Preview(reference.clone()))
                        .is_some()
                {
                    self.content_ui.preview = Some(reference);
                    self.content_ui.return_overlay = self.overlay;
                    self.handle(LocalAction::Open(Overlay::ContentPreview), window, cx);
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
            self.submit_content(ContentAction::ClosePreview);
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
                    .into_iter()
                    .flat_map(|p| p.shared_files.as_slice()),
            )
            .find(|m| m.local_id == reference.message)?
            .metadata
            .rich
            .as_slice()
            .iter()
            .find(|o| o.placeholder == reference.placeholder && o.kind.as_str() == "file")
    }
    pub(super) fn attachment_card(
        &self,
        reference: AttachmentRef,
        object: &talk_core::ui::messaging::RichObject,
        cx: &Context<Self>,
    ) -> Div {
        let target = AssetTarget::Thumbnail(reference.clone());
        let preview = self.asset_image(&target);
        div().flex().flex_col().gap_2().max_w(px(400.)).child(
            self.button(
                scoped_id(
                    "content-preview",
                    reference.context,
                    reference.message,
                    placeholder_key(reference.placeholder.as_str()),
                ),
                "",
                ButtonStyle::Quiet,
                true,
                LocalAction::Content(Action::Preview(reference.clone())),
                cx,
            )
            .w_full()
            .p_0()
            .overflow_hidden()
            .flex_col()
            .items_start()
            .child(
                div()
                    .w_full()
                    .h(px(180.))
                    .bg(self.theme.subtle)
                    .rounded(px(8.))
                    .flex()
                    .items_center()
                    .justify_center()
                    .child(if let Some(image) = preview {
                        img(image)
                            .size_full()
                            .object_fit(ObjectFit::Cover)
                            .into_any_element()
                    } else {
                        self.scaled_icon("file", 38., self.theme.muted)
                            .into_any_element()
                    }),
            )
            .child(
                div()
                    .w_full()
                    .flex()
                    .items_center()
                    .gap_2()
                    .p_3()
                    .bg(self.theme.subtle)
                    .child(self.scaled_icon("file", 20., self.theme.muted))
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .child(div().text_ellipsis().child(object.name.as_str().to_owned()))
                            .child(
                                self.hint(
                                    object
                                        .mime_type
                                        .as_ref()
                                        .map_or(self.l("Datei"), |m| m.as_str()),
                                ),
                            ),
                    ),
            ),
        )
    }

    pub(super) fn preview_body(&self, cx: &Context<Self>) -> Div {
        let Some(reference) = self
            .content_ui
            .preview
            .as_ref()
            .filter(|r| Some(r.context) == self.snapshot.selected)
        else {
            return self.state_note(
                "file",
                "Vorschau nicht verfügbar",
                "Öffne die Datei erneut aus dem Gespräch.",
            );
        };
        let target = AssetTarget::Image(reference.clone());
        let name = self
            .attachment_object(reference)
            .map_or(self.l("Datei"), |o| o.name.as_str());
        let mut body = div().flex().flex_col().gap_4().child(self.title(name));
        body = if let Some(image) = self.asset_image(&target) {
            body.child(
                img(image)
                    .w_full()
                    .h(px(340.))
                    .object_fit(ObjectFit::Contain)
                    .rounded(px(8.)),
            )
        } else {
            let status = match self.asset(&target).map(|a| a.state) {
                None | Some(AssetState::Loading | AssetState::Ready { .. }) => {
                    "Vorschau wird geladen …"
                }
                Some(AssetState::Denied) => "Du hast keinen Zugriff auf diese Vorschau.",
                Some(AssetState::Failed) => "Die Vorschau konnte nicht geladen werden.",
                Some(AssetState::Missing | AssetState::Unsupported) => {
                    "Keine native Vorschau verfügbar. Öffne die Datei in der Cloud-Webansicht."
                }
            };
            body.child(
                div()
                    .min_h(px(180.))
                    .flex()
                    .items_center()
                    .justify_center()
                    .bg(self.theme.subtle)
                    .rounded(px(8.))
                    .p_5()
                    .child(self.hint(self.l(status))),
            )
        };
        body = body.child(
            div()
                .flex()
                .flex_wrap()
                .gap_3()
                .child(self.button_with_icon(
                    "preview-download",
                    ("Herunterladen", "download"),
                    ButtonStyle::Primary,
                    self.message_available(talk_core::ui::messaging::MessageFeature::Files),
                    LocalAction::Content(Action::Download(reference.clone())),
                    cx,
                ))
                .child(self.button_with_icon(
                    "preview-cloud",
                    ("In der Cloud-Webansicht öffnen", "external"),
                    ButtonStyle::Secondary,
                    true,
                    LocalAction::Content(Action::Command(ContentAction::OpenCloud(
                        reference.clone(),
                    ))),
                    cx,
                )),
        );
        body.children(self.download_status(cx))
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

    pub(super) fn composer_more_menu(&self, cx: &Context<Self>) -> Option<Div> {
        if !self.content_ui.menu || !self.composer_active {
            return None;
        }
        Some(
            div()
                .flex()
                .flex_wrap()
                .gap_2()
                .child(self.button_with_icon(
                    "composer-add-file",
                    ("Datei anhängen", "paperclip"),
                    ButtonStyle::Quiet,
                    self.message_available(talk_core::ui::messaging::MessageFeature::Files),
                    LocalAction::Message(super::messaging::Action::Command(
                        talk_core::ui::messaging::MessageAction::PickUpload,
                    )),
                    cx,
                ))
                .child(self.create_poll_button("composer-create-poll", cx)),
        )
    }
    pub(super) fn create_poll_button(
        &self,
        id: &'static str,
        cx: &Context<Self>,
    ) -> gpui::Stateful<Div> {
        self.button_with_icon(
            id,
            ("Umfrage erstellen", "plus"),
            ButtonStyle::Quiet,
            Availability::when(
                self.content_scope_current() && self.snapshot.content_panel.poll_create.enabled(),
                super::conversation_dialog::unavailable(self.snapshot.content_panel.poll_create),
            ),
            LocalAction::Content(Action::CreatePoll),
            cx,
        )
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
            .child(self.content_ui.question.clone());
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
                    .child(div().flex_1().child(input.clone())),
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
        body = body.child(self.button(
            "poll-submit",
            "Umfrage veröffentlichen",
            ButtonStyle::Primary,
            available && !busy && self.poll_draft(cx).is_some(),
            LocalAction::Content(Action::SubmitPoll),
            cx,
        ));
        let status = match self.snapshot.content_panel.poll_state {
            RemoteState::Loading(_) | RemoteState::Saving(_) => Some("Umfrage wird gespeichert …"),
            RemoteState::Failed(_, _) => {
                Some("Die Umfrage konnte nicht gespeichert werden. Dein Entwurf bleibt erhalten.")
            }
            RemoteState::Uncertain(_) => {
                Some("Ergebnis unklar. Prüfe das Gespräch vor erneutem Veröffentlichen.")
            }
            RemoteState::Idle => None,
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

fn placeholder_key(value: &str) -> u128 {
    value.bytes().fold(0u128, |hash, byte| {
        hash.wrapping_mul(131).wrapping_add(u128::from(byte))
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
    use super::{poll_draft_values, toggled_poll_votes};

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
}
