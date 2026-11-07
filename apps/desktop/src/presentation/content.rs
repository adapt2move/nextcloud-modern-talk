//! Native content surfaces over scoped observations and composition-owned images.
//! Views never resolve a URL, decode remote bytes, or report transfer success.
mod browser;
pub(in crate::presentation) mod gallery;
pub(in crate::presentation) mod groups;
mod voice_scrub;
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
const MAX_POLL_OPTIONS: usize = 100;
const MAX_VOICE_LEVELS: usize = 32;
const INITIAL_VOICE_BUCKET_MS: u64 = 250;

// Preserve peaks across the whole recording, coarsening time buckets instead of
// discarding older samples. This keeps both memory and review rendering bounded.
fn record_voice_level(levels: &mut Vec<u16>, bucket_ms: &mut u64, elapsed_ms: u64, level: u16) {
    let mut index = elapsed_ms / *bucket_ms;
    while index >= MAX_VOICE_LEVELS as u64 {
        *levels = levels
            .chunks(2)
            .map(|pair| pair.iter().copied().max().unwrap_or(0))
            .collect();
        *bucket_ms = bucket_ms.saturating_mul(2);
        index = elapsed_ms / *bucket_ms;
    }
    let index = index as usize;
    levels.resize(levels.len().max(index + 1), 0);
    levels[index] = levels[index].max(level.min(1000));
}

#[derive(Clone)]
pub(crate) enum Action {
    OpenCloudBrowser,
    OpenFolderPicker,
    ChooseFolder,
    BrowseDirectory(Text<16384>),
    BrowserBack,
    BrowserClearSearch,
    ToggleBrowserSort,
    BrowserPage(i8),
    BrowserRecent,
    BrowserFavorites,
    BrowserRetry,
    BrowserSort(talk_core::ui::content::FileSort),
    BrowserStageSelection,
    Menu(bool),
    ComposerChoice(composer_menu::Target, composer_menu::Choice),
    CreatePoll,
    AddOption,
    RemoveOption(usize),
    MoveOption {
        index: usize,
        up: bool,
    },
    Multiple,
    VoteLimit {
        increase: bool,
    },
    HiddenResults,
    SubmitPoll,
    SaveDraft,
    ToggleDrafts,
    ToggleDraftMenu(u64),
    TogglePollVoters {
        poll: u64,
        option: u16,
    },
    LoadDraft {
        id: u64,
        edit: bool,
    },
    ConfirmDeleteDraft(u64),
    DeleteDraft(u64),
    SelectPollOption {
        poll: u64,
        option: u16,
    },
    SubmitVote(u64),
    TogglePollMenu(u64),
    ConfirmClosePoll(u64),
    CancelClosePoll,
    ClosePoll(u64),
    PollDetails(u64),
    VoiceStart(ConversationRef),
    VoiceStop(ConversationRef),
    VoiceFinishAndSend(ConversationRef),
    VoiceCancel(ConversationRef),
    VoiceSend(ConversationRef),
    VoiceReviewPlay {
        context: ConversationRef,
        play: bool,
    },
    VoiceReviewSpeed {
        context: ConversationRef,
        speed: u8,
    },
    Preview(AttachmentRef),
    GalleryMove {
        from: AttachmentRef,
        offset: i8,
    },
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
    browser_page: usize,
    browser_folder_picker: bool,
    browser_cursor: Option<Text<16384>>,
    browser_selection: Vec<Text<16384>>,
    browser_anchor: Option<Text<16384>>,
    browser_history: Vec<browser::Location>,
    browser_typeahead: String,
    browser_typeahead_at: Option<std::time::Instant>,
    pub(super) browser_search: Entity<TextInput>,
    browser_search_value: String,
    browser_search_since: Option<std::time::Instant>,
    browser_sort_open: bool,
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
    max_votes: u16,
    hidden_results: bool,
    drafts_open: bool,
    editing_draft: Option<u64>,
    draft_menu: Option<u64>,
    delete_draft: Option<u64>,
    pending_poll: Option<(ConversationRef, RequestId)>,
    acknowledged_result: Option<RequestId>,
    observed_polls: Vec<PollView>,
    selections: Vec<(u64, Page<u16, 100>)>,
    pending_vote: Option<(ConversationRef, RequestId, u64)>,
    vote_error: Option<u64>,
    pending_close: Option<(ConversationRef, RequestId, u64)>,
    close_error: Option<(u64, bool)>,
    confirm_close: Option<u64>,
    poll_menu: Option<u64>,
    details_poll: Option<u64>,
    expanded_voters: Option<(u64, u16)>,
    pub(super) voice: Option<talk_core::ui::content::VoiceSnapshot>,
    pub(super) voice_levels: Vec<u16>,
    voice_level_bucket_ms: u64,
    voice_epoch: u64,
    pub(super) voice_error: Option<&'static str>,
    voice_auto_send: Option<ConversationRef>,
    voice_playback: Option<talk_core::ui::content::VoiceReviewPlayback>,
    voice_scrub: Option<voice_scrub::Scrub>,
}
impl ContentUi {
    pub fn new(
        theme: super::Theme,
        account: Option<AccountScope>,
        context: Option<ConversationRef>,
        cx: &mut Context<Desktop>,
    ) -> Self {
        let browser_search = cx.new(|cx| TextInput::new("Dateien suchen …", 256, theme, cx));
        Self {
            account,
            context,
            images: Vec::new(),
            requested: Vec::new(),
            request: 0,
            menu: false,
            browser_page: 0,
            browser_folder_picker: false,
            browser_cursor: None,
            browser_selection: Vec::new(),
            browser_anchor: None,
            browser_history: Vec::new(),
            browser_typeahead: String::new(),
            browser_typeahead_at: None,
            browser_search,
            browser_search_value: String::new(),
            browser_search_since: None,
            browser_sort_open: false,
            composer_menu: composer_menu::Menu::new(cx),
            preview: None,
            gallery: Vec::new(),
            gallery_position: None,
            media_groups: Vec::new(),
            swipe: gallery::Swipe::default(),
            return_overlay: None,
            question: cx.new(|cx| TextInput::new("Frage der Umfrage", 4000, theme, cx)),
            options: (0..2)
                .map(|_| cx.new(|cx| TextInput::new("Antwortoption", 1000, theme, cx)))
                .collect(),
            option_count: 2,
            multiple: false,
            max_votes: 1,
            hidden_results: false,
            drafts_open: false,
            editing_draft: None,
            draft_menu: None,
            delete_draft: None,
            pending_poll: None,
            acknowledged_result: None,
            observed_polls: Vec::new(),
            selections: Vec::new(),
            pending_vote: None,
            vote_error: None,
            pending_close: None,
            close_error: None,
            confirm_close: None,
            poll_menu: None,
            details_poll: None,
            expanded_voters: None,
            voice: None,
            voice_levels: Vec::new(),
            voice_level_bucket_ms: INITIAL_VOICE_BUCKET_MS,
            voice_epoch: 0,
            voice_error: None,
            voice_auto_send: None,
            voice_playback: None,
            voice_scrub: None,
        }
    }
    pub fn inputs(&self) -> impl Iterator<Item = &Entity<TextInput>> {
        std::iter::once(&self.question).chain(self.options.iter())
    }
}
impl Desktop {
    /// Only disconnected development scenarios call this after installing
    /// their fictional snapshot; it never opens a server or a media device.
    pub(crate) fn configure_content_scenario(
        &mut self,
        scenario: Option<&str>,
        cx: &mut Context<Self>,
    ) {
        match scenario {
            Some("chat-polls") => {
                let mut selection = Page::empty();
                let _ = selection.try_push(1);
                self.content_ui.selections.push((9001, selection));
            }
            Some("poll-drafts") => self.content_ui.drafts_open = true,
            Some("poll-details") => self.content_ui.details_poll = Some(9001),
            Some("poll-settings") => self.open_room_dialog(Overlay::RoomInfo, cx),
            // Explicit UI fixtures use sample data; real sessions use recorder levels.
            Some("voice-recording" | "voice-review") => {
                self.content_ui.voice = self.settings_port.voice_snapshot();
                self.content_ui.voice_epoch = self.content_ui.voice_epoch.saturating_add(1);
                self.content_ui.voice_levels = vec![
                    90, 200, 360, 580, 370, 170, 220, 460, 710, 490, 240, 120, 280, 540, 680, 390,
                    180, 130, 310, 620, 450, 200, 90, 150,
                ];
            }
            _ => {}
        }
    }

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
        let mut thumbnail_layout_changed = false;
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
                    thumbnail_layout_changed |= self
                        .snapshot
                        .content_panel
                        .assets
                        .as_slice()
                        .iter()
                        .any(|asset| {
                            matches!(asset.target, AssetTarget::Thumbnail(_))
                                && asset.state == (AssetState::Ready { key })
                        });
                    self.content_ui.images.push((key, image));
                    changed = true;
                }
            }
        }
        if thumbnail_layout_changed {
            // Ready thumbnails change row height with their real aspect ratio.
            // Retire cached list measurements without moving the reader's anchor.
            let anchor = self.message_ui.list.logical_scroll_top();
            let count = self.message_ui.list.item_count();
            self.message_ui.list.splice(0..count, count);
            self.message_ui.list.scroll_to(anchor);
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

    /// At most two missing assets per existing owner tick. The content owner settles
    /// enqueue success with admission, bounded Busy retry or a failed asset;
    /// no render callback starts work.
    pub(super) fn refresh_content_ui(&mut self, cx: &mut Context<Self>) {
        self.refresh_attachment_send(cx);
        if self.overlay == Some(Overlay::CloudFiles) {
            let value = self.content_ui.browser_search.read(cx).content.to_string();
            if value != self.content_ui.browser_search_value {
                self.content_ui.browser_search_value = value;
                self.content_ui.browser_search_since = Some(std::time::Instant::now());
            }
            if self
                .content_ui
                .browser_search_since
                .is_some_and(|since| since.elapsed() >= std::time::Duration::from_millis(250))
            {
                self.content_ui.browser_search_since = None;
                self.content_ui.browser_page = 0;
                self.content_ui.browser_cursor = None;
                self.content_ui.browser_selection.clear();
                self.content_ui.browser_anchor = None;
                let value = self.content_ui.browser_search_value.trim();
                let action = if value.is_empty() {
                    Some(ContentAction::ListFiles(Text::new("/").expect("constant")))
                } else {
                    Text::new(value).ok().map(ContentAction::SearchFiles)
                };
                if !value.is_empty()
                    && self
                        .snapshot
                        .content_panel
                        .file_browser
                        .as_ref()
                        .is_some_and(|browser| {
                            browser.section != talk_core::ui::content::FileSection::Search
                        })
                {
                    self.remember_cloud_location();
                }
                if let Some(action) = action {
                    self.submit_content(action);
                }
            }
        }
        let mut voice = self.settings_port.voice_snapshot();
        if let Some(voice) = voice
            && voice_session_retired(
                Some(voice.context) == self.snapshot.selected,
                self.snapshot.content,
                !self.media_offline(),
                self.message_feature_enabled(talk_core::ui::messaging::MessageFeature::Voice),
            )
        {
            let _ = self.settings_port.stop_voice(voice.context, true);
            self.content_ui.voice_auto_send = None;
        }
        if let Some(context) = self.content_ui.voice_auto_send
            && let Some(current) = voice.filter(|voice| voice.context == context)
        {
            match current.phase {
                talk_core::ui::content::VoicePhase::Review => {
                    self.content_ui.voice_auto_send = None;
                    if self.media_offline() || self.snapshot.content != ContentState::Ready {
                        self.content_ui.voice_error =
                            Some("Verbinde dich erneut, um die Aufnahme zu senden.");
                    } else if let Err(error) = self.settings_port.send_voice(context) {
                        self.content_ui.voice_error = Some(voice_error(error));
                    } else {
                        self.content_ui.voice_error = None;
                        voice = None;
                    }
                }
                talk_core::ui::content::VoicePhase::Failed(_) => {
                    self.content_ui.voice_auto_send = None
                }
                _ => {}
            }
        }
        if self
            .content_ui
            .voice_auto_send
            .is_some_and(|context| !voice.is_some_and(|current| current.context == context))
        {
            self.content_ui.voice_auto_send = None;
        }
        let voice = voice.filter(|voice| Some(voice.context) == self.snapshot.selected);
        if self.content_ui.voice != voice {
            if self.content_ui.voice.map(|v| v.context) != voice.map(|v| v.context)
                || voice.is_some_and(|v| {
                    v.phase == talk_core::ui::content::VoicePhase::Starting
                        && self.content_ui.voice.is_none_or(|old| old.phase != v.phase)
                })
            {
                self.content_ui.voice_levels.clear();
                self.content_ui.voice_level_bucket_ms = INITIAL_VOICE_BUCKET_MS;
                self.content_ui.voice_epoch = self.content_ui.voice_epoch.saturating_add(1);
            }
            if let Some(voice) = voice
                && voice.phase == talk_core::ui::content::VoicePhase::Recording
            {
                record_voice_level(
                    &mut self.content_ui.voice_levels,
                    &mut self.content_ui.voice_level_bucket_ms,
                    voice.elapsed_ms,
                    voice.level,
                );
            }
            if voice.is_none() {
                self.content_ui.voice_levels.clear();
            }
            self.content_ui.voice = voice;
            cx.notify();
        }
        let playback = self
            .settings_port
            .voice_review_playback()
            .filter(|p| Some(p.context) == self.snapshot.selected);
        if self.content_ui.voice_playback != playback {
            self.content_ui.voice_playback = playback;
            cx.notify();
        }
        self.retire_voice_scrub();
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
            self.content_ui.options.truncate(2);
            self.content_ui.option_count = 2;
            self.content_ui.multiple = false;
            self.content_ui.max_votes = 1;
            self.content_ui.hidden_results = false;
            self.content_ui.drafts_open = false;
            self.content_ui.editing_draft = None;
            self.content_ui.draft_menu = None;
            self.content_ui.delete_draft = None;
            self.content_ui.selections.clear();
            self.content_ui.pending_vote = None;
            self.content_ui.vote_error = None;
            self.content_ui.pending_close = None;
            self.content_ui.close_error = None;
            self.content_ui.confirm_close = None;
            self.content_ui.poll_menu = None;
            self.content_ui.details_poll = None;
            self.content_ui.expanded_voters = None;
            self.content_ui.return_overlay = None;
            self.content_ui.browser_history.clear();
            self.content_ui.voice_scrub = None;
            self.content_ui.voice_auto_send = None;
            if matches!(
                self.overlay,
                Some(
                    Overlay::ContentPreview
                        | Overlay::CreatePoll
                        | Overlay::PollDetails
                        | Overlay::CloudFiles
                )
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
            let newly_visible_results: Vec<_> = self
                .snapshot
                .content_panel
                .polls
                .as_slice()
                .iter()
                .filter(|poll| {
                    poll.options
                        .as_slice()
                        .iter()
                        .any(|option| option.votes.is_some())
                })
                .filter(|poll| {
                    self.content_ui
                        .observed_polls
                        .iter()
                        .find(|old| old.id == poll.id)
                        .is_some_and(|old| {
                            old.closed != poll.closed
                                || !old
                                    .options
                                    .as_slice()
                                    .iter()
                                    .any(|option| option.votes.is_some())
                        })
                })
                .map(|poll| poll.id)
                .collect();
            for poll in newly_visible_results {
                self.begin_surface_reveal(("poll-results", poll as usize).into());
            }
            let anchor = self.message_ui.list.logical_scroll_top();
            let count = self.message_ui.list.item_count();
            self.message_ui.list.splice(0..count, count);
            self.message_ui.list.scroll_to(anchor);
            self.content_ui.observed_polls = self.snapshot.content_panel.polls.as_slice().to_vec();
        }
        if let Some((request, completion)) = self.snapshot.content_panel.result {
            let vote = self
                .content_ui
                .pending_vote
                .filter(|(context, pending, _)| {
                    self.snapshot.selected == Some(*context) && *pending == request
                });
            if let Some((_, _, poll)) = vote {
                self.content_ui.pending_vote = None;
                if completion == Completion::ServerAccepted {
                    self.content_ui.selections.retain(|(id, _)| *id != poll);
                    self.content_ui.vote_error = None;
                    self.begin_surface_reveal(("poll-results", poll as usize).into());
                } else {
                    self.content_ui.vote_error = Some(poll);
                }
            }
            let close = self
                .content_ui
                .pending_close
                .filter(|(context, pending, _)| {
                    self.snapshot.selected == Some(*context) && *pending == request
                });
            if let Some((_, _, poll)) = close {
                self.content_ui.pending_close = None;
                self.content_ui.close_error = match completion {
                    Completion::ServerAccepted => None,
                    Completion::Uncertain => Some((poll, true)),
                    _ => Some((poll, false)),
                };
            }
            if self
                .content_ui
                .pending_poll
                .is_some_and(|(context, pending)| {
                    self.snapshot.selected == Some(context) && pending == request
                })
                && completion == Completion::ServerAccepted
            {
                self.content_ui.pending_poll = None;
                self.content_ui.editing_draft = None;
                self.content_ui.drafts_open = false;
                self.content_ui.option_count = 2;
                self.content_ui.multiple = false;
                self.content_ui.max_votes = 1;
                self.content_ui.hidden_results = false;
                self.content_ui.question.update(cx, |i, _| i.reset());
                for input in &self.content_ui.options {
                    input.update(cx, |i, _| i.reset());
                }
                self.content_ui.options.truncate(2);
                if self.overlay == Some(Overlay::CreatePoll) {
                    self.overlay = self.content_ui.return_overlay.take();
                    self.pending_modal_focus = self.overlay.is_some();
                }
            }
            if self.content_ui.acknowledged_result != Some(request) {
                if let Completion::Failed(_) = completion
                    && !matches!(self.overlay, Some(Overlay::CreatePoll))
                    && vote.is_none()
                    && close.is_none()
                {
                    self.feedback = Some(super::feedback::Feedback::error(
                        "Die Inhaltsaktion ist fehlgeschlagen. Erneut versuchen.",
                    ));
                }
                if self.submit_content(ContentAction::Acknowledge).is_some() {
                    self.content_ui.acknowledged_result = Some(request);
                }
            }
        }
        if self.snapshot.content_panel.poll_state == RemoteState::Idle
            && let Some((id, uncertain)) = self.content_ui.close_error
            && let Some(poll) = self
                .snapshot
                .content_panel
                .polls
                .as_slice()
                .iter()
                .find(|poll| poll.id == id)
        {
            if poll.closed {
                self.content_ui.close_error = None;
            } else if uncertain {
                self.content_ui.close_error = Some((id, false));
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
                    .filter(|o| o.kind.as_str() == "file" && super::attachments::previewable(o))
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
                        .filter(|r| r.kind.as_str() == "file" && super::attachments::previewable(r))
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
        if self.overlay == Some(Overlay::CloudFiles)
            && !self.media_offline()
            && let Some(browser) = self.snapshot.content_panel.file_browser.as_ref()
        {
            for entry in browser
                .entries
                .as_slice()
                .iter()
                .skip(
                    self.content_ui
                        .browser_page
                        .saturating_mul(browser::PAGE_SIZE),
                )
                .take(browser::PAGE_SIZE)
                .filter(|entry| {
                    !entry.directory
                        && entry.mime.as_ref().is_some_and(|mime| {
                            mime.as_str().starts_with("image/")
                                || mime.as_str() == "application/pdf"
                        })
                })
                .take(8)
            {
                if let Some(file) = entry
                    .file_id
                    .as_ref()
                    .and_then(|id| id.as_str().parse::<u64>().ok())
                    .filter(|id| *id > 0)
                    && targets.len() < MAX_IMAGES
                {
                    targets.push(AssetTarget::CloudFile {
                        account,
                        path: entry.path.clone(),
                        file,
                        generation: browser.generation,
                    });
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
        // Visible attachments get the lane before background avatar decoration.
        targets.sort_by_key(|target| match target {
            AssetTarget::Image(_) => 0,
            AssetTarget::Thumbnail(_) => 1,
            AssetTarget::SelfAvatar => 2,
            _ => 3,
        });
        let missing: Vec<_> = targets
            .into_iter()
            .filter(|target| {
                !self.content_ui.requested.contains(target)
                    && !self
                        .snapshot
                        .content_panel
                        .assets
                        .as_slice()
                        .iter()
                        .any(|a| &a.target == target)
            })
            .take(2)
            .collect();
        for target in missing {
            let Some(next) = self.content_ui.request.checked_add(1) else {
                break;
            };
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
    pub(super) fn voice_composer(
        &self,
        voice: talk_core::ui::content::VoiceSnapshot,
        compact: bool,
        cx: &Context<Self>,
    ) -> Div {
        use talk_core::ui::content::VoicePhase;
        let context = voice.context;
        let time = format!(
            "{:02}:{:02}",
            voice.elapsed_ms / 60_000,
            (voice.elapsed_ms / 1_000) % 60
        );
        let playback = self
            .content_ui
            .voice_playback
            .filter(|p| p.context == context);
        let playing =
            playback.is_some_and(|p| p.state == talk_core::ui::content::PlaybackState::Playing);
        let controllable = playback.is_some_and(|p| {
            matches!(
                p.state,
                talk_core::ui::content::PlaybackState::Paused
                    | talk_core::ui::content::PlaybackState::Playing
                    | talk_core::ui::content::PlaybackState::Ended
            )
        });
        let speed = playback.map_or(2, |p| p.speed_half_steps);
        let mut body = div()
            .flex()
            .items_center()
            .gap_2()
            .child(self.icon_button(
                "voice-cancel",
                ("trash-2", "Aufnahme verwerfen"),
                ButtonStyle::Quiet,
                true.into(),
                LocalAction::Content(Action::VoiceCancel(context)),
                cx,
            ))
            .when(
                matches!(voice.phase, VoicePhase::Starting | VoicePhase::Recording),
                |d| {
                    d.child(self.recording_dot())
                        .child(self.hint(self.l("Aufnahme läuft")))
                        .child(self.hint(&time))
                },
            )
            .when(voice.phase == VoicePhase::Review, |d| {
                d.child(self.icon_button(
                    "voice-review-play",
                    (
                        if playing { "pause" } else { "play" },
                        if playing {
                            "Pausieren"
                        } else {
                            "Aufnahme abspielen"
                        },
                    ),
                    ButtonStyle::Quiet,
                    true.into(),
                    LocalAction::Content(Action::VoiceReviewPlay {
                        context,
                        play: !playing,
                    }),
                    cx,
                ))
            })
            .child(self.voice_review_wave(context, voice.phase == VoicePhase::Review, playback, cx))
            .when(voice.phase == VoicePhase::Review, |d| {
                d.child(self.hint(&time)).child(
                    self.button(
                        "voice-review-speed",
                        format!("{}×", f32::from(speed) / 2.0),
                        ButtonStyle::Secondary,
                        Availability::when(
                            controllable,
                            "Die Aufnahme kann hier nicht abgespielt werden.",
                        ),
                        LocalAction::Content(Action::VoiceReviewSpeed {
                            context,
                            speed: if speed >= 4 { 2 } else { speed + 1 },
                        }),
                        cx,
                    )
                    .min_h(px(28.))
                    .h(px(28.))
                    .px_2()
                    .text_color(self.theme.muted),
                )
            });
        let can_send = Availability::when(
            !self.media_offline()
                && self.snapshot.content == ContentState::Ready
                && self.message_feature_enabled(talk_core::ui::messaging::MessageFeature::Voice),
            "Verbinde dich erneut, um die Aufnahme zu senden.",
        );
        if matches!(voice.phase, VoicePhase::Starting | VoicePhase::Recording) {
            body = body
                .child(self.icon_button(
                    "voice-stop",
                    ("square", "Aufnahme prüfen"),
                    ButtonStyle::StageDanger,
                    (self.content_ui.voice_auto_send.is_none()).into(),
                    LocalAction::Content(Action::VoiceStop(context)),
                    cx,
                ))
                .child(self.compact_round_icon_button(
                    "voice-send-recording",
                    ("arrow-up", "Sprachnachricht senden"),
                    ButtonStyle::Primary,
                    if self.content_ui.voice_auto_send.is_some() {
                        Availability::pending()
                    } else {
                        can_send
                    },
                    LocalAction::Content(Action::VoiceFinishAndSend(context)),
                    cx,
                ));
        } else if voice.phase == VoicePhase::Review {
            body = body.child(self.compact_round_icon_button(
                "voice-send",
                ("arrow-up", "Sprachnachricht senden"),
                ButtonStyle::Primary,
                can_send,
                LocalAction::Content(Action::VoiceSend(context)),
                cx,
            ));
        } else if voice.phase == VoicePhase::Stopping {
            body = body.child(self.hint(self.l("Aufnahme wird abgeschlossen …")));
        }
        div()
            .flex_none()
            .px(if compact { px(12.) } else { px(24.) })
            .pb_4()
            .pt_2()
            .flex().flex_col().gap_2()
            .child(self.typing_slot(context))
            .child(
                self.composer_frame()
                    .justify_center()
                    .child(body)
                    .when(voice.phase == VoicePhase::Review && self.media_offline(), |d| {
                        d.child(self.hint(self.l("Verbinde dich erneut, um die Aufnahme zu senden.")))
                    })
                    .when_some(
                        match voice.phase {
                        VoicePhase::Failed(talk_core::ui::UnavailableReason::PermissionDenied) => Some("Mikrofonzugriff verweigert. Erlaube den Zugriff in den Systemeinstellungen."),
                        VoicePhase::Failed(_) => Some("Mikrofon nicht verfügbar. Prüfe die Geräteauswahl."),
                            _ => self.content_ui.voice_error,
                        },
                        |d, error| d.child(self.hint(self.l(error))),
                    ),
            )
    }
    fn voice_review_wave(
        &self,
        context: ConversationRef,
        review: bool,
        playback: Option<talk_core::ui::content::VoiceReviewPlayback>,
        cx: &Context<Self>,
    ) -> impl IntoElement {
        let duration = playback.and_then(|p| p.duration_ms).unwrap_or(0);
        let epoch = self.content_ui.voice_epoch;
        let position = playback.map_or(0, |p| p.position_ms);
        let enabled = !self.background_inert
            && epoch < u64::MAX
            && review
            && duration > 0
            && playback.is_some_and(|p| {
                matches!(
                    p.state,
                    talk_core::ui::content::PlaybackState::Paused
                        | talk_core::ui::content::PlaybackState::Playing
                        | talk_core::ui::content::PlaybackState::Ended
                )
            });
        let id = scoped_id("voice-review-wave", context, 0, 0);
        let focus = self.control_focus(id.clone(), cx);
        let pointer_focus = focus.clone();
        // Each retained peak spans three thin bars so a long scrubber stays
        // readable without inventing additional audio samples.
        let bars = self.content_ui.voice_levels.len().saturating_mul(3);
        let owner = cx.entity().downgrade();
        let surface = gpui::canvas(
            |bounds, window, _| {
                (
                    bounds.intersect(&window.content_mask().bounds),
                    window.insert_hitbox(bounds, gpui::HitboxBehavior::Normal),
                )
            },
            move |bounds, (clip, hitbox), window, _| {
                if !enabled {
                    return;
                }
                let owner = owner.clone();
                let focus = pointer_focus.clone();
                window.on_mouse_event(move |event: &gpui::MouseDownEvent, phase, window, cx| {
                    if phase != gpui::DispatchPhase::Bubble
                        || event.button != gpui::MouseButton::Left
                        || !hitbox.is_hovered(window)
                        || !clip.contains(&event.position)
                    {
                        return;
                    }
                    let position = voice_scrub::seek_position(bounds, event.position.x, duration);
                    let _ = owner.update(cx, |this, cx| {
                        if this.content_ui.voice_epoch == epoch
                            && this.snapshot.selected == Some(context)
                            && this.content_ui.voice.is_some_and(|v| {
                                v.context == context
                                    && v.phase == talk_core::ui::content::VoicePhase::Review
                            })
                        {
                            if let Err(error) =
                                this.settings_port.voice_review_seek(context, position)
                            {
                                this.content_ui.voice_error = Some(voice_error(error));
                            }
                            cx.notify();
                        }
                    });
                    if let Some(focus) = &focus {
                        focus.focus(window);
                    }
                    window.prevent_default();
                    cx.stop_propagation();
                });
            },
        )
        .absolute()
        .inset_0();
        div()
            .id(id)
            .relative()
            .flex_1()
            .min_w_0()
            .h(px(32.))
            .flex()
            .items_center()
            .justify_between()
            .gap(px(2.))
            .when_some(focus, |d, focus| {
                d.track_focus(&focus.clone().tab_stop(enabled))
                    .child(self.field_ring(focus, 5.))
            })
            .when(review && self.content_ui.voice_levels.is_empty(), |d| {
                d.child(self.hint(self.l("Aufnahme bereit zum Senden")))
            })
            .children((0..bars).map(|index| {
                let level = self.content_ui.voice_levels[index / 3];
                let played = duration > 0
                    && (index as f32 / bars.max(1) as f32) <= position as f32 / duration as f32;
                div()
                    .flex_1()
                    .min_w_0()
                    .max_w(px(3.))
                    .h(px(3. + f32::from(level) / 1000. * 25.))
                    .rounded(px(2.))
                    .bg(if !review || played {
                        if self.theme.dark {
                            gpui::rgb(0x6fa0e8).into()
                        } else {
                            self.theme.accent
                        }
                    } else {
                        self.theme.muted
                    })
            }))
            .child(surface)
            .when(enabled, |d| {
                d.on_key_down(cx.listener(move |this, event: &gpui::KeyDownEvent, _, cx| {
                    let direction = match event.keystroke.key.as_str() {
                        "left" => -1,
                        "right" => 1,
                        _ => return,
                    };
                    if !event.keystroke.modifiers.modified()
                        && this.content_ui.voice_epoch == epoch
                        && this.snapshot.selected == Some(context)
                        && let Some(p) = this
                            .content_ui
                            .voice_playback
                            .filter(|p| p.context == context)
                    {
                        let target =
                            voice_scrub::keyboard_position(p.position_ms, duration, direction);
                        if let Err(error) = this.settings_port.voice_review_seek(context, target) {
                            this.content_ui.voice_error = Some(voice_error(error));
                        }
                        cx.notify();
                        cx.stop_propagation();
                    }
                }))
            })
    }
    fn poll_submission_blocked(&self) -> bool {
        self.snapshot.content_panel.poll_state.pending()
    }
    pub(super) fn voice_message_card(
        &self,
        reference: AttachmentRef,
        object: &talk_core::ui::messaging::RichObject,
        cx: &Context<Self>,
    ) -> gpui::Stateful<Div> {
        use talk_core::ui::content::PlaybackState;
        let playback = self
            .snapshot
            .content_panel
            .voice_playback
            .as_ref()
            .filter(|p| p.attachment == reference);
        let playing = playback.is_some_and(|p| p.state == PlaybackState::Playing);
        let loading = playback
            .is_some_and(|p| matches!(p.state, PlaybackState::Loading | PlaybackState::Stopping));
        let controllable = playback.is_some_and(|p| {
            matches!(
                p.state,
                PlaybackState::Paused | PlaybackState::Playing | PlaybackState::Ended
            )
        });
        let speed = playback.map_or(2, |p| p.speed_half_steps);
        let play = if playing {
            ContentAction::PauseVoice(reference.clone())
        } else {
            ContentAction::PlayVoice(reference.clone())
        };
        let position = self
            .content_ui
            .voice_scrub
            .as_ref()
            .filter(|scrub| scrub.reference == reference)
            .map_or_else(
                || playback.map_or(0, |p| p.position_ms),
                |scrub| scrub.position_ms,
            );
        let duration = playback.and_then(|p| p.duration_ms).unwrap_or(0);
        let elapsed = if loading {
            "…".to_owned()
        } else {
            format!("{:02}:{:02}", position / 60_000, position / 1_000 % 60)
        };
        let failed = playback.is_some_and(|p| matches!(p.state, PlaybackState::Failed(_)));
        let fallback_id = format!("{}:{}", reference.message, reference.placeholder.as_str());
        let source = object
            .id
            .as_ref()
            .map_or(fallback_id.as_str(), |id| id.as_str());
        let group: gpui::SharedString = scoped_id(
            "voice-card",
            reference.context,
            reference.message,
            placeholder_key(reference.placeholder.as_str()),
        )
        .to_string()
        .into();
        let row = div()
            .flex()
            .items_center()
            .gap_2()
            .child(self.compact_round_icon_button(
                scoped_id(
                    "voice-play",
                    reference.context,
                    reference.message,
                    placeholder_key(reference.placeholder.as_str()),
                ),
                (
                    if playing { "pause" } else { "play" },
                    if playing {
                        "Pausieren"
                    } else {
                        "Sprachnachricht abspielen"
                    },
                ),
                ButtonStyle::Primary,
                Availability::when(
                    self.snapshot.content == ContentState::Ready
                        && !self.media_offline()
                        && !loading
                        && self.snapshot.call.is_none(),
                    "Die Verbindung ist derzeit nicht bereit.",
                ),
                LocalAction::Content(Action::Command(play)),
                cx,
            ))
            .child(self.voice_scrub_bar(
                reference.clone(),
                position,
                duration,
                source,
                controllable,
                cx,
            ))
            .child(self.hint(&elapsed))
            .child(
                self.button(
                    scoped_id(
                        "voice-speed",
                        reference.context,
                        reference.message,
                        placeholder_key(reference.placeholder.as_str()),
                    ),
                    format!("{}×", f32::from(speed) / 2.0),
                    ButtonStyle::Quiet,
                    controllable,
                    LocalAction::Content(Action::Command(ContentAction::SpeedVoice {
                        attachment: reference.clone(),
                        speed_half_steps: if speed >= 4 { 2 } else { speed + 1 },
                    })),
                    cx,
                )
                .min_h(px(28.))
                .h(px(28.))
                .px_2()
                .rounded(px(8.))
                .border_1()
                .border_color(self.theme.border),
            )
            .child(
                self.icon_button(
                    scoped_id(
                        "voice-download",
                        reference.context,
                        reference.message,
                        placeholder_key(reference.placeholder.as_str()),
                    ),
                    ("download", "Herunterladen"),
                    ButtonStyle::Quiet,
                    if self.media_offline() {
                        Availability::when(false, "Die Verbindung ist derzeit nicht bereit.")
                    } else {
                        self.download_available()
                    },
                    LocalAction::Content(Action::Download(reference.clone())),
                    cx,
                )
                .opacity(if failed { 1. } else { 0. })
                .group_hover(group.clone(), |s| s.opacity(1.))
                .focus(|s| s.opacity(1.)),
            );
        div()
            .id(scoped_id(
                "voice-card",
                reference.context,
                reference.message,
                placeholder_key(reference.placeholder.as_str()),
            ))
            .group(group)
            .flex()
            .flex_col()
            .w_full()
            .max_w(px(360.))
            .gap_1()
            .p_2()
            .rounded(px(8.))
            .bg(self.theme.subtle)
            .child(row)
            .when(failed, |d| {
                d.child(self.hint(self.l("Wiedergabe nicht verfügbar. Lade die Datei herunter.")))
            })
    }
    pub(in crate::presentation) fn asset_image(&self, target: &AssetTarget) -> Option<ImageSource> {
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
        let base = self.avatar_shape(size).overflow_hidden();
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
                        .rounded(extent * 0.28)
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
        if cx.has_active_drag() && matches!(action, Action::Preview(_) | Action::Download(_)) {
            return;
        }
        // Poll controls can change a variable-height message without changing
        // its snapshot. Invalidate cached row heights while retaining the anchor.
        let poll_layout_changed = matches!(
            &action,
            Action::TogglePollMenu(_)
                | Action::ConfirmClosePoll(_)
                | Action::CancelClosePoll
                | Action::SelectPollOption { .. }
                | Action::SubmitVote(_)
                | Action::ClosePoll(_)
        );
        match action {
            Action::OpenCloudBrowser => {
                self.content_ui.return_overlay = self.overlay;
                self.content_ui.browser_folder_picker = false;
                self.content_ui.browser_page = 0;
                self.content_ui.browser_cursor = None;
                self.content_ui.browser_selection.clear();
                self.content_ui.browser_anchor = None;
                self.content_ui.browser_history.clear();
                self.clear_cloud_search(cx);
                if self
                    .submit_content(ContentAction::ListFiles(Text::new("/").expect("constant")))
                    .is_some()
                {
                    self.handle(LocalAction::Open(Overlay::CloudFiles), window, cx);
                }
            }
            Action::OpenFolderPicker => {
                self.content_ui.return_overlay = self.overlay;
                self.content_ui.browser_folder_picker = true;
                self.content_ui.browser_page = 0;
                self.content_ui.browser_cursor = None;
                self.content_ui.browser_selection.clear();
                self.content_ui.browser_anchor = None;
                self.content_ui.browser_history.clear();
                self.clear_cloud_search(cx);
                let path = self.upload_folder.read(cx).content.to_string();
                let path = if talk_core::preferences::account::NextcloudFolder::new(&path).is_ok() {
                    path
                } else {
                    "/".to_owned()
                };
                if let Ok(path) = Text::new(&path)
                    && self
                        .submit_content(ContentAction::ListFiles(path))
                        .is_some()
                {
                    self.handle(LocalAction::Open(Overlay::CloudFiles), window, cx);
                }
            }
            Action::ChooseFolder => {
                if self.content_ui.browser_folder_picker
                    && let Some(browser) = self.snapshot.content_panel.file_browser.as_ref()
                    && browser.section == talk_core::ui::content::FileSection::All
                    && Some(browser.account) == self.snapshot.active_account
                    && talk_core::preferences::account::NextcloudFolder::new(browser.path.as_str())
                        .is_ok()
                {
                    let path = browser.path.as_str().to_owned();
                    self.upload_folder.update(cx, |input, cx| {
                        input.set_context_value(&path);
                        cx.notify();
                    });
                    self.close_content_overlay();
                }
            }
            Action::BrowseDirectory(path) => {
                let changed = self
                    .snapshot
                    .content_panel
                    .file_browser
                    .as_ref()
                    .is_none_or(|browser| {
                        browser.section != talk_core::ui::content::FileSection::All
                            || browser.path != path
                    });
                if changed {
                    self.remember_cloud_location();
                }
                self.content_ui.browser_page = 0;
                self.content_ui.browser_cursor = None;
                self.content_ui.browser_selection.clear();
                self.content_ui.browser_anchor = None;
                self.clear_cloud_search(cx);
                self.submit_content(ContentAction::ListFiles(path));
            }
            Action::ToggleBrowserSort => {
                self.content_ui.browser_sort_open = !self.content_ui.browser_sort_open
            }
            Action::BrowserClearSearch => {
                self.clear_cloud_search(cx);
                self.submit_content(ContentAction::ListFiles(Text::new("/").expect("constant")));
            }
            Action::BrowserBack => {
                if let Some(location) = self.content_ui.browser_history.pop() {
                    self.content_ui.browser_page = 0;
                    self.content_ui.browser_cursor = None;
                    self.content_ui.browser_selection.clear();
                    self.content_ui.browser_anchor = None;
                    self.clear_cloud_search(cx);
                    let command = match location.clone() {
                        browser::Location::Folder(path) => ContentAction::ListFiles(path),
                        browser::Location::Recent => ContentAction::RecentFiles,
                        browser::Location::Favorites => ContentAction::FavoriteFiles,
                        browser::Location::Search(term) => {
                            self.content_ui.browser_search_value = term.as_str().to_owned();
                            self.content_ui.browser_search.update(cx, |input, cx| {
                                input.set_context_value(term.as_str());
                                cx.notify();
                            });
                            ContentAction::SearchFiles(term)
                        }
                    };
                    if self.submit_content(command).is_none() {
                        self.content_ui.browser_history.push(location);
                    }
                }
            }
            Action::BrowserPage(delta) => {
                if let Some(browser) = self.snapshot.content_panel.file_browser.as_ref()
                    && let Some((offset, total)) = browser.pagination
                {
                    let next = if delta < 0 {
                        offset.saturating_sub(40)
                    } else {
                        offset.saturating_add(40)
                    };
                    if next != offset && next < total && !browser.loading {
                        let path = browser.path.clone();
                        self.content_ui.browser_page = 0;
                        self.content_ui.browser_cursor = None;
                        self.content_ui.browser_selection.clear();
                        self.content_ui.browser_anchor = None;
                        self.submit_content(ContentAction::ListFilesPage { path, offset: next });
                    }
                    cx.notify();
                    return;
                }
                let count = self
                    .snapshot
                    .content_panel
                    .file_browser
                    .as_ref()
                    .map_or(0, |b| b.entries.as_slice().len());
                let last = count.saturating_sub(1) / browser::PAGE_SIZE;
                self.content_ui.browser_page = if delta < 0 {
                    self.content_ui.browser_page.saturating_sub(1)
                } else {
                    (self.content_ui.browser_page + 1).min(last)
                };
            }
            Action::BrowserRecent | Action::BrowserFavorites => {
                let desired = if matches!(action, Action::BrowserRecent) {
                    talk_core::ui::content::FileSection::Recent
                } else {
                    talk_core::ui::content::FileSection::Favorites
                };
                if self
                    .snapshot
                    .content_panel
                    .file_browser
                    .as_ref()
                    .is_none_or(|browser| browser.section != desired)
                {
                    self.remember_cloud_location();
                }
                self.content_ui.browser_page = 0;
                self.content_ui.browser_cursor = None;
                self.content_ui.browser_selection.clear();
                self.content_ui.browser_anchor = None;
                self.clear_cloud_search(cx);
                self.submit_content(if matches!(action, Action::BrowserRecent) {
                    ContentAction::RecentFiles
                } else {
                    ContentAction::FavoriteFiles
                });
            }
            Action::BrowserRetry => {
                use talk_core::ui::content::FileSection;
                if let Some(browser) = self.snapshot.content_panel.file_browser.as_ref() {
                    let retry = match browser.section {
                        FileSection::All => Some(ContentAction::ListFiles(browser.path.clone())),
                        FileSection::Recent => Some(ContentAction::RecentFiles),
                        FileSection::Favorites => Some(ContentAction::FavoriteFiles),
                        FileSection::Search => {
                            Text::new(self.content_ui.browser_search_value.trim())
                                .ok()
                                .map(ContentAction::SearchFiles)
                        }
                    };
                    if let Some(retry) = retry {
                        self.submit_content(retry);
                    }
                }
            }
            Action::BrowserSort(sort) => {
                self.content_ui.browser_sort_open = false;
                self.content_ui.browser_page = 0;
                self.content_ui.browser_cursor = None;
                self.submit_content(ContentAction::SortFiles(sort));
            }
            Action::BrowserStageSelection => {
                if let Some(context) = self.snapshot.selected
                    && let Some(browser) = self.snapshot.content_panel.file_browser.as_ref()
                    && self.cloud_can_stage(browser)
                {
                    let mut paths = Page::empty();
                    for path in &self.content_ui.browser_selection {
                        if paths.try_push(path.clone()).is_err() {
                            return;
                        }
                    }
                    if !paths.as_slice().is_empty()
                        && self
                            .submit_content(ContentAction::StageCloudFiles { context, paths })
                            .is_some()
                        && self.close_content_overlay()
                        && self.overlay.is_none()
                    {
                        self.composer.focus_handle(cx).focus(window);
                    }
                }
            }
            Action::Menu(thread) => self.toggle_composer_menu(thread, window, cx),
            Action::ComposerChoice(target, choice) => {
                self.choose_composer_menu(target, choice, window, cx);
            }
            Action::CreatePoll => {
                if !self.content_scope_current()
                    || !self.snapshot.content_panel.poll_create.enabled()
                {
                    return;
                }
                self.content_ui.return_overlay = self.overlay;
                self.content_ui.menu = false;
                self.handle(LocalAction::Open(Overlay::CreatePoll), window, cx);
                self.content_ui.question.focus_handle(cx).focus(window);
            }
            Action::AddOption => {
                if self.content_ui.option_count < MAX_POLL_OPTIONS {
                    if self.content_ui.options.len() == self.content_ui.option_count {
                        let theme = self.theme;
                        self.content_ui
                            .options
                            .push(cx.new(|cx| TextInput::new("Antwortoption", 1000, theme, cx)));
                    }
                    self.content_ui.option_count += 1;
                }
            }
            Action::RemoveOption(index) => {
                if self.content_ui.option_count > 2 && index < self.content_ui.option_count {
                    for i in index..self.content_ui.option_count - 1 {
                        self.content_ui.options.swap(i, i + 1);
                    }
                    self.content_ui.option_count -= 1;
                    self.content_ui.options[self.content_ui.option_count]
                        .update(cx, |i, _| i.reset());
                    if usize::from(self.content_ui.max_votes) > self.content_ui.option_count {
                        self.content_ui.max_votes = self.content_ui.option_count as u16;
                    }
                }
            }
            Action::MoveOption { index, up } => {
                let other = if up {
                    index.checked_sub(1)
                } else {
                    index.checked_add(1)
                };
                if let Some(other) = other.filter(|i| *i < self.content_ui.option_count)
                    && index < self.content_ui.option_count
                {
                    self.content_ui.options.swap(index, other);
                    self.content_ui.options[other]
                        .focus_handle(cx)
                        .focus(window);
                }
            }
            Action::Multiple => {
                self.content_ui.multiple = !self.content_ui.multiple;
                self.content_ui.max_votes = if self.content_ui.multiple { 0 } else { 1 };
            }
            Action::VoteLimit { increase } => {
                if self.content_ui.multiple {
                    let max = self.content_ui.option_count as u16;
                    self.content_ui.max_votes = if increase {
                        if self.content_ui.max_votes == 0 {
                            2.min(max)
                        } else {
                            (self.content_ui.max_votes + 1).min(max)
                        }
                    } else if self.content_ui.max_votes <= 2 {
                        0
                    } else {
                        self.content_ui.max_votes - 1
                    };
                }
            }
            Action::HiddenResults => {
                self.content_ui.hidden_results = !self.content_ui.hidden_results
            }
            Action::SubmitPoll => {
                if let Some(draft) = self.poll_draft(cx)
                    && self.content_scope_current()
                    && self.snapshot.content == ContentState::Ready
                    && !self.media_offline()
                    && self.snapshot.content_panel.poll_create.enabled()
                    && !self.poll_submission_blocked()
                    && let Some(context) = self.snapshot.selected
                    && let Some(request) = self.submit_content(ContentAction::CreatePoll(draft))
                {
                    self.content_ui.pending_poll = Some((context, request));
                }
            }
            Action::SaveDraft => {
                if let Some(draft) = self.poll_draft(cx)
                    && self.content_scope_current()
                    && self.snapshot.content == ContentState::Ready
                    && !self.media_offline()
                    && self.snapshot.content_panel.poll_drafts_manage.enabled()
                    && !self.poll_submission_blocked()
                    && let Some(context) = self.snapshot.selected
                {
                    let action = match self.content_ui.editing_draft {
                        Some(poll) if self.snapshot.content_panel.poll_drafts_edit.enabled() => {
                            ContentAction::UpdatePollDraft { poll, draft }
                        }
                        Some(_) => return,
                        None => ContentAction::SavePollDraft(draft),
                    };
                    if let Some(request) = self.submit_content(action) {
                        self.content_ui.pending_poll = Some((context, request));
                    }
                }
            }
            Action::ToggleDrafts => {
                self.content_ui.drafts_open = !self.content_ui.drafts_open;
                self.content_ui.draft_menu = None;
                self.content_ui.delete_draft = None;
                if self.content_ui.drafts_open
                    && self.snapshot.content_panel.poll_drafts_manage.enabled()
                    && self.snapshot.content == ContentState::Ready
                    && !self.media_offline()
                {
                    self.submit_content(ContentAction::RefreshPollDrafts);
                }
            }
            Action::ToggleDraftMenu(id) => {
                if self.content_scope_current() && self.content_ui.drafts_open {
                    self.content_ui.draft_menu =
                        (self.content_ui.draft_menu != Some(id)).then_some(id);
                    self.content_ui.delete_draft = None;
                }
            }
            Action::LoadDraft { id, edit } => {
                if let Some(entry) = self
                    .snapshot
                    .content_panel
                    .poll_drafts
                    .as_slice()
                    .iter()
                    .find(|entry| entry.id == id)
                    .cloned()
                    && (!edit || self.snapshot.content_panel.poll_drafts_edit.enabled())
                {
                    while self.content_ui.options.len() < entry.draft.options.as_slice().len() {
                        let theme = self.theme;
                        self.content_ui
                            .options
                            .push(cx.new(|cx| TextInput::new("Antwortoption", 1000, theme, cx)));
                    }
                    self.content_ui.question.update(cx, |input, _| {
                        input.set_context_value(entry.draft.question.as_str());
                    });
                    for (index, input) in self.content_ui.options.iter().enumerate() {
                        let value = entry
                            .draft
                            .options
                            .as_slice()
                            .get(index)
                            .map_or("", |value| value.as_str());
                        input.update(cx, |input, _| {
                            input.set_context_value(value);
                        });
                    }
                    self.content_ui.option_count = entry.draft.options.as_slice().len();
                    self.content_ui.max_votes = entry.draft.max_votes;
                    self.content_ui.multiple = entry.draft.max_votes != 1;
                    self.content_ui.hidden_results = entry.draft.hidden_results;
                    self.content_ui.editing_draft = edit.then_some(id);
                    self.content_ui.drafts_open = false;
                    self.content_ui.question.focus_handle(cx).focus(window);
                }
            }
            Action::ConfirmDeleteDraft(id) => {
                if self.content_ui.draft_menu == Some(id) {
                    self.content_ui.draft_menu = None;
                    self.content_ui.delete_draft = Some(id);
                }
            }
            Action::DeleteDraft(id) => {
                if self.content_ui.delete_draft == Some(id)
                    && self.snapshot.content_panel.poll_drafts_manage.enabled()
                    && self.snapshot.content == ContentState::Ready
                    && !self.media_offline()
                {
                    self.submit_content(ContentAction::DeletePollDraft(id));
                    self.content_ui.delete_draft = None;
                    self.content_ui.draft_menu = None;
                }
            }
            Action::SelectPollOption { poll, option } => {
                if let Some(current) = self
                    .snapshot
                    .content_panel
                    .polls
                    .as_slice()
                    .iter()
                    .find(|p| p.id == poll)
                    && self.content_scope_current()
                    && current.vote.enabled()
                    && !current.closed
                    && !self.poll_submission_blocked()
                {
                    let selected = self
                        .content_ui
                        .selections
                        .iter()
                        .find(|(id, _)| *id == poll)
                        .map(|(_, values)| values.as_slice().to_vec())
                        .unwrap_or_else(|| {
                            current
                                .options
                                .as_slice()
                                .iter()
                                .filter(|o| o.own)
                                .map(|o| o.id)
                                .collect()
                        });
                    if let Some(next) = toggled_poll_votes(current.max_votes, selected, option) {
                        self.content_ui.selections.retain(|(id, _)| *id != poll);
                        if self.content_ui.selections.len() >= 20 {
                            self.content_ui.selections.remove(0);
                        }
                        self.content_ui.selections.push((poll, next));
                        self.content_ui.vote_error = None;
                    }
                }
            }
            Action::SubmitVote(poll) => {
                if self.content_scope_current()
                    && self.snapshot.content == ContentState::Ready
                    && !self.media_offline()
                    && !self.poll_submission_blocked()
                    && let Some((_, options)) = self
                        .content_ui
                        .selections
                        .iter()
                        .find(|(id, _)| *id == poll)
                    && let Some(context) = self.snapshot.selected
                {
                    let options = options.clone();
                    if let Some(request) =
                        self.submit_content(ContentAction::Vote { poll, options })
                    {
                        self.content_ui.pending_vote = Some((context, request, poll));
                        self.content_ui.vote_error = None;
                    }
                }
            }
            Action::TogglePollMenu(poll) => {
                self.content_ui.poll_menu =
                    (self.content_ui.poll_menu != Some(poll)).then_some(poll);
            }
            Action::TogglePollVoters { poll, option } => {
                if self.content_scope_current() && self.content_ui.details_poll == Some(poll) {
                    let target = (poll, option);
                    self.content_ui.expanded_voters =
                        (self.content_ui.expanded_voters != Some(target)).then_some(target);
                }
            }
            Action::ConfirmClosePoll(poll) => {
                self.content_ui.poll_menu = None;
                self.content_ui.confirm_close = Some(poll);
            }
            Action::CancelClosePoll => self.content_ui.confirm_close = None,
            Action::ClosePoll(poll) => {
                if self.content_ui.confirm_close == Some(poll)
                    && !self.poll_submission_blocked()
                    && self.snapshot.content == ContentState::Ready
                    && !self.media_offline()
                    && let Some(context) = self.snapshot.selected
                    && let Some(request) = self.submit_content(ContentAction::ClosePoll(poll))
                {
                    self.content_ui.pending_close = Some((context, request, poll));
                    self.content_ui.close_error = None;
                    self.content_ui.confirm_close = None;
                }
            }
            Action::PollDetails(poll) => {
                if self.content_scope_current()
                    && self
                        .snapshot
                        .content_panel
                        .polls
                        .as_slice()
                        .iter()
                        .any(|p| p.id == poll)
                {
                    self.content_ui.details_poll = Some(poll);
                    self.content_ui.return_overlay = self.overlay;
                    self.handle(LocalAction::Open(Overlay::PollDetails), window, cx);
                }
            }
            Action::VoiceStart(context) => {
                self.content_ui.voice_error = None;
                self.content_ui.voice_auto_send = None;
                if self.composer_available()
                    && !self.drafts.target(context).new_thread
                    && self.drafts.target(context).private_reply.is_none()
                    && Some(context) == self.snapshot.selected
                    && self.composer.read(cx).content.is_empty()
                    && !self.has_staged_attachments()
                    && self.message_feature_enabled(talk_core::ui::messaging::MessageFeature::Voice)
                {
                    self.stop_local_typing();
                    match self.settings_port.start_voice(context) {
                        Ok(()) => {
                            self.content_ui.voice_levels.clear();
                            self.content_ui.voice_level_bucket_ms = INITIAL_VOICE_BUCKET_MS;
                            self.content_ui.voice_epoch =
                                self.content_ui.voice_epoch.saturating_add(1);
                            self.content_ui.voice = self.settings_port.voice_snapshot();
                        }
                        Err(error) => self.content_ui.voice_error = Some(voice_error(error)),
                    }
                }
            }
            Action::VoiceStop(context) => {
                if let Err(error) = self.settings_port.stop_voice(context, false) {
                    self.content_ui.voice_error = Some(voice_error(error));
                }
            }
            Action::VoiceFinishAndSend(context) => {
                if Some(context) == self.snapshot.selected
                    && self.content_ui.voice_auto_send.is_none()
                    && self.snapshot.content == ContentState::Ready
                    && !self.media_offline()
                    && self.message_feature_enabled(talk_core::ui::messaging::MessageFeature::Voice)
                {
                    match self.settings_port.stop_voice(context, false) {
                        Ok(()) => {
                            self.content_ui.voice_auto_send = Some(context);
                            self.content_ui.voice_error = None;
                        }
                        Err(error) => self.content_ui.voice_error = Some(voice_error(error)),
                    }
                }
            }
            Action::VoiceCancel(context) => {
                self.content_ui.voice_auto_send = None;
                let _ = self.settings_port.stop_voice(context, true);
                self.content_ui.voice_error = None;
            }
            Action::VoiceSend(context) => {
                if let Err(error) = self.settings_port.send_voice(context) {
                    self.content_ui.voice_error = Some(voice_error(error));
                } else {
                    self.content_ui.voice_error = None;
                }
            }
            Action::VoiceReviewPlay { context, play } => {
                if let Err(error) = self.settings_port.voice_review_play(context, play) {
                    self.content_ui.voice_error = Some(voice_error(error));
                } else {
                    self.content_ui.voice_error = None;
                }
            }
            Action::VoiceReviewSpeed { context, speed } => {
                if let Err(error) = self.settings_port.voice_review_speed(context, speed) {
                    self.content_ui.voice_error = Some(voice_error(error));
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
                if matches!(command, ContentAction::PlayVoice(_)) {
                    self.settings_port.stop_voice_review_playback();
                }
                let selected_file = matches!(&command, ContentAction::StageCloudFile { .. });
                if self.submit_content(command).is_some()
                    && selected_file
                    && self.close_content_overlay()
                    && self.overlay.is_none()
                {
                    self.composer.focus_handle(cx).focus(window);
                }
            }
        }
        if poll_layout_changed {
            let anchor = self.message_ui.list.logical_scroll_top();
            let count = self.message_ui.list.item_count();
            self.message_ui.list.splice(0..count, count);
            self.message_ui.list.scroll_to(anchor);
        }
        cx.notify();
    }

    pub(super) fn close_content_overlay(&mut self) -> bool {
        if !matches!(
            self.overlay,
            Some(
                Overlay::ContentPreview
                    | Overlay::CreatePoll
                    | Overlay::CloudFiles
                    | Overlay::PollDetails
            )
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
        if self.overlay == Some(Overlay::CloudFiles) {
            self.content_ui.browser_search_since = None;
            self.content_ui.browser_history.clear();
            self.submit_content(ContentAction::CloseFileBrowser);
        }
        self.content_ui.details_poll = None;
        self.content_ui.expanded_voters = None;
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
    /// Application-wide status: the transfer owner outlives a mounted file row.
    pub(super) fn download_status(&self, cx: &Context<Self>) -> Option<Div> {
        let transfer = self.snapshot.content_panel.transfer.as_ref()?;
        let (symbol, label) = match transfer.state {
            DownloadState::Transferring => ("download", "Datei wird heruntergeladen …"),
            DownloadState::Committing => ("download", "Datei wird gespeichert …"),
            DownloadState::Saved => ("check", "Datei gespeichert."),
            DownloadState::Cancelled => ("x", "Download abgebrochen."),
            DownloadState::Failed(_) => {
                ("alert-circle", "Download fehlgeschlagen. Erneut versuchen.")
            }
        };
        let active = matches!(
            transfer.state,
            DownloadState::Transferring | DownloadState::Committing
        );
        let fraction = transfer
            .total
            .filter(|total| *total > 0)
            .map(|total| (transfer.received.min(total) as f64 / total as f64) as f32);
        Some(
            div()
                .relative()
                .flex_none()
                .w(px(280.))
                .max_w(gpui::relative(0.45))
                .min_w(px(100.))
                .h(px(32.))
                .rounded(px(6.))
                .bg(self.theme.subtle)
                .overflow_hidden()
                .flex()
                .items_center()
                .gap_2()
                .pl_2()
                .pr_1()
                .child(self.scaled_icon(symbol, 16., self.theme.muted))
                .child(
                    div()
                        .flex_1()
                        .min_w_0()
                        .flex()
                        .flex_col()
                        .child(
                            div()
                                .truncate()
                                .text_size(self.sp(12.))
                                .child(transfer.name.as_str().to_owned()),
                        )
                        .child(
                            div()
                                .truncate()
                                .text_size(self.sp(12.))
                                .text_color(self.theme.muted)
                                .child(if active {
                                    match transfer.total {
                                        Some(total) => format!(
                                            "{} / {}",
                                            super::localization::file_size(
                                                self.locale,
                                                Some(transfer.received)
                                            ),
                                            super::localization::file_size(
                                                self.locale,
                                                Some(total)
                                            )
                                        ),
                                        None => super::localization::file_size(
                                            self.locale,
                                            Some(transfer.received),
                                        ),
                                    }
                                } else {
                                    self.l(label).to_owned()
                                }),
                        ),
                )
                .child(
                    self.icon_button(
                        "download-status-action",
                        (
                            "x",
                            if active {
                                "Download abbrechen"
                            } else {
                                "Schließen"
                            },
                        ),
                        ButtonStyle::Quiet,
                        Availability::available(),
                        LocalAction::Content(Action::Command(if active {
                            ContentAction::CancelDownload
                        } else {
                            ContentAction::DismissDownload
                        })),
                        cx,
                    )
                    .size(px(26.))
                    .min_h(px(26.)),
                )
                .when_some(fraction.filter(|_| active), |d, fraction| {
                    d.child(
                        div()
                            .absolute()
                            .left_0()
                            .bottom_0()
                            .h(px(2.))
                            .w(gpui::relative(fraction))
                            .bg(self.theme.accent),
                    )
                }),
        )
    }

    fn poll_draft(&self, cx: &Context<Self>) -> Option<PollDraft> {
        let question = self.content_ui.question.read(cx).submitted_value()?;
        let options = self
            .content_ui
            .options
            .iter()
            .take(self.content_ui.option_count)
            .map(|input| input.read(cx).submitted_value())
            .collect::<Option<Vec<_>>>()?;
        poll_draft_values(
            question,
            options,
            self.content_ui.max_votes,
            self.content_ui.hidden_results,
        )
    }
    pub(super) fn poll_drafts_open(&self) -> bool {
        self.content_ui.drafts_open
    }
    pub(super) fn poll_dialog_preferred_height(&self) -> f32 {
        let additional_options = self.content_ui.option_count.saturating_sub(2) as f32;
        let vote_limit = if self.content_ui.multiple { 48. } else { 0. };
        (520. + additional_options * 52. + vote_limit).min(632.)
    }
    pub(super) fn poll_drafts_header(&self, cx: &Context<Self>) -> Option<gpui::Stateful<Div>> {
        (self.content_scope_current() && self.snapshot.content_panel.poll_drafts_manage.enabled())
            .then(|| {
                let label = self.lf(
                    "Entwürfe · {n}",
                    &[(
                        "n",
                        &self
                            .snapshot
                            .content_panel
                            .poll_drafts
                            .as_slice()
                            .len()
                            .to_string(),
                    )],
                );
                self.button(
                    "poll-drafts-toggle",
                    label,
                    ButtonStyle::Quiet,
                    Availability::when(
                        !self.poll_submission_blocked(),
                        "Eine Anfrage wird bereits bearbeitet.",
                    ),
                    LocalAction::Content(Action::ToggleDrafts),
                    cx,
                )
            })
    }

    pub(super) fn poll_creation_body(&self, cx: &Context<Self>) -> Div {
        let busy = self.poll_submission_blocked();
        let online = self.snapshot.content == ContentState::Ready && !self.media_offline();
        let available = self.content_scope_current()
            && online
            && self.snapshot.content_panel.poll_create.enabled();
        let mut body = div().flex().flex_col().gap_3();
        if self.content_scope_current()
            && self.snapshot.content_panel.poll_drafts_manage.enabled()
            && self.content_ui.drafts_open
        {
            if self
                .snapshot
                .content_panel
                .poll_drafts
                .as_slice()
                .is_empty()
            {
                let label = match self.snapshot.content_panel.poll_state {
                    RemoteState::Loading(_) | RemoteState::Saving(_) => {
                        "Umfrageentwürfe werden geladen …"
                    }
                    RemoteState::Failed(_, _) | RemoteState::Uncertain(_) => {
                        "Umfrageentwürfe konnten nicht geladen werden."
                    }
                    RemoteState::Idle => "Keine Umfrageentwürfe vorhanden.",
                };
                body = body.child(if self.snapshot.content_panel.poll_state.pending() {
                    self.loading_row(self.l(label))
                } else {
                    self.empty_row(self.l(label))
                });
                if matches!(
                    self.snapshot.content_panel.poll_state,
                    RemoteState::Failed(_, _)
                ) {
                    body = body.child(self.button(
                        "poll-drafts-retry",
                        "Erneut laden",
                        ButtonStyle::Quiet,
                        online,
                        LocalAction::Content(Action::Command(ContentAction::RefreshPollDrafts)),
                        cx,
                    ));
                }
            }
            for entry in self.snapshot.content_panel.poll_drafts.as_slice() {
                let id = entry.id;
                let group: gpui::SharedString = format!("poll-draft-{id}").into();
                let mut actions = div()
                    .w_full()
                    .flex()
                    .flex_wrap()
                    .items_center()
                    .gap_1()
                    .child(self.button(
                        ("poll-draft-use", id as usize),
                        "Für neue Umfrage verwenden",
                        ButtonStyle::Quiet,
                        Availability::when(!busy, "Eine Anfrage wird bereits bearbeitet."),
                        LocalAction::Content(Action::LoadDraft { id, edit: false }),
                        cx,
                    ));
                if self.snapshot.content_panel.poll_drafts_edit.enabled() {
                    actions = actions.child(
                        self.button(
                            ("poll-draft-edit", id as usize),
                            "Entwurf bearbeiten",
                            ButtonStyle::Quiet,
                            Availability::when(!busy, "Eine Anfrage wird bereits bearbeitet."),
                            LocalAction::Content(Action::LoadDraft { id, edit: true }),
                            cx,
                        )
                        .opacity(0.)
                        .group_hover(group.clone(), |style| style.opacity(1.))
                        .focus(|style| style.opacity(1.)),
                    );
                }
                actions = if self.content_ui.delete_draft == Some(id) {
                    actions
                        .child(self.hint(self.l("Diesen Entwurf löschen?")))
                        .child(self.button(
                            ("poll-draft-delete-confirm", id as usize),
                            "Entwurf endgültig löschen",
                            ButtonStyle::Danger,
                            Availability::when(
                                !busy && online,
                                if busy {
                                    "Eine Anfrage wird bereits bearbeitet."
                                } else {
                                    "Die Verbindung ist derzeit nicht bereit."
                                },
                            ),
                            LocalAction::Content(Action::DeleteDraft(id)),
                            cx,
                        ))
                } else {
                    actions
                        .child(
                            self.icon_button(
                                ("poll-draft-menu", id as usize),
                                ("more", "Weitere Optionen"),
                                ButtonStyle::Quiet,
                                Availability::when(!busy, "Eine Anfrage wird bereits bearbeitet."),
                                LocalAction::Content(Action::ToggleDraftMenu(id)),
                                cx,
                            )
                            .ml_auto(),
                        )
                        .when(self.content_ui.draft_menu == Some(id), |actions| {
                            actions.child(self.button(
                                ("poll-draft-delete", id as usize),
                                "Entwurf löschen",
                                ButtonStyle::Quiet,
                                !busy,
                                LocalAction::Content(Action::ConfirmDeleteDraft(id)),
                                cx,
                            ))
                        })
                };
                body = body.child(
                    div()
                        .group(group)
                        .flex()
                        .flex_col()
                        .gap_1()
                        .p_3()
                        .bg(self.theme.subtle)
                        .rounded(px(8.))
                        .child(entry.draft.question.as_str().to_owned())
                        .child(actions),
                );
            }
            return body;
        }
        let options: Vec<_> = self
            .content_ui
            .options
            .iter()
            .take(self.content_ui.option_count)
            .map(|input| input.read(cx).content.clone())
            .collect();
        let options_interacted = options.iter().any(|value| !value.trim().is_empty());
        body = div()
            .flex()
            .flex_col()
            .gap_3()
            .child(self.hint(self.l("Frage")))
            .child(
                self.input_surface(&self.content_ui.question, cx)
                    .child(self.content_ui.question.clone()),
            );
        if options_interacted && self.content_ui.question.read(cx).content.trim().is_empty() {
            body = body.child(self.hint(self.l("Gib eine Frage ein.")));
        }
        if self.content_ui.editing_draft.is_some() {
            body = body.child(self.hint(self.l("Entwurf bearbeiten")));
        }
        for (index, input) in self
            .content_ui
            .options
            .iter()
            .take(self.content_ui.option_count)
            .enumerate()
        {
            let group: gpui::SharedString = format!("poll-option-row-{index}").into();
            body = body.child(
                div()
                    .group(group.clone())
                    .relative()
                    .flex()
                    .items_center()
                    .child(
                        self.input_surface(input, cx)
                            .flex_1()
                            .min_w_0()
                            .pr(px(100.))
                            .child(input.clone()),
                    )
                    .child(
                        div()
                            .absolute()
                            .right(px(2.))
                            .top(px(2.))
                            .flex()
                            .items_center()
                            .child(
                                self.icon_button(
                                    ("poll-option-up", index),
                                    ("chevron-up", "Nach oben"),
                                    ButtonStyle::Quiet,
                                    Availability::when(
                                        !busy && index > 0,
                                        if busy {
                                            "Eine Anfrage wird bereits bearbeitet."
                                        } else {
                                            "Die Option ist bereits an erster Stelle."
                                        },
                                    ),
                                    LocalAction::Content(Action::MoveOption { index, up: true }),
                                    cx,
                                )
                                .opacity(0.)
                                .group_hover(group.clone(), |style| style.opacity(1.))
                                .focus(|style| style.opacity(1.)),
                            )
                            .child(
                                self.icon_button(
                                    ("poll-option-down", index),
                                    ("chevron-down", "Nach unten"),
                                    ButtonStyle::Quiet,
                                    Availability::when(
                                        !busy && index + 1 < self.content_ui.option_count,
                                        if busy {
                                            "Eine Anfrage wird bereits bearbeitet."
                                        } else {
                                            "Die Option ist bereits an letzter Stelle."
                                        },
                                    ),
                                    LocalAction::Content(Action::MoveOption { index, up: false }),
                                    cx,
                                )
                                .opacity(0.)
                                .group_hover(group.clone(), |style| style.opacity(1.))
                                .focus(|style| style.opacity(1.)),
                            )
                            .child(
                                self.icon_button(
                                    ("poll-option-remove", index),
                                    ("trash-2", "Entfernen"),
                                    ButtonStyle::Quiet,
                                    Availability::when(
                                        !busy && self.content_ui.option_count > 2,
                                        if busy {
                                            "Eine Anfrage wird bereits bearbeitet."
                                        } else {
                                            "Mindestens zwei Antwortoptionen sind erforderlich."
                                        },
                                    ),
                                    LocalAction::Content(Action::RemoveOption(index)),
                                    cx,
                                )
                                .opacity(0.)
                                .group_hover(group.clone(), |style| style.opacity(1.))
                                .focus(|style| style.opacity(1.)),
                            ),
                    ),
            );
        }
        if options_interacted
            && (options.iter().any(|value| value.trim().is_empty())
                || options.iter().enumerate().any(|(index, value)| {
                    options[..index]
                        .iter()
                        .any(|previous| previous.trim() == value.trim())
                }))
        {
            body = body.child(
                self.hint(self.l("Mindestens zwei unterschiedliche Antwortoptionen eingeben.")),
            );
        }
        body = body
            .child(
                div()
                    .flex()
                    .flex_wrap()
                    .gap_2()
                    .child(self.button_with_leading_icon(
                        "poll-add-option",
                        ("Option hinzufügen", "plus"),
                        ButtonStyle::Quiet,
                        Availability::when(
                            !busy && self.content_ui.option_count < MAX_POLL_OPTIONS,
                            if busy {
                                "Eine Anfrage wird bereits bearbeitet."
                            } else {
                                "Die maximale Anzahl an Optionen ist erreicht."
                            },
                        ),
                        LocalAction::Content(Action::AddOption),
                        cx,
                    )),
            )
            .child(
                self.dialog_row(
                    "Mehrere Antworten erlauben",
                    None,
                    self.switch(
                        "poll-multiple",
                        "Mehrere Antworten erlauben",
                        Some(self.content_ui.multiple),
                        Availability::when(!busy, "Eine Anfrage wird bereits bearbeitet.")
                            .help("Teilnehmende können mehr als eine Antwort wählen."),
                        LocalAction::Content(Action::Multiple),
                        cx,
                    ),
                ),
            )
            .when(self.content_ui.multiple, |d| {
                d.child(
                    div()
                        .flex()
                        .items_center()
                        .gap_2()
                        .child(self.button(
                            "poll-vote-limit-down",
                            "Weniger Stimmen",
                            ButtonStyle::Quiet,
                            Availability::when(!busy, "Eine Anfrage wird bereits bearbeitet."),
                            LocalAction::Content(Action::VoteLimit { increase: false }),
                            cx,
                        ))
                        .child(if self.content_ui.max_votes == 0 {
                            self.hint(self.l("Unbegrenzte Stimmen"))
                        } else {
                            self.hint(&self.lf(
                                "Höchstens {n} Stimmen",
                                &[("n", &self.content_ui.max_votes.to_string())],
                            ))
                        })
                        .child(self.button(
                            "poll-vote-limit-up",
                            "Mehr Stimmen",
                            ButtonStyle::Quiet,
                            Availability::when(
                                !busy
                                    && usize::from(self.content_ui.max_votes)
                                        < self.content_ui.option_count,
                                if busy {
                                    "Eine Anfrage wird bereits bearbeitet."
                                } else {
                                    "Die Stimmenzahl entspricht bereits der Anzahl der Optionen."
                                },
                            ),
                            LocalAction::Content(Action::VoteLimit { increase: true }),
                            cx,
                        )),
                )
            })
            .child(
                self.dialog_row(
                    "Anonym abstimmen",
                    None,
                    self.switch(
                        "poll-hidden",
                        "Anonym abstimmen",
                        Some(self.content_ui.hidden_results),
                        Availability::when(!busy, "Eine Anfrage wird bereits bearbeitet.")
                            .help("Ergebnisse erscheinen nach dem Ende; Stimmen bleiben anonym."),
                        LocalAction::Content(Action::HiddenResults),
                        cx,
                    ),
                ),
            );
        if !available {
            body = body.child(self.hint(self.l(if !online {
                "Die Verbindung ist derzeit nicht bereit."
            } else {
                super::conversation_dialog::unavailable(self.snapshot.content_panel.poll_create)
            })));
        }
        let status = match self.snapshot.content_panel.poll_state {
            RemoteState::Failed(request, _) if request != RequestId(0) => {
                Some("Die Umfrage konnte nicht gespeichert werden. Dein Entwurf bleibt erhalten.")
            }
            RemoteState::Uncertain(_) => {
                Some("Ergebnis unklar. Prüfe das Gespräch vor erneutem Veröffentlichen.")
            }
            RemoteState::Idle
            | RemoteState::Loading(_)
            | RemoteState::Saving(_)
            | RemoteState::Failed(_, _) => None,
        };
        body.when_some(status, |d, s| d.child(self.hint(self.l(s))))
    }

    pub(super) fn poll_creation_footer(&self, cx: &Context<Self>) -> Div {
        let busy = self.poll_submission_blocked();
        let online = self.snapshot.content == ContentState::Ready && !self.media_offline();
        let valid = self.poll_draft(cx).is_some();
        let available = self.content_scope_current()
            && online
            && self.snapshot.content_panel.poll_create.enabled();
        self.dialog_footer()
            .justify_between()
            .child(div().when(
                self.snapshot.content_panel.poll_drafts_manage.enabled(),
                |d| {
                    d.child(self.button(
                        "poll-save-draft",
                        "Als Entwurf speichern",
                        ButtonStyle::Quiet,
                        Availability::when(!busy
                            && online
                            && valid
                            && (self.content_ui.editing_draft.is_none()
                                || self.snapshot.content_panel.poll_drafts_edit.enabled()),
                            if busy { "Eine Anfrage wird bereits bearbeitet." } else if !online { "Die Verbindung ist derzeit nicht bereit." } else if !valid { "Gib eine Frage und mindestens zwei unterschiedliche Optionen ein." } else { "Die Berechtigung ist derzeit nicht verfügbar." }),
                        LocalAction::Content(Action::SaveDraft),
                        cx,
                    ))
                },
            ))
            .child(div().flex().items_center().gap_2()
                .child(self.button("poll-cancel", "Abbrechen", ButtonStyle::Quiet, true, LocalAction::Close, cx))
                .child(self.button(
                    "poll-submit",
                    "Veröffentlichen",
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
                        Availability::when(available && !busy && valid, if busy { "Eine Anfrage wird bereits bearbeitet." } else if !valid { "Gib eine Frage und mindestens zwei unterschiedliche Optionen ein." } else if !online { "Die Verbindung ist derzeit nicht bereit." } else { super::conversation_dialog::unavailable(self.snapshot.content_panel.poll_create) })
                    },
                    LocalAction::Content(Action::SubmitPoll),
                    cx,
                )))
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
            if self.snapshot.content_panel.poll_create.server
                == talk_core::ui::presentation::FeatureSupport::Unsupported
            {
                return div().child(
                    self.hint(self.l("Umfragen werden von diesem Server nicht unterstützt.")),
                );
            }
            return self
                .style_card(div(), super::controls::CardStyle::Inset)
                .flex()
                .flex_col()
                .gap_2()
                .p_3()
                .child(self.hint(self.l(if self.media_offline() {
                    "Die Verbindung ist derzeit nicht bereit."
                } else {
                    "Umfrage wird geladen …"
                })))
                .when(!self.snapshot.content_panel.poll_state.pending(), |body| {
                    body.child(self.button(
                        ("load-poll", id as usize),
                        "Umfrage öffnen",
                        ButtonStyle::Secondary,
                        self.content_scope_current()
                            && self.snapshot.content == ContentState::Ready
                            && !self.media_offline(),
                        LocalAction::Content(Action::Command(ContentAction::RefreshPoll(id))),
                        cx,
                    ))
                });
        };
        self.loaded_poll(poll, cx)
    }
    fn loaded_poll(&self, poll: &PollView, cx: &Context<Self>) -> Div {
        self.poll_card_contents(poll, false, cx)
    }

    pub(super) fn poll_details_body(&self, cx: &Context<Self>) -> Div {
        self.content_ui
            .details_poll
            .and_then(|id| {
                self.snapshot
                    .content_panel
                    .polls
                    .as_slice()
                    .iter()
                    .find(|poll| poll.id == id)
            })
            .filter(|_| self.content_scope_current())
            .map_or_else(div, |poll| self.poll_card_contents(poll, true, cx))
    }

    fn poll_card_contents(&self, poll: &PollView, full: bool, cx: &Context<Self>) -> Div {
        let selection = self
            .content_ui
            .selections
            .iter()
            .find(|(id, _)| *id == poll.id)
            .map(|(_, selection)| selection.as_slice().to_vec())
            .unwrap_or_else(|| {
                poll.options
                    .as_slice()
                    .iter()
                    .filter(|o| o.own)
                    .map(|o| o.id)
                    .collect()
            });
        let pending = self
            .content_ui
            .pending_vote
            .is_some_and(|(_, _, id)| id == poll.id);
        let meta = if poll.max_votes == 1 {
            "Eine Antwort"
        } else {
            "Mehrere Antworten"
        };
        let privacy = if poll.hidden_results {
            "Anonym"
        } else {
            "Öffentlich"
        };
        let voter_label = poll
            .voters
            .map(|count| self.lf("{n} Teilnehmende abgestimmt", &[("n", &count.to_string())]));
        let meta_label = match voter_label {
            Some(voters) => format!("{} · {} · {}", self.l(meta), self.l(privacy), voters),
            None => format!("{} · {}", self.l(meta), self.l(privacy)),
        };
        let mut body = div()
            .flex()
            .flex_col()
            .items_start()
            .w_full()
            .max_w(px(if full { 620. } else { 480. }))
            .gap_2()
            .when(!full, |d| {
                self.style_card(d.p_4(), super::controls::CardStyle::Inset)
            })
            .child(
                div()
                    .w_full()
                    .flex()
                    .items_start()
                    .justify_between()
                    .gap_2()
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .text_size(self.sp(15.))
                            .line_height(self.sp(22.))
                            .font_weight(gpui::FontWeight::SEMIBOLD)
                            .child(poll.question.as_str().to_owned()),
                    )
                    .when(!full && !poll.closed && poll.close.enabled(), |d| {
                        d.child(
                            self.icon_button(
                                ("close-poll", poll.id as usize),
                                ("more", "Umfrage beenden"),
                                ButtonStyle::Quiet,
                                Availability::when(
                                    !self.poll_submission_blocked() && !self.media_offline(),
                                    "Die Verbindung ist derzeit nicht bereit.",
                                ),
                                LocalAction::Content(Action::TogglePollMenu(poll.id)),
                                cx,
                            )
                            .size(px(32.))
                            .min_h(px(32.)),
                        )
                    }),
            )
            .child(
                div()
                    .flex()
                    .flex_wrap()
                    .items_center()
                    .gap_2()
                    .child(self.hint(&meta_label))
                    .when(!full, |d| {
                        d.child(
                            self.button(
                                ("poll-details", poll.id as usize),
                                "Details und Ergebnisse",
                                ButtonStyle::Quiet,
                                true,
                                LocalAction::Content(Action::PollDetails(poll.id)),
                                cx,
                            )
                            .min_h(px(28.))
                            .h(px(28.))
                            .px_1()
                            .text_size(self.sp(12.)),
                        )
                    }),
            );
        if self.media_offline() {
            body = body.child(self.hint(self.l("Gespeicherter Stand")));
        }
        let count = if full {
            poll.options.as_slice().len()
        } else {
            5
        };
        for option in poll.options.as_slice().iter().take(count) {
            let selected = selection.contains(&option.id);
            let enabled = poll.vote.enabled()
                && !poll.closed
                && !self.poll_submission_blocked()
                && toggled_poll_votes(poll.max_votes, selection.iter().copied(), option.id)
                    .is_some();
            let mut row = div().flex().flex_col().w_full().gap_1().child(
                self.button(
                    (
                        "poll-option",
                        ((poll.id as usize) << 8) ^ usize::from(option.id),
                    ),
                    "",
                    ButtonStyle::Quiet,
                    enabled,
                    LocalAction::Content(Action::SelectPollOption {
                        poll: poll.id,
                        option: option.id,
                    }),
                    cx,
                )
                .w_full()
                .justify_start()
                .child(self.choice_marker(selected, poll.max_votes == 1))
                .child(
                    div()
                        .flex_1()
                        .text_color(self.theme.text)
                        .child(option.label.as_str().to_owned()),
                )
                .when(option.own && selected, |d| {
                    d.child(self.hint(self.l("Deine Stimme")))
                })
                .when(pending && selected, |d| {
                    d.child(self.hint(self.l("Wird gesendet …")))
                }),
            );
            if let Some(votes) = option.votes {
                let percent = poll.voters.filter(|n| *n > 0).map_or(0, |n| {
                    ((u64::from(votes) * 100) / u64::from(n)).min(100) as u32
                });
                let bar = div()
                    .w_full()
                    .h(px(6.))
                    .rounded(px(3.))
                    .bg(if self.theme.dark {
                        gpui::rgb(0x3a4558).into()
                    } else {
                        self.theme.muted.opacity(0.35)
                    })
                    .child(
                        div()
                            .h_full()
                            .rounded(px(3.))
                            .bg(if self.theme.dark {
                                gpui::rgb(0x6fa0e8).into()
                            } else {
                                self.theme.accent
                            })
                            .w(gpui::relative(percent as f32 / 100.)),
                    );
                row = row
                    .child(self.poll_result_reveal(poll.id, option.id, bar))
                    .child(self.hint(&format!("{percent}% · {votes}")));
            }
            if full && !poll.hidden_results {
                let voters: Vec<_> = poll
                    .details
                    .as_slice()
                    .iter()
                    .filter(|v| v.option == option.id)
                    .take(100)
                    .collect();
                if !voters.is_empty() {
                    let expanded = self.content_ui.expanded_voters == Some((poll.id, option.id));
                    row = row.child(self.button(
                        (
                            "poll-voters",
                            ((poll.id as usize) << 8) ^ usize::from(option.id),
                        ),
                        if expanded {
                            "Stimmen ausblenden"
                        } else {
                            "Stimmen anzeigen"
                        },
                        ButtonStyle::Quiet,
                        true,
                        LocalAction::Content(Action::TogglePollVoters {
                            poll: poll.id,
                            option: option.id,
                        }),
                        cx,
                    ));
                    if expanded {
                        row = row.children(voters.into_iter().map(|voter| {
                            let initials: String = voter
                                .name
                                .as_str()
                                .split_whitespace()
                                .take(2)
                                .filter_map(|part| part.chars().next())
                                .flat_map(char::to_uppercase)
                                .collect();
                            div()
                                .flex()
                                .items_center()
                                .gap_2()
                                .child(self.person_avatar(&initials, 20.))
                                .child(self.hint(voter.name.as_str()))
                        }));
                    }
                }
            }
            body = body.child(row);
        }
        let changed_vote = self
            .content_ui
            .selections
            .iter()
            .find(|(id, _)| *id == poll.id);
        if !poll.closed && poll.vote.enabled() && changed_vote.is_some() {
            body = body.child(
                self.button(
                    ("poll-vote-submit", poll.id as usize),
                    if self.content_ui.vote_error == Some(poll.id) {
                        "Erneut abstimmen"
                    } else if changed_vote.is_some_and(|(_, choices)| choices.as_slice().is_empty())
                    {
                        "Stimme zurückziehen"
                    } else {
                        "Abstimmen"
                    },
                    ButtonStyle::Primary,
                    !self.poll_submission_blocked()
                        && self.snapshot.content == ContentState::Ready
                        && !self.media_offline()
                        && self
                            .content_ui
                            .selections
                            .iter()
                            .any(|(id, _)| *id == poll.id),
                    LocalAction::Content(Action::SubmitVote(poll.id)),
                    cx,
                ),
            );
        }
        if self.content_ui.vote_error == Some(poll.id) {
            body = body
                .child(self.hint(self.l("Deine Auswahl bleibt erhalten. Bitte erneut versuchen.")));
        }
        if !poll.closed && self.media_offline() {
            body = body.child(self.hint(self.l("Verbinde dich erneut, um abzustimmen.")));
        }
        if let Some((id, uncertain)) = self.content_ui.close_error
            && id == poll.id
        {
            body = body.child(self.hint(self.l(if uncertain {
                "Der Status ist unklar. Die Umfrage wird erneut geprüft."
            } else {
                "Die Umfrage konnte nicht beendet werden. Bitte erneut versuchen."
            })));
        }
        if matches!(
            self.snapshot.content_panel.poll_state,
            RemoteState::Uncertain(_)
        ) && (self.content_ui.vote_error == Some(poll.id)
            || self
                .content_ui
                .close_error
                .is_some_and(|(id, uncertain)| id == poll.id && uncertain))
        {
            body = body.child(self.button(
                ("poll-recheck", poll.id as usize),
                "Umfrage erneut prüfen",
                ButtonStyle::Quiet,
                !self.media_offline(),
                LocalAction::Content(Action::Command(ContentAction::RefreshPoll(poll.id))),
                cx,
            ));
        }
        if poll.closed {
            body = body.child(self.hint(self.l("Umfrage beendet.")));
        } else if poll.hidden_results {
            body = body.child(self.hint(self.l("Ergebnisse werden nach Abschluss angezeigt.")));
        }
        let mut secondary = div().w_full().flex().flex_col().items_start().gap_1();
        if !poll.closed && poll.close.enabled() {
            if self.content_ui.confirm_close == Some(poll.id) {
                secondary = secondary
                    .child(self.hint(self.l("Umfrage wirklich beenden?")))
                    .child(self.hint(self.l(
                        "Danach kann nicht mehr abgestimmt werden; die Ergebnisse sind sichtbar.",
                    )))
                    .child(self.button(
                        ("poll-close-confirm", poll.id as usize),
                        "Jetzt beenden",
                        ButtonStyle::Secondary,
                        !self.poll_submission_blocked() && !self.media_offline(),
                        LocalAction::Content(Action::ClosePoll(poll.id)),
                        cx,
                    ))
                    .child(self.button(
                        ("poll-close-cancel", poll.id as usize),
                        "Abbrechen",
                        ButtonStyle::Quiet,
                        true,
                        LocalAction::Content(Action::CancelClosePoll),
                        cx,
                    ));
            } else {
                if self.content_ui.poll_menu == Some(poll.id) {
                    secondary = secondary.child(self.button(
                        ("poll-close-menu", poll.id as usize),
                        "Umfrage beenden",
                        ButtonStyle::Secondary,
                        Availability::when(
                            !self.poll_submission_blocked() && !self.media_offline(),
                            "Die Verbindung ist derzeit nicht bereit.",
                        ),
                        LocalAction::Content(Action::ConfirmClosePoll(poll.id)),
                        cx,
                    ));
                }
            }
        }
        body.child(secondary)
    }
}

fn voice_session_retired(
    same_scope: bool,
    content: ContentState,
    online: bool,
    feature_enabled: bool,
) -> bool {
    !same_scope
        || matches!(content, ContentState::Denied | ContentState::Unconfigured)
        || (content == ContentState::Ready && online && !feature_enabled)
}

fn voice_error(error: talk_core::preferences::devices::DeviceError) -> &'static str {
    use talk_core::preferences::devices::DeviceError;
    match error {
        DeviceError::PermissionDenied => {
            "Mikrofonzugriff verweigert. Erlaube den Zugriff in den Systemeinstellungen."
        }
        DeviceError::Removed | DeviceError::InvalidSelection | DeviceError::Unavailable => {
            "Mikrofon nicht verfügbar. Prüfe die Geräteauswahl."
        }
        _ => "Sprachnachricht konnte nicht verarbeitet werden. Bitte erneut versuchen.",
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

pub(in crate::presentation) fn placeholder_key(value: &str) -> u128 {
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
    max_votes: u16,
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
    if options.as_slice().len() < 2 || usize::from(max_votes) > options.as_slice().len() {
        return None;
    }
    Some(PollDraft {
        question: Text::new(question).ok()?,
        options,
        max_votes,
        hidden_results,
    })
}

#[cfg(test)]
mod tests {
    use super::{
        INITIAL_VOICE_BUCKET_MS, MAX_VOICE_LEVELS, message_avatar_alias, poll_creation_pending,
        poll_draft_values, record_voice_level, toggled_poll_votes, voice_session_retired,
    };
    use talk_core::ui::content::{AssetState, AssetTarget, AssetView};
    use talk_core::ui::conversation::RemoteState;
    use talk_core::ui::presentation::{
        ContentState, ConversationRef, DeliveryState, MessageView, RequestId,
    };
    use talk_core::ui::{
        AccountId, AccountScope, ConversationId, HostId, Page, RuntimeGeneration, Text,
    };

    #[test]
    fn recording_levels_remain_bounded_and_keep_peaks_from_the_entire_timeline() {
        let mut levels = Vec::new();
        let mut bucket_ms = INITIAL_VOICE_BUCKET_MS;
        for i in 0..=480 {
            let level = if i == 0 {
                900
            } else if i == 480 {
                700
            } else {
                100
            };
            record_voice_level(&mut levels, &mut bucket_ms, i * 250, level);
            assert!(levels.len() <= MAX_VOICE_LEVELS);
        }
        assert!(bucket_ms > INITIAL_VOICE_BUCKET_MS);
        assert_eq!(levels.first(), Some(&900));
        assert_eq!(levels.last(), Some(&700));
        record_voice_level(&mut levels, &mut bucket_ms, u64::MAX, u16::MAX);
        assert!(levels.len() <= MAX_VOICE_LEVELS);
        assert_eq!(levels.last(), Some(&1000));
    }

    #[test]
    fn voice_draft_survives_connection_loss_but_not_scope_or_permission_loss() {
        assert!(!voice_session_retired(
            true,
            ContentState::Offline,
            false,
            false
        ));
        assert!(!voice_session_retired(
            true,
            ContentState::Ready,
            false,
            false
        ));
        assert!(voice_session_retired(
            false,
            ContentState::Offline,
            false,
            false
        ));
        assert!(voice_session_retired(
            true,
            ContentState::Denied,
            false,
            false
        ));
        assert!(voice_session_retired(
            true,
            ContentState::Ready,
            true,
            false
        ));
    }

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
        let draft = poll_draft_values("  ¿Cuándo?  ", ["  Hoy", "Mañana  "], 0, true).unwrap();
        assert_eq!(draft.question.as_str(), "¿Cuándo?");
        assert_eq!(draft.options.as_slice()[0].as_str(), "Hoy");
        assert_eq!(draft.options.as_slice()[1].as_str(), "Mañana");
        assert_eq!(draft.max_votes, 0);
        assert!(draft.hidden_results);
        assert!(poll_draft_values("Q", ["Same", " Same "], 1, false).is_none());
        assert!(poll_draft_values("Q", ["One"], 1, false).is_none());
        assert!(poll_draft_values(" ", ["One", "Two"], 1, false).is_none());
        assert!(poll_draft_values("Q", ["One", "  "], 1, false).is_none());
        let oversized = "a".repeat(1001);
        assert!(poll_draft_values("Q", [oversized.as_str(), "Two"], 1, false).is_none());
        assert!(poll_draft_values("Q", ["One", "Two"], 3, false).is_none());
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
