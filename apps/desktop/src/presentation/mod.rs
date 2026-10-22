mod account_removal;
mod account_settings;
mod call;
mod collection;
mod content;
mod controls;
mod conversation_dialog;
mod creation;
mod device_settings;
mod feedback;
mod focus;
pub mod input;
pub mod localization;
mod messaging;
mod motion;
mod navigation;
mod onboarding;
mod organization;
mod overlays;
mod permissions;
pub mod preferences;
pub(super) mod probes;
mod settings;
mod settings_pages;
mod shell;
mod sidebar;
mod state;
pub mod theme;
mod window_chrome;

use gpui::{
    App, Context, Div, Entity, FocusHandle, Focusable, FontWeight, KeyDownEvent, Subscription,
    Window, div, prelude::*, px,
};
use input::TextInput;
use state::{CallEntry, Drafts, call_entry, valid_snapshot};
use std::time::Instant;
use talk_core::ui::presentation::{
    CallStage, Completion, ContentState, ConversationRef, ConversationView, NativeDesktopPort,
    PresentationSnapshot, RequestId, ServerInput, StorageChoice, UiIntent,
};
use talk_core::ui::{CallScope, Decision, MediaIntent, UserIntent};
use theme::{Appearance, Palette, Theme};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Overlay {
    Settings,
    RoomInfo,
    Participants,
    Files,
    Accounts,
    Thread,
    CallParticipants,
    CallChat,
    Search,
    Create,
    Discover,
    Organize,
    ContentPreview,
    CreatePoll,
}
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SidebarDestination {
    Conversations,
    Inbox,
    Archive,
}
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SearchKind {
    All,
    Messages,
    Files,
    Conversations,
    People,
}
#[derive(Clone)]
pub enum LocalAction {
    Window(window_chrome::Action),
    Content(content::Action),
    CreateMode(bool),
    FindPeople,
    FindSearchPeople,
    CreateConversation,
    CreateDirect(talk_core::ui::conversation::AccountPersonRef),
    CheckUpdates,
    InstallUpdate(u64),
    Room(conversation_dialog::Action),
    Message(messaging::Action),
    MessageIn(ConversationRef, messaging::Action),
    EmojiPicker(ConversationRef),
    Open(Overlay),
    Close,
    DismissFeedback,
    DismissIncoming,
    ShowIncoming,
    ShowNavigation,
    HideNavigation,
    ToggleCallChat,
    Navigate(ConversationRef),
    Destination(SidebarDestination),
    CollapseSection(sidebar::Section),
    Favorite(ConversationRef, bool),
    Archive(ConversationRef, bool),
    Send,
    BeginLogin,
    RetryLogin,
    AddAccount,
    Reauthenticate(talk_core::ui::AccountId),
    RemoveAccount(
        talk_core::ui::AccountId,
        Option<talk_core::ui::AccountScope>,
    ),
    ConfirmAccountRemoval,
    CancelAccountRemoval,
    RetryAccountRemoval,
    FinishOnboarding,
    OnboardingReady,
    RetryOnboardingHandoff,
    PermissionSetup(talk_core::preferences::permissions::PermissionKind),
    Storage(StorageChoice),
    Intent(UiIntent),
    Call,
    Accept,
    PrejoinJoin,
    ToggleMicIntent,
    ToggleCameraIntent,
    Theme(Palette),
    Appearance(Appearance),
    Density(bool),
    Scale(u16),
    Preference(preferences::PreferenceEdit),
    SettingsTab(preferences::SettingsTab),
    SettingsMenu,
    RetryPreferences(bool),
    TestNotification(talk_core::preferences::notifications::NotificationCategory),
    ReloadAccountPreferences,
    AccountPreference(talk_core::preferences::account::AccountPreferenceChange),
    SaveUploadFolder,
    RefreshDevice(talk_core::preferences::devices::DeviceKind),
    Permission(
        talk_core::preferences::permissions::PermissionKind,
        talk_core::preferences::permissions::PermissionAction,
    ),
    ToggleDevicePicker(talk_core::preferences::devices::DeviceKind),
    OpenDeviceSettings,
    SelectDevice(
        talk_core::preferences::devices::DeviceKind,
        talk_core::preferences::devices::DeviceSelection,
    ),
    StartDevice(talk_core::preferences::devices::DeviceKind),
    StopDevice(talk_core::preferences::devices::DeviceKind),
    Search,
    SearchKind(SearchKind),
    CreateGroup,
    RenameLocalGroup(talk_core::preferences::navigation::NavigationGroupId),
    MoveLocalGroup(talk_core::preferences::navigation::NavigationGroupId, bool),
    EditTag(talk_core::ui::presentation::TagEdit),
    RenameTag(talk_core::ui::presentation::TagRef),
    MoveTag(talk_core::ui::presentation::TagRef, bool),
    ToggleTag(talk_core::ui::presentation::TagRef),
    EditGroup(
        talk_core::ui::AccountScope,
        talk_core::preferences::navigation::NavigationEdit,
    ),
    PlaceInGroup(
        ConversationRef,
        Option<talk_core::preferences::navigation::NavigationGroupId>,
    ),
    LocateMessage(ConversationRef, u128),
    ReturnConversation,
    Thread(ConversationRef, u128),
    Notice(&'static str),
}
#[derive(Clone, Copy)]
pub enum ButtonStyle {
    Primary,
    Secondary,
    Quiet,
    Danger,
    Stage,
    StageDanger,
}

use crate::presentation::probes::Probe;

pub struct InitialView {
    pub probe: Probe,
    pub development: bool,
    pub palette: Palette,
    pub appearance: Appearance,
    pub overlay: Option<Overlay>,
    pub prejoin: bool,
    pub onboarding: bool,
    pub scale: u16,
}

use crate::presentation::feedback::Feedback;

pub struct Desktop {
    updates: talk_core::preferences::updates::UpdateSnapshot,
    // Successful install admission drains all service owners. This latch is
    // never cleared by an error snapshot; composition owns restart/quit.
    update_restart_pending: bool,
    room_ui: conversation_dialog::RoomUi,
    message_ui: messaging::MessageUi,
    content_ui: content::ContentUi,
    probe: Probe,
    background_inert: bool,
    control_focus: controls::FocusStates,
    navigation: navigation::Navigation,
    hover_hints: controls::HoverHints,
    hovered_control: Option<gpui::ElementId>,
    reduced_motion: bool,
    motion_trigger: Option<motion::MotionTrigger>,
    motion_frames: std::rc::Rc<std::cell::Cell<u64>>,
    emoji_picker_pending: Option<(ConversationRef, u64)>,
    emoji_picker_generation: u64,
    emoji_picker_context: Option<ConversationRef>,
    keyboard_navigation: bool,
    highlighted_message: Option<(ConversationRef, u128)>,
    dismissed_terminal_call: Option<CallScope>,
    pub snapshot: PresentationSnapshot,
    projection_blocked: bool,
    port: Box<dyn NativeDesktopPort>,
    pub development: bool,
    pub palette: Palette,
    pub appearance: Appearance,
    pub scale: u16,
    pub compact: bool,
    pub locale: talk_core::preferences::Locale,
    pub preferences: talk_core::preferences::PreferenceSnapshot,
    settings_port: Box<dyn preferences::ComputerSettingsPort>,
    settings_request: u64,
    preference_error: Option<talk_core::preferences::PreferenceError>,
    notification_state: talk_core::preferences::notifications::NotificationState,
    notification_error: Option<talk_core::preferences::notifications::NotificationError>,
    notification_test: Option<(
        talk_core::preferences::RequestId,
        talk_core::preferences::notifications::NotificationCategory,
    )>,
    settings_tab: preferences::SettingsTab,
    settings_navigation_open: bool,
    devices: Result<
        talk_core::preferences::devices::DeviceSnapshot,
        talk_core::preferences::devices::DeviceError,
    >,
    device_error: Option<talk_core::preferences::devices::DeviceError>,
    permission_ui: permissions::PermissionUi,
    onboarding_ui: onboarding::OnboardingUi,
    device_stopping: Option<talk_core::preferences::devices::PreviewSession>,
    microphone_auto_attempted: bool,
    device_request: u64,
    device_session_counter: u64,
    device_session: Option<talk_core::preferences::devices::PreviewSession>,
    applied_devices: Option<(
        talk_core::preferences::Revision,
        talk_core::preferences::devices::DevicePreferences,
    )>,
    camera_image: Option<gpui::ImageSource>,
    call_images: Vec<(CallScope, u128, std::time::Instant, gpui::ImageSource)>,
    device_picker: Option<talk_core::preferences::devices::DeviceKind>,
    pending_device_focus: Option<gpui::ElementId>,
    pub overlay: Option<Overlay>,
    destination: SidebarDestination,
    collapsed_sections: [bool; 5],
    archive_request: u64,
    pub navigation_open: bool,
    pub call_chat_open: bool,
    onboarding_open: bool,
    pub prejoin: Option<CallScope>,
    pub dismissed_incoming: Option<CallScope>,
    pub microphone_intent: bool,
    pub camera_intent: bool,
    feedback: Option<Feedback>,
    pub server: Entity<TextInput>,
    pub search: Entity<TextInput>,
    pub sidebar_filter: Entity<TextInput>,
    pub composer: Entity<TextInput>,
    composer_focus: FocusHandle,
    composer_active: bool,
    pub conversation_name: Entity<TextInput>,
    group_name: Entity<TextInput>,
    upload_folder: Entity<TextInput>,
    account_preferences: Option<talk_core::preferences::account::AccountPreferenceSnapshot>,
    account_preference_request: u64,
    account_preference_error: Option<talk_core::preferences::account::AccountPreferenceError>,
    account_removal: account_removal::AccountRemovalUi,
    pub search_kind: SearchKind,
    directory_query: Option<(talk_core::ui::AccountScope, String)>,
    pub thread_parent: Option<(ConversationRef, u128)>,
    drafts: Drafts,
    root_focus: FocusHandle,
    modal_focus: FocusHandle,
    pending_modal_focus: bool,
    pending_feedback_focus: bool,
    pending_navigation_focus: Option<bool>,
    diagnostic_frames: u8,
    return_focus: Option<FocusHandle>,
    prejoin_return_focus: Option<CallScope>,
    pending_prejoin_return: Option<CallScope>,
    feedback_return_focus: Option<FocusHandle>,
    incoming_return_focus: Option<FocusHandle>,
    last_incoming_scope: Option<CallScope>,
    _appearance_observer: Subscription,
    _input_observers: [Subscription; 3],
    _composer_focus_observers: [Subscription; 2],
    _dialog_observers: Vec<Subscription>,
    pending_ack: Option<talk_core::ui::presentation::SubmissionResult>,
    pub theme: Theme,
}
impl Desktop {
    pub fn new(
        mut port: Box<dyn NativeDesktopPort>,
        initial: InitialView,
        settings_port: Box<dyn preferences::ComputerSettingsPort>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let InitialView {
            probe,
            development,
            palette,
            appearance,
            overlay: initial_overlay,
            prejoin,
            onboarding,
            scale,
        } = initial;
        let snapshot = port
            .take_snapshot()
            .expect("composition supplies one bounded initial snapshot");
        assert!(
            valid_snapshot(&snapshot),
            "composition supplied inconsistent initial projection"
        );
        let initial_overlay = initial_overlay.or_else(|| {
            (snapshot.accounts.as_slice().is_empty()
                && !snapshot.saved_accounts.as_slice().is_empty())
            .then_some(Overlay::Accounts)
        });
        let theme = Theme::resolve(palette, appearance, cx.window_appearance());
        let root_focus = cx.focus_handle();
        let modal_focus = cx.focus_handle();
        let server = cx.new(|cx| TextInput::new("cloud.example.test/nextcloud", 2048, theme, cx));
        let search = cx.new(|cx| TextInput::new("Geladene Inhalte durchsuchen …", 256, theme, cx));
        let sidebar_filter = cx.new(|cx| TextInput::new("Gespräche filtern …", 256, theme, cx));
        let composer = cx.new(|cx| {
            let mut input = TextInput::new("Nachricht schreiben …", 4096, theme, cx);
            input.multiline = true;
            input
        });
        let conversation_name = cx.new(|cx| TextInput::new("Name des Gesprächs", 256, theme, cx));
        let group_name = cx.new(|cx| TextInput::new("Name der Gruppe", 128, theme, cx));
        let upload_folder = cx.new(|cx| TextInput::new("/Talk", 2048, theme, cx));
        let weak = cx.weak_entity();
        let observer = window.observe_window_appearance(move |_, cx| {
            let _ = weak.update(cx, |_, cx| cx.notify());
        });
        let input_observers = [
            cx.observe(&search, |_, _, cx| cx.notify()),
            cx.observe(&sidebar_filter, |_, _, cx| cx.notify()),
            cx.observe(&composer, |_, _, cx| cx.notify()),
        ];
        let composer_focus = cx.focus_handle().tab_stop(false);
        let composer_focus_observers = [
            cx.on_focus_in(&composer_focus, window, |_, _, cx| cx.notify()),
            cx.on_focus_out(&composer_focus, window, |this, _, _, cx| {
                this.content_ui.menu = false;
                cx.notify();
            }),
        ];
        let prejoin = if prejoin {
            snapshot
                .conversations
                .as_slice()
                .iter()
                .find(|r| Some(r.context) == snapshot.selected)
                .and_then(|r| match call_entry(r, snapshot.content) {
                    Some(CallEntry::Conference(call)) => Some(call),
                    _ => None,
                })
        } else {
            None
        };
        if initial_overlay.is_some() {
            modal_focus.focus(window)
        } else {
            root_focus.focus(window)
        };
        let preferences = settings_port.snapshot();
        let devices = settings_port.device_snapshot();
        let locale =
            if preferences.active.locale == talk_core::preferences::LocalePreference::System {
                settings_port.system_locale()
            } else {
                preferences.active.locale.resolve(&[])
            };
        let room_ui = conversation_dialog::RoomUi::new(theme, cx);
        let message_ui = messaging::MessageUi::new(cx);
        let content_ui =
            content::ContentUi::new(theme, snapshot.active_account, snapshot.selected, cx);
        let mut dialog_observers = Vec::new();
        for input in room_ui
            .inputs()
            .into_iter()
            .chain([message_ui.search.clone(), message_ui.time_input.clone()])
            .chain(content_ui.inputs().cloned())
        {
            dialog_observers.push(cx.observe(&input, |_, _, cx| cx.notify()));
        }
        let mut desktop = Self {
            updates: settings_port.update_snapshot(),
            update_restart_pending: false,
            room_ui,
            message_ui,
            content_ui,
            probe,
            background_inert: false,
            control_focus: controls::FocusStates::default(),
            navigation: navigation::Navigation::new(cx),
            hover_hints: controls::HoverHints::default(),
            hovered_control: None,
            reduced_motion: settings_port.reduced_motion(),
            motion_trigger: None,
            motion_frames: Default::default(),
            emoji_picker_pending: None,
            emoji_picker_generation: 0,
            emoji_picker_context: None,
            keyboard_navigation: false,
            highlighted_message: None,
            dismissed_terminal_call: None,
            snapshot,
            projection_blocked: false,
            port,
            development,
            palette,
            appearance,
            scale,
            compact: preferences.active.sidebar_density
                == talk_core::preferences::SidebarDensity::Compact,
            locale,
            settings_request: preferences.revision.0,
            preferences,
            permission_ui: permissions::PermissionUi::new(settings_port.permission_snapshot()),
            onboarding_ui: onboarding::OnboardingUi::new(),
            settings_port,
            preference_error: None,
            notification_state: talk_core::preferences::notifications::NotificationState::Idle,
            notification_error: None,
            notification_test: None,
            settings_navigation_open: false,
            devices,
            device_error: None,
            device_stopping: None,
            microphone_auto_attempted: false,
            device_request: 0,
            device_session_counter: 0,
            device_session: None,
            applied_devices: None,
            camera_image: None,
            call_images: Vec::with_capacity(16),
            device_picker: None,
            pending_device_focus: None,
            settings_tab: if development {
                preferences::SettingsTab::Appearance
            } else {
                preferences::SettingsTab::General
            },
            overlay: initial_overlay,
            destination: SidebarDestination::Conversations,
            collapsed_sections: [false; 5],
            archive_request: 0,
            navigation_open: false,
            call_chat_open: true,
            onboarding_open: onboarding,
            prejoin,
            dismissed_incoming: None,
            microphone_intent: true,
            camera_intent: false,
            feedback: None,
            server,
            search,
            sidebar_filter,
            composer,
            composer_focus,
            composer_active: false,
            conversation_name,
            group_name,
            upload_folder,
            account_preferences: None,
            account_preference_request: 0,
            account_preference_error: None,
            account_removal: account_removal::AccountRemovalUi::new(cx),
            search_kind: SearchKind::All,
            directory_query: None,
            thread_parent: None,
            drafts: Drafts::default(),
            root_focus,
            modal_focus,
            pending_modal_focus: initial_overlay.is_some() || prejoin.is_some(),
            pending_feedback_focus: false,
            pending_navigation_focus: None,
            diagnostic_frames: 0,
            return_focus: None,
            prejoin_return_focus: None,
            pending_prejoin_return: None,
            feedback_return_focus: None,
            incoming_return_focus: None,
            last_incoming_scope: None,
            _appearance_observer: observer,
            _input_observers: input_observers,
            _composer_focus_observers: composer_focus_observers,
            _dialog_observers: dialog_observers,
            pending_ack: None,
            theme,
        };
        if development && initial_overlay == Some(Overlay::Thread) {
            desktop.thread_parent = desktop.snapshot.selected.and_then(|c| {
                desktop
                    .snapshot
                    .messages
                    .as_slice()
                    .iter()
                    .find(|m| m.reply_count > 0)
                    .map(|m| (c, m.local_id))
            });
        }
        if development && initial_overlay == Some(Overlay::Search) {
            desktop.search.update(cx, |i, _| i.set_value("Navigation"));
        }
        if development {
            desktop
                .server
                .update(cx, |i, _| i.set_value("cloud.example.test"));
        }
        desktop.refresh_message_ui(cx);
        desktop
    }
    fn account_name(&self, account: &talk_core::ui::presentation::AccountView) -> String {
        if account.label.as_str().is_empty() {
            self.l("Konto").to_owned()
        } else {
            account.label.as_str().to_owned()
        }
    }
    pub fn refresh(&mut self, cx: &mut Context<Self>) {
        self.refresh_composer_mentions(cx);
        self.refresh_content_ui(cx);
        let reduced_motion = self.settings_port.reduced_motion();
        if self.reduced_motion != reduced_motion {
            self.reduced_motion = reduced_motion;
            self.motion_trigger = None;
            cx.notify();
        }
        self.refresh_preferences(cx);
        self.refresh_onboarding(cx);
        if let Some(result) = self.pending_ack
            && self
                .port
                .try_submit(UiIntent::AcknowledgeSubmission {
                    request: result.request,
                    context: result.context,
                })
                .is_ok()
        {
            self.pending_ack = None;
        }
        let Some(next) = self.port.take_snapshot() else {
            return;
        };
        if next.revision <= self.snapshot.revision {
            return;
        }
        self.hover_hints.cancel();
        if !valid_snapshot(&next) {
            self.projection_blocked = true;
            self.feedback = Some(Feedback::error(
                "Ungültiger Ansichtsstand verworfen. Bitte neu verbinden.",
            ));
            cx.notify();
            return;
        }
        self.projection_blocked = false;
        let previous = self.snapshot.selected;
        let previous_account = self.snapshot.active_account;
        if let Some(c) = previous
            && self.message_ui.editing.is_none()
        {
            let body = self.composer.read(cx).committed_value();
            let _ = self.drafts.save(c, body);
        }
        let mut reset_draft = false;
        if let Some(result) = next.submission {
            self.pending_ack = Some(result);
        }
        if let Some(result) = next.submission
            && self.drafts.complete(result)
        {
            self.feedback = match result.result {
                Completion::ServerAccepted => None,
                Completion::Failed(_) => Some(Feedback::error(
                    "Nicht gesendet. Dein Entwurf bleibt erhalten.",
                )),
                Completion::Uncertain => Some(Feedback::warning(
                    "Sendestatus unklar. Vor erneutem Senden muss der Status abgeglichen werden.",
                )),
            }
            .map(|feedback| feedback.in_conversation(result.context));
            reset_draft = Some(result.context) == previous;
            // A Busy acknowledgement leaves the same result in the producer's latest slot.
            self.pending_ack = Some(result);
        }
        if let Some(result) = next.organization_result {
            if result.result != Completion::ServerAccepted {
                self.feedback = Some(Feedback::error(
                    "Die Änderung konnte nicht bestätigt werden. Bitte neu laden.",
                ));
            }
            let _ = self.port.try_submit(UiIntent::AcknowledgeOrganization {
                request: result.request,
                account: result.account,
            });
        }
        if let Some(result) = next.mutation {
            if result.result != Completion::ServerAccepted {
                self.feedback = Some(
                    Feedback::error("Die Änderung konnte nicht bestätigt werden. Bitte neu laden.")
                        .in_conversation(result.context),
                );
            }
            let _ = self.port.try_submit(UiIntent::AcknowledgeMutation {
                request: result.request,
                context: result.context,
            });
        }
        if self.snapshot.saved_accounts.as_slice().is_empty()
            && !next.saved_accounts.as_slice().is_empty()
            && next.active_account.is_none()
            && next.login.phase == talk_core::ui::presentation::LoginPhase::EnterServer
        {
            self.overlay = Some(Overlay::Accounts);
            self.pending_modal_focus = true;
        }
        self.drafts.retain_accounts(&next);
        if previous != next.selected || previous_account != next.active_account {
            self.retire_emoji_picker();
        }
        self.refresh_account_removal(&next);
        self.snapshot = next;
        self.refresh_content_ui(cx);
        self.refresh_creation(cx);
        self.refresh_room_dialog(cx);
        self.refresh_message_ui(cx);
        if self.incoming_visible() {
            self.account_removal.dismiss_confirmation();
            if self.emoji_picker_pending.is_some() || self.emoji_picker_context.is_some() {
                self.retire_emoji_picker();
            }
            self.stop_device_preview(cx);
        }
        if previous_account != self.snapshot.active_account {
            self.permission_ui.microphone_ready = false;
            self.destination = SidebarDestination::Conversations;
            self.collapsed_sections = [false; 5];
            self.stop_device_preview(cx);
            self.feedback = None;
            self.highlighted_message = None;
            self.sidebar_filter.update(cx, |i, _| i.reset());
            self.feedback_return_focus = None;
            self.pending_feedback_focus = false;
            self.search.update(cx, |i, cx| {
                i.reset();
                cx.notify();
            });
            self.conversation_name.update(cx, |i, cx| {
                i.reset();
                cx.notify();
            });
            self.thread_parent = None;
        }
        if previous != self.snapshot.selected || reset_draft {
            let body = self
                .snapshot
                .selected
                .map_or("", |c| self.drafts.text(c))
                .to_owned();
            let changed_context = previous != self.snapshot.selected;
            self.composer.update(cx, |i, cx| {
                if changed_context {
                    i.set_context_value(&body);
                } else {
                    i.set_value(&body);
                }
                cx.notify();
            });
        }
        if let Some(prejoin) = self.prejoin
            && self.selected_call() != Some(CallEntry::Conference(prejoin))
        {
            self.prejoin = None;
            self.feedback = Some(Feedback::info(
                "Beitritt nicht mehr verfügbar. Gespräch und Freigaben müssen erneut geprüft werden.",
            ));
        }
        cx.notify();
    }
    fn submit(&mut self, intent: UiIntent) -> bool {
        let unavailable = match &intent {
            UiIntent::DownloadFile { .. } => {
                Some("Herunterladen ist in diesem Client noch nicht verfügbar.")
            }
            UiIntent::PickAttachment(_) => {
                Some("Dateien anhängen ist in diesem Client noch nicht verfügbar.")
            }
            UiIntent::ReopenBrowser(_) => {
                Some("Der Browser kann hier noch nicht erneut geöffnet werden.")
            }
            _ => None,
        };
        match self.port.try_submit(intent) {
            Ok(()) => true,
            Err(e) => {
                self.feedback = Some(
                    if let Some(label) = unavailable
                        && matches!(
                            e,
                            talk_core::ui::SubmitError::Unavailable(
                                talk_core::ui::UnavailableReason::Unsupported
                                    | talk_core::ui::UnavailableReason::Unconfigured
                            )
                        )
                    {
                        Feedback::info(label)
                    } else {
                        Feedback::from_error(e)
                    },
                );
                false
            }
        }
    }
    fn save_draft(&mut self, cx: &App) -> bool {
        if self.message_ui.editing.is_some() {
            return true;
        }
        let Some(context) = self.snapshot.selected else {
            return true;
        };
        match self
            .drafts
            .save(context, self.composer.read(cx).committed_value())
        {
            Ok(()) => true,
            Err(e) => {
                self.feedback = Some(Feedback::from_error(e));
                false
            }
        }
    }
    // Deferred GPUI elements render after the surrounding surface has finished.
    // Restore their captured interaction policy for every nested control.
    fn in_surface<R>(&mut self, inert: bool, render: impl FnOnce(&Self) -> R) -> R {
        let previous = self.background_inert;
        self.background_inert = inert;
        let result = render(self);
        self.background_inert = previous;
        result
    }
    pub fn selected(&self) -> Option<&ConversationView> {
        self.snapshot
            .conversations
            .as_slice()
            .iter()
            .find(|r| Some(r.context) == self.snapshot.selected)
    }
    fn selected_call(&self) -> Option<CallEntry> {
        self.selected()
            .and_then(|row| call_entry(row, self.snapshot.content))
    }
    fn composer_available(&self) -> bool {
        !self.update_restart_pending
            && composer_allowed(
                self.overlay,
                self.prejoin.is_some(),
                self.incoming_visible(),
            )
    }
    pub fn handle(&mut self, action: LocalAction, window: &mut Window, cx: &mut Context<Self>) {
        // Window controls stay available while the content is modal or inert,
        // just like the system controls they replace.
        if let LocalAction::Window(action) = action {
            action.perform(window);
            return;
        }
        self.motion_trigger = None;
        if self.update_restart_pending {
            return;
        }
        if self.account_removal.confirming() {
            match action {
                LocalAction::Close | LocalAction::CancelAccountRemoval => {
                    self.cancel_account_removal(window, cx);
                }
                LocalAction::ConfirmAccountRemoval => self.confirm_account_removal(window, cx),
                _ => {}
            }
            cx.notify();
            return;
        }
        if self.projection_blocked
            && !matches!(
                &action,
                LocalAction::Intent(UiIntent::Legacy(UserIntent::Leave(_)))
            )
        {
            return;
        }
        if self.local_preference_action(&action, cx) {
            return;
        }
        if matches!(
            &action,
            LocalAction::Destination(_)
                | LocalAction::ShowNavigation
                | LocalAction::ToggleCallChat
                | LocalAction::Call
                | LocalAction::Accept
                | LocalAction::PrejoinJoin
        ) {
            self.retire_emoji_picker();
        }
        if self.settings_tab == preferences::SettingsTab::Devices
            && matches!(
                &action,
                LocalAction::Close
                    | LocalAction::Open(_)
                    | LocalAction::SettingsTab(_)
                    | LocalAction::Navigate(_)
                    | LocalAction::Call
                    | LocalAction::Accept
                    | LocalAction::PrejoinJoin
            )
        {
            self.stop_device_preview(cx);
        }
        let reveal_result = matches!(
            &action,
            LocalAction::Intent(UiIntent::DownloadFile { .. } | UiIntent::Refresh(_))
                | LocalAction::BeginLogin
                | LocalAction::RetryLogin
        );
        self.hover_hints.cancel();
        let previous_feedback = self.feedback;
        let invoker = window.focused(cx);
        let mut server_input_rejected = false;
        match action {
            LocalAction::RemoveAccount(account, scope) => {
                self.request_account_removal(account, scope, window, cx)
            }
            LocalAction::RetryAccountRemoval => self.retry_account_removal(window, cx),
            LocalAction::ConfirmAccountRemoval | LocalAction::CancelAccountRemoval => {}
            LocalAction::Window(action) => action.perform(window),
            LocalAction::Content(action) => self.content_action(action, window, cx),
            LocalAction::CreateMode(direct) => self.room_ui.create_direct = direct,
            LocalAction::FindPeople => {
                if let Ok(query) =
                    talk_core::ui::Text::new(self.room_ui.search.read(cx).content.trim())
                {
                    self.submit_creation(talk_core::ui::conversation::CreateAction::FindPeople(
                        query,
                    ));
                }
            }
            LocalAction::FindSearchPeople => self.find_search_people(cx),
            LocalAction::CreateConversation => {
                if let Ok(name) =
                    talk_core::ui::Text::new(self.conversation_name.read(cx).content.trim())
                {
                    self.submit_creation(talk_core::ui::conversation::CreateAction::Group(name));
                }
            }
            LocalAction::CreateDirect(person) => {
                self.submit_creation(talk_core::ui::conversation::CreateAction::Direct(person))
            }
            LocalAction::CheckUpdates => {
                if let Err(error) = self.settings_port.check_updates() {
                    self.updates.state =
                        talk_core::preferences::updates::UpdateState::Failed(error);
                }
            }
            LocalAction::InstallUpdate(generation) => {
                if generation == self.updates.generation
                    && self.can_install_update()
                    && self.save_draft(cx)
                {
                    if self.drafts.has_unsent()
                        || !self.composer.read(cx).content.is_empty()
                        || self.message_ui.pending_edit.is_some()
                        || self.input_composing(cx)
                    {
                        self.feedback = Some(Feedback::info(
                            "Schließe ungesendete Nachrichten vor dem Neustart ab oder leere sie.",
                        ));
                    } else {
                        match self.settings_port.install_update(generation) {
                            Ok(()) => self.freeze_for_update(window, cx),
                            Err(error) => {
                                self.feedback =
                                    Some(Feedback::error(preferences::update_error_label(error)));
                            }
                        }
                    }
                }
            }
            LocalAction::Room(action) => self.room_action(action, cx),
            LocalAction::Message(action) => self.message_action(action, window, cx),
            LocalAction::MessageIn(context, action) => {
                if Some(context) == self.snapshot.selected {
                    self.message_action(action, window, cx);
                }
            }
            LocalAction::EmojiPicker(context) => {
                if Some(context) == self.snapshot.selected {
                    self.show_composer_emoji(window, cx);
                }
            }
            LocalAction::Open(overlay) => {
                if !self.save_draft(cx) {
                    cx.notify();
                    return;
                }
                self.retire_emoji_picker();
                if self.overlay.is_none() {
                    self.return_focus = window.focused(cx);
                }
                self.overlay = Some(overlay);
                if overlay == Overlay::Create {
                    self.open_creation(cx);
                }
                if matches!(overlay, Overlay::RoomInfo | Overlay::Participants) {
                    self.open_room_dialog(overlay, cx);
                }
                self.pending_modal_focus = true;
                if overlay == Overlay::Search {
                    self.directory_query = None;
                    self.search.focus_handle(cx).focus(window);
                } else {
                    self.modal_focus.focus(window);
                }
            }
            LocalAction::Thread(context, id) => {
                if Some(context) == self.snapshot.selected
                    && ((self.thread_parent == Some((context, id))
                        && self.overlay == Some(Overlay::Thread))
                        || self
                            .snapshot
                            .messages
                            .as_slice()
                            .iter()
                            .any(|m| m.local_id == id || m.metadata.thread == Some(id)))
                {
                    self.thread_parent = Some((context, id));
                    let request =
                        self.submit_message(talk_core::ui::messaging::MessageAction::Thread(id));
                    self.message_ui.thread_load = Some(messaging::ThreadLoad {
                        context,
                        root: id,
                        request,
                    });
                    self.handle(LocalAction::Open(Overlay::Thread), window, cx);
                }
            }
            LocalAction::SearchKind(kind) => self.search_kind = kind,
            LocalAction::CreateGroup => self.create_group(cx),
            LocalAction::RenameLocalGroup(id) => self.rename_local_group(id, cx),
            LocalAction::MoveLocalGroup(id, up) => self.move_local_group(id, up, cx),
            LocalAction::EditTag(edit) => self.edit_tag(edit),
            LocalAction::RenameTag(id) => self.rename_tag(id, cx),
            LocalAction::MoveTag(id, up) => self.move_tag(id, up),
            LocalAction::ToggleTag(id) => self.toggle_tag(id),
            LocalAction::EditGroup(scope, edit) => self.edit_group(scope, edit, cx),
            LocalAction::PlaceInGroup(context, group) => self.place_in_group(context, group, cx),
            LocalAction::LocateMessage(context, id) => {
                if Some(context) == self.snapshot.selected
                    && let Some(index) = self
                        .snapshot
                        .messages
                        .as_slice()
                        .iter()
                        .position(|m| m.local_id == id)
                {
                    self.overlay = None;
                    self.return_focus = None;
                    self.highlighted_message = Some((context, id));
                    self.message_ui.list.scroll_to_reveal_item(index);
                    self.navigation.locate_message(index, window);
                }
            }
            LocalAction::ReturnConversation => {
                if let Some(call) = self.snapshot.call.as_ref()
                    && matches!(call.stage, CallStage::Ended | CallStage::Failed)
                {
                    let scope = call.scope;
                    let context = ConversationRef {
                        account: scope.account,
                        conversation: scope.conversation,
                    };
                    if self.snapshot.selected != Some(context)
                        && self
                            .snapshot
                            .conversations
                            .as_slice()
                            .iter()
                            .any(|r| r.context == context)
                        && !self.submit(UiIntent::OpenConversation(context))
                    {
                        return;
                    }
                    self.dismissed_terminal_call = Some(scope);
                    self.root_focus.focus(window);
                }
            }
            LocalAction::DismissFeedback => {
                self.feedback = None;
                self.navigation.reveal.request();
                if let Some(focus) = self
                    .feedback_return_focus
                    .take()
                    .filter(|f| self.root_focus.contains(f, window))
                {
                    focus.focus(window);
                } else {
                    self.root_focus.focus(window);
                }
            }
            LocalAction::ShowNavigation => {
                self.navigation_open = true;
                self.pending_navigation_focus = Some(true);
            }
            LocalAction::HideNavigation => {
                self.navigation_open = false;
                self.pending_navigation_focus = Some(false);
            }
            LocalAction::ToggleCallChat => {
                if window.viewport_size().width < px(1000.) || self.scale >= 150 {
                    self.handle(LocalAction::Open(Overlay::CallChat), window, cx);
                } else {
                    self.call_chat_open = !self.call_chat_open;
                }
            }
            LocalAction::DismissIncoming => {
                self.dismissed_incoming = self.snapshot.call.as_ref().map(|c| c.scope);
                if let Some(focus) = self
                    .incoming_return_focus
                    .take()
                    .filter(|f| self.root_focus.contains(f, window))
                {
                    focus.focus(window);
                } else {
                    self.root_focus.focus(window);
                }
            }
            LocalAction::ShowIncoming => self.dismissed_incoming = None,
            LocalAction::Close => {
                if self.close_content_overlay() {
                    if self.overlay.is_none() {
                        if let Some(focus) = self
                            .return_focus
                            .take()
                            .filter(|f| self.root_focus.contains(f, window))
                        {
                            focus.focus(window);
                        } else {
                            self.root_focus.focus(window);
                        }
                    }
                    cx.notify();
                    return;
                }
                self.retire_emoji_picker();
                self.room_ui.password.update(cx, |i, _| i.reset());
                if self.navigation_open {
                    self.pending_navigation_focus = Some(false);
                }
                self.navigation_open = false;
                let target = if self.overlay.take().is_some() {
                    self.return_focus.take()
                } else if self.prejoin.take().is_some() {
                    self.pending_prejoin_return = self.prejoin_return_focus.take();
                    None
                } else {
                    None
                };
                if let Some(focus) = target.filter(|f| self.root_focus.contains(f, window)) {
                    focus.focus(window);
                } else {
                    self.root_focus.focus(window);
                }
            }
            LocalAction::Navigate(context) => {
                if self.save_draft(cx) && self.submit(UiIntent::OpenConversation(context)) {
                    self.retire_emoji_picker();
                    self.navigation_open = false;
                    if self.destination == SidebarDestination::Inbox {
                        self.destination = SidebarDestination::Conversations;
                    }
                    if self.overlay == Some(Overlay::Search) {
                        self.overlay = None;
                        self.root_focus.focus(window);
                    }
                }
            }
            LocalAction::Destination(destination) => self.destination = destination,
            LocalAction::CollapseSection(section) => self.collapse_sidebar_section(section, cx),
            LocalAction::Favorite(context, favorite) => {
                if let Some(row) = self
                    .snapshot
                    .conversations
                    .as_slice()
                    .iter()
                    .find(|r| r.context == context && self.can_favorite(r))
                {
                    if row.organization.favorites
                        == talk_core::ui::presentation::FeatureSupport::Unsupported
                    {
                        if let Some(room) = row.navigation_key.clone() {
                            self.edit_navigation(talk_core::preferences::navigation::RemoteNavigationSupport::Unsupported,
                                talk_core::preferences::navigation::NavigationEdit::SetFavorite { room, favorite: Some(favorite) }, cx);
                        }
                    } else if let Some(request) = self.archive_request.checked_add(1) {
                        self.archive_request = request;
                        self.submit(UiIntent::SetFavorite {
                            request: RequestId(request),
                            context,
                            favorite,
                        });
                    }
                }
            }
            LocalAction::Archive(context, archived) => {
                if self.snapshot.conversation_list.content == ContentState::Ready
                    && self.snapshot.conversations.as_slice().iter().any(|r| {
                        r.context == context
                            && r.archive == Decision::Allowed
                            && r.archived == Some(!archived)
                    })
                    && let Some(request) = self.archive_request.checked_add(1)
                {
                    self.archive_request = request;
                    self.submit(UiIntent::SetArchived {
                        request: RequestId(request),
                        context,
                        archived,
                    });
                }
            }
            LocalAction::Send => {
                if self.composer.read(cx).composing() {
                    return;
                }
                if self.send_edit(cx) {
                    cx.notify();
                    return;
                }
                // Enter follows the disabled Send button once a confirmed
                // message has cleared the field; it is not a failed action.
                if self.composer.read(cx).content.trim().is_empty() {
                    return;
                }
                if self.save_draft(cx)
                    && let Some(row) = self.selected()
                {
                    let context = row.context;
                    let gate = if self.snapshot.content == ContentState::Ready {
                        row.send
                    } else {
                        Decision::Unknown
                    };
                    self.feedback = Some(
                        (match self.drafts.send(context, gate, self.port.as_mut()) {
                            Ok(_) => Feedback::info("Senden wird vorbereitet …"),
                            Err(e) => Feedback::from_error(e),
                        })
                        .in_conversation(context),
                    );
                }
            }
            LocalAction::FinishOnboarding => {
                if (self.snapshot.login.phase == talk_core::ui::presentation::LoginPhase::Ready
                    || self.onboarding_ui.resumed)
                    && self.snapshot.active_account.is_some()
                {
                    self.finish_onboarding(cx);
                    self.root_focus.focus(window);
                }
            }
            LocalAction::OnboardingReady => {
                if self.onboarding_open && self.snapshot.active_account.is_some() {
                    self.onboarding_ui.ready = true;
                    self.navigation.reset_onboarding();
                }
            }
            LocalAction::RetryOnboardingHandoff => self.retry_onboarding_handoff(cx),
            LocalAction::PermissionSetup(kind) => {
                if self.onboarding_open
                    && self.overlay.is_none()
                    && self.snapshot.login.phase == talk_core::ui::presentation::LoginPhase::Ready
                {
                    self.enter_onboarding_step(kind);
                    self.root_focus.focus(window);
                }
            }
            LocalAction::Reauthenticate(account) => {
                if self.save_draft(cx) && self.submit(UiIntent::Reauthenticate(account)) {
                    self.reset_onboarding(cx);
                    self.server.update(cx, |input, _| input.reset());
                    self.onboarding_open = true;
                    self.overlay = None;
                    self.server.focus_handle(cx).focus(window);
                }
            }
            LocalAction::AddAccount => {
                if self.save_draft(cx) && self.submit(UiIntent::NewLogin) {
                    self.reset_onboarding(cx);
                    self.onboarding_open = true;
                    self.overlay = None;
                    self.server.focus_handle(cx).focus(window);
                }
            }
            LocalAction::BeginLogin => match ServerInput::new(&self.server.read(cx).content) {
                Ok(server) => {
                    self.feedback = None;
                    self.pending_feedback_focus = false;
                    self.feedback_return_focus = None;
                    self.onboarding_open = true;
                    self.submit(UiIntent::BeginLogin {
                        request: RequestId(self.snapshot.revision.saturating_add(1)),
                        server,
                    });
                }
                Err(_) => {
                    server_input_rejected = true;
                    self.feedback = Some(Feedback::error(
                        "Bitte eine HTTPS-Serveradresse ohne Zugangsdaten, Abfrage oder Fragment eingeben.",
                    ))
                }
            },
            LocalAction::RetryLogin => {
                self.feedback = None;
                self.submit(UiIntent::RetryLogin {
                    request: RequestId(self.snapshot.revision.saturating_add(1)),
                });
            }
            LocalAction::Storage(choice) => {
                if let Some(attempt) = self.snapshot.login.attempt {
                    self.submit(UiIntent::ChooseStorage { attempt, choice });
                }
            }
            LocalAction::Intent(intent) => {
                let cancelled_login = matches!(intent, UiIntent::CancelLogin(_));
                if self.submit(intent)
                    && cancelled_login
                    && !self.snapshot.accounts.as_slice().is_empty()
                {
                    self.onboarding_open = false;
                }
            }
            LocalAction::Call => match self.selected_call() {
                Some(CallEntry::Direct(call)) => {
                    self.submit(UiIntent::StartDirect {
                        call,
                        media: MediaIntent::MicrophoneOnCameraOff,
                    });
                }
                Some(CallEntry::Conference(call)) => {
                    if self.submit(UiIntent::Legacy(UserIntent::PrepareConference(call))) {
                        self.prejoin = Some(call);
                        self.microphone_intent = true;
                        self.camera_intent = false;
                        self.prejoin_return_focus = Some(call);
                        self.modal_focus.focus(window);
                        self.pending_modal_focus = true;
                    }
                }
                None => {
                    self.feedback = Some(Feedback::info(
                        "Anrufe sind für dieses Gespräch noch nicht verfügbar.",
                    ))
                }
            },
            LocalAction::Accept => {
                if let Some(call) = &self.snapshot.call
                    && call.direct
                    && call.stage == CallStage::Incoming
                    && call.accept == Decision::Allowed
                {
                    self.submit(UiIntent::Legacy(UserIntent::AcceptDirect {
                        call: call.scope,
                        media: MediaIntent::MicrophoneOnCameraOff,
                    }));
                }
            }
            LocalAction::PrejoinJoin => {
                if let Some(call) = self.prejoin
                    && self.selected_call() == Some(CallEntry::Conference(call))
                    && self.submit(UiIntent::JoinConference {
                        call,
                        microphone: self.microphone_intent,
                        camera: self.camera_intent,
                    })
                {
                    self.prejoin = None;
                }
            }
            LocalAction::ToggleMicIntent => self.microphone_intent = !self.microphone_intent,
            LocalAction::ToggleCameraIntent => self.camera_intent = !self.camera_intent,
            LocalAction::Theme(_)
            | LocalAction::Appearance(_)
            | LocalAction::Density(_)
            | LocalAction::Scale(_) => {}
            LocalAction::Preference(edit) => self.edit_preference(edit, cx),
            LocalAction::RetryPreferences(effects) => self.retry_preferences(effects, cx),
            LocalAction::SettingsMenu => {
                self.settings_navigation_open = !self.settings_navigation_open
            }
            LocalAction::SettingsTab(tab) => {
                self.device_picker = None;
                self.pending_device_focus = None;
                self.settings_tab = tab;
                self.overlay = Some(Overlay::Settings);
                self.settings_navigation_open = false;
                if tab == preferences::SettingsTab::Devices {
                    self.device_picker = None;
                    self.enter_device_page();
                }
                if tab == preferences::SettingsTab::Privacy {
                    self.queue_permission_check(
                        talk_core::preferences::permissions::PermissionKind::ScreenSharing,
                    );
                }
                if matches!(
                    tab,
                    preferences::SettingsTab::Privacy | preferences::SettingsTab::Files
                ) {
                    self.load_account_preferences(cx);
                }
                if let Some(focus) = self.control_focus.current(&"settings-menu".into()) {
                    focus.focus(window);
                }
                self.hover_hints.cancel();
            }
            LocalAction::OpenDeviceSettings => {
                self.handle(LocalAction::Open(Overlay::Settings), window, cx);
                self.settings_tab = preferences::SettingsTab::Devices;
                self.enter_device_page();
            }
            LocalAction::ToggleDevicePicker(kind) => self.toggle_device_picker(kind),
            LocalAction::RefreshDevice(kind) => self.refresh_device_kind(kind),
            LocalAction::Permission(kind, action) => {
                if self.prepare_onboarding_permission(kind, action, cx) {
                    self.permission_action(kind, action);
                }
            }
            LocalAction::SelectDevice(kind, selection) => {
                self.device_picker = None;
                self.pending_device_focus = None;
                if let Some(focus) = self
                    .control_focus
                    .current(&("device-picker", kind as usize).into())
                {
                    focus.focus(window);
                }
                let mut values = self.preferences.active.clone();
                match kind {
                    talk_core::preferences::devices::DeviceKind::Microphone => {
                        values.devices.microphone = selection
                    }
                    talk_core::preferences::devices::DeviceKind::Speaker => {
                        values.devices.speaker = selection
                    }
                    talk_core::preferences::devices::DeviceKind::Camera => {
                        values.devices.camera = selection
                    }
                }
                self.stop_device_preview(cx);
                self.microphone_auto_attempted = false;
                self.commit_preferences(values, cx);
            }
            LocalAction::StartDevice(kind) => self.start_device_preview(kind),
            LocalAction::StopDevice(kind) => self.stop_one_device(kind, cx),
            LocalAction::ReloadAccountPreferences => self.load_account_preferences(cx),
            LocalAction::AccountPreference(change) => self.change_account_preference(change, cx),
            LocalAction::SaveUploadFolder => {
                match talk_core::preferences::account::NextcloudFolder::new(
                    self.upload_folder.read(cx).content.as_str(),
                ) {
                    Ok(folder) => self.change_account_preference(
                        talk_core::preferences::account::AccountPreferenceChange::AttachmentFolder(
                            folder,
                        ),
                        cx,
                    ),
                    Err(error) => self.account_preference_error = Some(error),
                }
            }
            LocalAction::TestNotification(category) => {
                if let Some(request) = self.settings_request.checked_add(1) {
                    self.settings_request = request;
                    self.notification_test =
                        Some((talk_core::preferences::RequestId(request), category));
                    self.notification_error = self
                        .settings_port
                        .test_notification(
                            talk_core::preferences::RequestId(request),
                            category,
                            self.l("Testbenachrichtigung"),
                            self.l("Benachrichtigungen für euren Arbeitsplatz."),
                        )
                        .err();
                }
            }
            LocalAction::Search => {
                self.handle(LocalAction::Open(Overlay::Search), window, cx);
            }
            LocalAction::Notice(text) => self.feedback = Some(Feedback::info(text)),
        }
        // Immediate fixture updates are projection changes, never invented backend outcomes.
        self.refresh(cx);
        if self.feedback.is_some() && self.feedback != previous_feedback {
            self.feedback_return_focus = invoker;
            self.pending_feedback_focus = reveal_result && self.overlay.is_none();
        }
        if server_input_rejected {
            // Let the user correct the field directly. Moving focus to the
            // banner also scrolls the entire form when vertical space is tight.
            let focus = self.server.focus_handle(cx);
            focus.focus(window);
            self.feedback_return_focus = Some(focus);
            self.pending_feedback_focus = false;
        }
        cx.notify();
    }
    fn key(&mut self, event: &KeyDownEvent, window: &mut Window, cx: &mut Context<Self>) {
        let diagnostic = self.development && event.keystroke.key == "f7";
        if (self.update_restart_pending && !diagnostic) || self.input_composing(cx) {
            return;
        }
        let key = &event.keystroke.key;
        let modifiers = &event.keystroke.modifiers;
        use talk_core::preferences::shortcuts::{ActionId, KeyContext, Modifiers, matching};
        let primary = if cfg!(target_os = "macos") {
            modifiers.platform
        } else {
            modifiers.control
        };
        let registry_modifiers =
            if modifiers.alt || (cfg!(target_os = "macos") && modifiers.control) {
                None
            } else {
                Some(match (primary, modifiers.shift) {
                    (true, true) => Modifiers::PrimaryShift,
                    (true, false) => Modifiers::Primary,
                    (false, true) => Modifiers::Shift,
                    (false, false) => Modifiers::None,
                })
            };
        let action = registry_modifiers.and_then(|m| matching(key, m, KeyContext::Application));
        if key == "f7" && self.development && self.diagnostic_frames < 64 {
            self.diagnostic_frames += 1;
            self.trace_chat_ui(window, cx);
            println!(
                "UI_LOCAL_STATE editor_chars={} editor_focused={} history_focused={} filter_chars={} search_chars={} overlay={:?} prejoin={} highlighted={:?} keyboard_navigation={}",
                self.composer.read(cx).content.len(),
                self.composer.focus_handle(cx).is_focused(window),
                self.navigation.history_focus().is_focused(window),
                self.sidebar_filter.read(cx).content.len(),
                self.search.read(cx).content.len(),
                self.overlay,
                self.prejoin.is_some(),
                self.highlighted_message.map(|(_, id)| id),
                self.keyboard_navigation
            );
            let collection = collection::CollectionState::new(
                self.snapshot.content,
                !self.snapshot.messages.as_slice().is_empty(),
            );
            println!(
                "UI_INTERACTION overlay_focus={} server_focus={} editing={:?} content={:?} retained={} history_visible={} confirmed_empty={} thread_state={:?} thread_request={:?}",
                self.modal_focus.contains_focused(window, cx),
                self.server.focus_handle(cx).is_focused(window),
                self.message_ui.editing,
                self.snapshot.content,
                self.snapshot.messages.as_slice().len(),
                collection.shows_content()
                    && f32::from(self.message_ui.list.viewport_bounds().size.height) > 0.,
                collection.confirmed_empty(),
                self.message_ui
                    .thread_load
                    .as_ref()
                    .map(|load| load.content_state(self.snapshot.message_panel.as_ref())),
                self.snapshot.message_panel.as_ref().and_then(|panel| {
                    use talk_core::ui::conversation::RemoteState;
                    match panel.thread_state {
                        RemoteState::Idle => None,
                        RemoteState::Loading(id)
                        | RemoteState::Saving(id)
                        | RemoteState::Failed(id, _)
                        | RemoteState::Uncertain(id) => Some(id.0),
                    }
                })
            );
            println!(
                "UI_SETTINGS revision={} durable={} locale={:?} palette={:?} appearance={:?} density={:?} tab={:?} load={:?} save={:?} startup={:?} background={} quick_switcher={:?}",
                self.preferences.revision.0,
                self.preferences.durable_revision.0,
                self.preferences.active.locale,
                self.palette,
                self.appearance,
                self.preferences.active.sidebar_density,
                self.settings_tab,
                self.preferences.load,
                self.preferences.save,
                self.preferences.startup,
                self.preferences.active.start_in_background,
                self.preferences.active.quick_switcher
            );
            println!(
                "UI_NOTIFICATIONS active={:?} result={:?} current_test={:?} focus_test={:?}",
                self.preferences.active.notifications,
                self.notification_state,
                self.notification_test,
                (0..4usize).find(|i| self
                    .control_focus
                    .current(&("test-notification", *i).into())
                    .is_some_and(|h| h.is_focused(window)))
            );
            if let Ok(devices) = &self.devices {
                println!(
                    "UI_DEVICES inventory={} session={:?} microphone={:?} camera={:?} speaker={:?} level={:?} output_level={:?} rendered_frame={} runtime={:?}",
                    devices.devices.as_slice().len(),
                    devices.session,
                    devices.microphone_preview,
                    devices.camera_preview,
                    devices.speaker_test,
                    devices.microphone_level,
                    devices.speaker_level,
                    self.camera_image.is_some(),
                    devices.runtime
                );
            }
            println!("UI_HISTORY {:?}", self.navigation.history_geometry());
            self.trace_message_motion();
            println!("UI_RESTART_LATCH active={}", self.update_restart_pending);
            println!("UI_HOVER {:?}", self.hover_hints.diagnostic());
            println!(
                "UI_ACTION_FOCUS {:?}",
                self.control_focus.named_action(window)
            );
            println!(
                "UI_GEOMETRY search={:?} focus={:?} viewports={:?}",
                self.search.read(cx).geometry(),
                self.navigation.reveal.geometry(),
                self.navigation.geometry()
            );
            cx.stop_propagation();
        } else if key == "f8" && self.development && self.probe == Probe::Messages {
            // Explicit fictional UI fixture: exercise the admitted UI latch only.
            // This never reserves an updater, installs files or reports success.
            println!("UI_RESTART_LATCH fixture=true no_install=true");
            self.freeze_for_update(window, cx);
            cx.notify();
            cx.stop_propagation();
        } else if matches!(
            action,
            Some(ActionId::NextControl | ActionId::PreviousControl)
        ) {
            self.navigation.reveal.request();
            if modifiers.shift {
                window.focus_prev();
            } else {
                window.focus_next();
            }
            cx.stop_propagation();
        } else if action == Some(ActionId::Close) {
            if self.conversation_search_open() && self.composer_available() {
                self.handle(
                    LocalAction::Message(messaging::Action::CloseSearch),
                    window,
                    cx,
                );
                cx.stop_propagation();
                return;
            }
            self.handle(
                if self.incoming_visible() {
                    LocalAction::DismissIncoming
                } else if self.overlay.is_none()
                    && self.prejoin.is_none()
                    && self.feedback.is_some()
                {
                    LocalAction::DismissFeedback
                } else {
                    LocalAction::Close
                },
                window,
                cx,
            );
            cx.stop_propagation();
        } else if !self.incoming_visible() && action == Some(ActionId::OpenSettings) {
            self.handle(LocalAction::Open(Overlay::Settings), window, cx);
            cx.stop_propagation();
        } else if key == "enter"
            && self.overlay == Some(Overlay::Search)
            && self.search.focus_handle(cx).is_focused(window)
            && !self.search.read(cx).composing()
        {
            self.handle(LocalAction::FindSearchPeople, window, cx);
            cx.stop_propagation();
        } else if self.composer_available()
            && !self.onboarding_open
            && self.destination != SidebarDestination::Inbox
            && action == Some(ActionId::ConversationSearch)
        {
            self.handle(
                LocalAction::Message(messaging::Action::ToggleSearch),
                window,
                cx,
            );
            cx.stop_propagation();
        } else if !self.incoming_visible()
            && self.overlay.is_none()
            && action == Some(ActionId::Search)
        {
            self.handle(LocalAction::Search, window, cx);
            cx.stop_propagation();
        } else if key == "f6"
            && self.development
            && ((matches!(self.probe, Probe::Focus | Probe::Messages) && self.overlay.is_none())
                || (self.probe == Probe::Accounts && self.overlay == Some(Overlay::Accounts)))
        {
            if let Some(context) = self.snapshot.selected {
                let _ = self.port.try_submit(UiIntent::Refresh(context));
                self.refresh(cx);
            }
            cx.stop_propagation();
        } else if self.composer_available()
            && registry_modifiers.and_then(|m| matching(key, m, KeyContext::Composer))
                == Some(ActionId::SendMessage)
            && self.composer.focus_handle(cx).is_focused(window)
        {
            self.handle(LocalAction::Send, window, cx);
            cx.stop_propagation();
        }
    }
    fn freeze_for_update(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.update_restart_pending = true;
        self.disable_update_inputs(cx);
        self.retire_emoji_picker();
        self.hover_hints.cancel();
        self.hovered_control = None;
        self.overlay = None;
        self.return_focus = None;
        self.pending_modal_focus = false;
        self.pending_feedback_focus = false;
        self.pending_navigation_focus = None;
        self.pending_prejoin_return = None;
        self.root_focus.focus(window);
    }
    fn disable_update_inputs(&self, cx: &mut Context<Self>) {
        for input in [
            &self.server,
            &self.search,
            &self.sidebar_filter,
            &self.composer,
            &self.conversation_name,
            &self.group_name,
            &self.upload_folder,
            &self.message_ui.search,
            &self.message_ui.time_input,
        ]
        .into_iter()
        .chain(self.room_ui.inputs().iter())
        .chain(self.content_ui.inputs())
        {
            input.update(cx, |input, _| input.interactive = false);
        }
    }
    fn input_composing(&self, cx: &App) -> bool {
        [
            &self.server,
            &self.search,
            &self.sidebar_filter,
            &self.composer,
            &self.conversation_name,
            &self.group_name,
            &self.upload_folder,
            &self.message_ui.search,
            &self.message_ui.time_input,
        ]
        .into_iter()
        .chain(self.room_ui.inputs().iter())
        .chain(self.content_ui.inputs())
        .any(|input| {
            let input = input.read(cx);
            input.interactive && input.composing()
        })
    }
    pub fn sp(&self, value: f32) -> gpui::Pixels {
        px(value * f32::from(self.scale) / 100.)
    }
    pub fn title(&self, text: &str) -> Div {
        div()
            .text_size(self.sp(24.))
            .font_weight(FontWeight::BOLD)
            .child(text.to_owned())
    }
    pub fn hint(&self, text: &str) -> Div {
        div()
            .text_size(self.sp(12.))
            .line_height(self.sp(18.))
            .font_weight(FontWeight::NORMAL)
            .text_color(self.theme.muted)
            .child(text.to_owned())
    }
}
impl Render for Desktop {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        // Retained parent focus includes toolbar controls reached by Tab.
        // Only entry/exit notifications rebuild the optional composer details.
        self.composer_active =
            window.is_window_active() && self.composer_focus.contains_focused(window, cx);
        self.prepare_message_motion(window);
        self.prepare_message_menu(window, cx);
        // One native frame deadline for the current eligible control only.
        // No background task survives this view; at most one queued frame can
        // drain after cancellation. Pending and displayed IDs must remain live.
        self.hover_hints.retire(|id| self.control_focus.live(id));
        if self.hover_hints.request_frame(Instant::now()) {
            window.request_animation_frame();
        }
        let incoming = !self.update_restart_pending && self.incoming_visible();
        if incoming && self.snapshot.call.as_ref().map(|c| c.scope) != self.last_incoming_scope {
            self.incoming_return_focus = window.focused(cx);
            self.last_incoming_scope = self.snapshot.call.as_ref().map(|c| c.scope);
        }
        self.control_focus.begin_frame();
        self.navigation.reveal.begin();
        self.navigation.hints.begin();
        self.navigation.prepare_history(self.snapshot.selected);
        self.theme = Theme::resolve(self.palette, self.appearance, cx.window_appearance());
        let t = self.theme;
        for input in self.content_ui.inputs() {
            input.update(cx, |i, _| {
                i.theme = t;
                i.text_scale = f32::from(self.scale) / 100.;
                i.clipboard_allowed = true;
                i.interactive = self.overlay == Some(Overlay::CreatePoll)
                    && !self.snapshot.content_panel.poll_state.pending()
                    && !incoming;
            });
        }
        self.content_ui
            .question
            .update(cx, |i, _| i.set_placeholder(self.l("Frage der Umfrage")));
        for input in &self.content_ui.options {
            input.update(cx, |i, _| i.set_placeholder(self.l("Antwortoption")));
        }
        for input in [
            &self.server,
            &self.search,
            &self.sidebar_filter,
            &self.composer,
            &self.conversation_name,
            &self.group_name,
            &self.upload_folder,
        ] {
            input.update(cx, |i, _| {
                i.theme = t;
                i.interactive = self.overlay.is_none() && !incoming;
                i.text_scale = f32::from(self.scale) / 100.;
            });
        }
        for input in self.room_ui.inputs() {
            input.update(cx, |i, _| {
                i.theme = t;
                i.text_scale = f32::from(self.scale) / 100.;
                i.clipboard_allowed = true;
                i.interactive = !incoming
                    && matches!(
                        self.overlay,
                        Some(Overlay::RoomInfo | Overlay::Participants)
                    );
            });
        }
        if self.overlay == Some(Overlay::Create) {
            self.room_ui.search.update(cx, |i, _| {
                i.interactive = !incoming && self.room_ui.create_direct
            });
        }
        for input in [&self.message_ui.search, &self.message_ui.time_input] {
            input.update(cx, |i, _| {
                i.theme = t;
                i.text_scale = f32::from(self.scale) / 100.;
                i.interactive = self.composer_available();
                i.clipboard_allowed = true;
            });
        }
        self.room_ui.password.update(cx, |i, _| {
            i.concealed = true;
            i.clipboard_allowed = false;
        });
        for (input, key) in [
            (&self.room_ui.name, "Name des Gesprächs"),
            (&self.room_ui.description, "Beschreibung"),
            (&self.room_ui.search, "Personen suchen …"),
            (&self.room_ui.csv, "E-Mail-Adressen, eine pro Zeile"),
            (&self.room_ui.avatar, "Emoji"),
            (&self.room_ui.password, "Neues Passwort"),
            (&self.message_ui.search, "Im Gespräch suchen …"),
        ] {
            input.update(cx, |i, _| i.set_placeholder(self.l(key)));
        }
        for (input, key) in [
            (&self.search, "Nachrichten, Gespräche und Personen suchen …"),
            (&self.sidebar_filter, "Gespräche filtern …"),
            (&self.conversation_name, "Name des Gesprächs"),
            (&self.group_name, "Name der Gruppe"),
        ] {
            let label = self.l(key);
            input.update(cx, |i, _| i.set_placeholder(label));
        }
        for input in [&self.server, &self.search, &self.sidebar_filter] {
            input.update(cx, |i, _| i.clipboard_allowed = true);
        }
        self.server.update(cx, |i, _| {
            i.interactive = self.overlay.is_none()
                && !incoming
                && self.snapshot.login.phase
                    == talk_core::ui::presentation::LoginPhase::EnterServer;
        });
        self.sidebar_filter.update(cx, |i, _| {
            i.theme.surface = t.subtle;
            i.borderless = true;
        });
        self.search.update(cx, |i, _| {
            i.theme.surface = t.subtle;
            i.interactive =
                !incoming && (self.overlay.is_none() || self.overlay == Some(Overlay::Search));
        });
        self.upload_folder.update(cx, |i, _| {
            i.interactive = !incoming
                && self.overlay == Some(Overlay::Settings)
                && self.settings_tab == preferences::SettingsTab::Files
                && self.account_preferences.as_ref().is_some_and(|p| {
                    p.state == talk_core::preferences::account::AccountPreferenceState::Ready
                });
            i.clipboard_allowed = true;
        });
        self.group_name.update(cx, |i, _| {
            i.interactive = !incoming && self.overlay == Some(Overlay::Organize);
        });
        self.conversation_name.update(cx, |i, _| {
            i.interactive = !incoming && self.overlay == Some(Overlay::Create);
            i.clipboard_allowed = true;
            i.borderless = true;
        });
        if let Some(row) = self.selected() {
            let placeholder = self.lf("Nachricht an {name}", &[("name", row.title.as_str())]);
            self.composer
                .update(cx, |i, _| i.set_placeholder(&placeholder));
        }
        let clipboard = self.snapshot.content == ContentState::Ready
            && self
                .selected()
                .is_some_and(|r| r.draft_clipboard == Decision::Allowed);
        let narrow = window.viewport_size().width < px(880.) || self.scale >= 150;
        // Reserve header, navigation, toolbar/result and a usable history slice.
        // The editor scrolls internally when large text cannot fit five lines.
        let reserved = px(window_chrome::HEIGHT + if narrow { 230. } else { 180. })
            + self.sp(if self.feedback.is_some() { 100. } else { 40. })
            + self.sp(72.);
        let max_rows = (f32::from((window.viewport_size().height - reserved).max(px(24.)))
            / f32::from(self.sp(24.)))
        .floor() as usize;
        self.composer.update(cx, |i, _| {
            i.interactive = self.composer_available();
            i.clipboard_allowed = clipboard;
            i.max_visible_rows = max_rows.clamp(1, 5);
        });
        if self.update_restart_pending {
            self.disable_update_inputs(cx);
        }
        for input in [
            &self.server,
            &self.search,
            &self.sidebar_filter,
            &self.composer,
            &self.conversation_name,
            &self.group_name,
            &self.upload_folder,
            &self.message_ui.search,
            &self.message_ui.time_input,
        ]
        .into_iter()
        .chain(self.room_ui.inputs().iter())
        .chain(self.content_ui.inputs())
        {
            input.update(cx, |input, _| input.configure_caret(self.reduced_motion));
        }
        // Text scaling is independent of spacing; scaling both exhausts small windows.
        window.set_rem_size(px(16.));
        self.background_inert = self.overlay.is_some() || incoming;
        let body = if self.update_restart_pending {
            use talk_core::preferences::updates::UpdateState;
            let status = match self.updates.state {
                UpdateState::Applying(_) => "Das Update wird installiert …",
                UpdateState::Installed(_) => {
                    "Das Update ist installiert. Der Neustart wird vorbereitet …"
                }
                UpdateState::Failed(_) | UpdateState::Stopped => {
                    "Das Update konnte nicht abgeschlossen werden."
                }
                _ => "Die App bereitet die Installation vor …",
            };
            self.reveal(
                "update-restart",
                div()
                    .size_full()
                    .flex()
                    .items_center()
                    .justify_center()
                    .p_6()
                    .child(self.state_note(
                        "download",
                        "Update installieren und neu starten",
                        status,
                    )),
            )
        } else if self.development && self.probe == Probe::Controls {
            self.control_probe(cx).into_any_element()
        } else if self.projection_blocked {
            div().size_full().flex().flex_col().items_center().justify_center().gap_4()
                .child(self.title(self.l("Ansicht vorübergehend nicht verfügbar")))
                .when(self.feedback.is_some(), |d| d.child(self.feedback_view(cx)))
                .child(self.hint(self.l("Ein konsistenter Kontostand ist erforderlich. Inhalte und normale Aktionen sind gesperrt.")))
                .when_some(self.snapshot.call.as_ref(),|d,call|d.child(self.button("leave-invalid-projection","Anruf verlassen",ButtonStyle::Danger,true,LocalAction::Intent(UiIntent::Legacy(UserIntent::Leave(call.scope))),cx)))
                .into_any_element()
        } else if self.prejoin.is_some() {
            self.prejoin_view(window, cx).into_any_element()
        } else if self.snapshot.call.as_ref().is_some_and(|c| {
            c.stage != CallStage::Incoming
                && !(matches!(c.stage, CallStage::Ended | CallStage::Failed)
                    && self.dismissed_terminal_call == Some(c.scope))
        }) {
            self.call_view(window, cx).into_any_element()
        } else if self.onboarding_open
            || (self.snapshot.accounts.as_slice().is_empty()
                && self.snapshot.saved_accounts.as_slice().is_empty())
        {
            self.onboarding(window, cx).into_any_element()
        } else {
            self.shell(window, cx).into_any_element()
        };
        self.background_inert = incoming || self.account_removal.confirming();
        let overlay = self
            .overlay
            .filter(|_| !self.update_restart_pending)
            .map(|overlay| {
                self.reveal(
                    ("dialog-reveal", overlay as usize),
                    self.overlay_view(window, cx),
                )
            });
        self.background_inert = false;
        let account_removal_dialog = (!incoming && !self.update_restart_pending)
            .then(|| self.account_removal_dialog(window, cx))
            .flatten();
        let incoming_card = incoming.then(|| self.incoming(cx));
        let message_menu = self.message_menu_popup(window, cx);
        let reveal = self.navigation.reveal.clone();
        let view = div()
            .relative()
            .on_children_prepainted(cx.processor(move |this, _, window, cx| {
                // Virtual list controls register during child prepaint, after
                // Render returns. Retiring them earlier loses row focus and
                // hover ownership on every keyboard-triggered render.
                reveal.finish();
                this.control_focus.finish_frame(
                    window,
                    if this.account_removal.confirming() {
                        &this.account_removal.focus
                    } else if this.overlay.is_some() || this.prejoin.is_some() {
                        &this.modal_focus
                    } else {
                        &this.root_focus
                    },
                );
                this.finish_message_menu_frame(window, cx);
                if this.hover_hints.retire(|id| this.control_focus.live(id)) {
                    cx.notify();
                }
                if this
                    .hovered_control
                    .as_ref()
                    .is_some_and(|id| !this.control_focus.live(id))
                {
                    this.hovered_control = None;
                }
            }))
            .size_full()
            .flex()
            .flex_col()
            .font_family("Google Sans")
            .text_size(self.sp(14.))
            .line_height(self.sp(22.))
            .bg(t.chrome())
            .text_color(t.text)
            .track_focus(&self.root_focus)
            .tab_group()
            .capture_key_down(cx.listener(|this, event: &KeyDownEvent, window, cx| {
                let composing = this.input_composing(cx);
                if event.keystroke.key != "f7" {
                    this.hover_hints.cancel();
                    if !composing {
                        // Includes activation, menu Home/End and shortcuts that
                        // open dialogs. Focus transfer preserves this modality.
                        this.keyboard_navigation = true;
                    }
                }
                if !composing && this.message_menu_key(event, window, cx) {
                    cx.stop_propagation();
                    cx.notify();
                    return;
                }
                cx.notify();
            }))
            .on_key_down(cx.listener(Self::key))
            .child(self.pointer_modality(cx))
            .child(self.window_chrome(window, cx))
            .child(
                div()
                    .relative()
                    .flex_1()
                    .min_h_0()
                    .child(body)
                    .when_some(overlay, |d, overlay| d.child(overlay))
                    .when_some(account_removal_dialog, |d, dialog| d.child(dialog))
                    .when_some(incoming_card, |d, card| d.child(card))
                    .when_some(message_menu, |d, menu| d.child(menu)),
            )
            .children(self.window_resize_edges(window));
        if let Some(scope) = self.pending_prejoin_return.take()
            && self.overlay.is_none()
            && self.prejoin.is_none()
            && self.selected_call() == Some(CallEntry::Conference(scope))
            && let Some(focus) = self.control_focus.current(&"call".into())
        {
            focus.focus(window);
            self.navigation.reveal.request();
        }
        if let Some(open) = self.pending_navigation_focus.take()
            && let Some(focus) = self.control_focus(
                if open {
                    "hide-navigation"
                } else {
                    "show-navigation"
                },
                cx,
            )
        {
            focus.focus(window);
            self.navigation.reveal.request();
            let reveal = self.navigation.reveal.clone();
            window.on_next_frame(move |window, _| {
                reveal.request();
                window.refresh();
            });
        }
        if let Some(id) = self.pending_device_focus.take()
            && self.devices_open()
            && let Some(focus) = self.control_focus.current(&id)
        {
            focus.focus(window);
            self.navigation.reveal.request();
        }
        if self.account_removal.focus_cancel && !incoming && !self.update_restart_pending {
            self.account_removal.focus_cancel = false;
            if let Some(focus) = self.control_focus.current(&"cancel-account-removal".into()) {
                focus.focus(window);
            }
        }
        if self.pending_modal_focus
            && !self.account_removal.confirming()
            && !incoming
            && !self.update_restart_pending
        {
            self.pending_modal_focus = false;
            let id: Option<gpui::ElementId> = match self.overlay {
                Some(Overlay::Settings)
                    if self.settings_tab == preferences::SettingsTab::Appearance =>
                {
                    Some(
                        (
                            "appearance-card",
                            match self.appearance {
                                Appearance::System => 0usize,
                                Appearance::Light => 1,
                                Appearance::Dark => 2,
                            },
                        )
                            .into(),
                    )
                }
                Some(Overlay::Settings | Overlay::Accounts) => Some("close-settings".into()),
                Some(Overlay::Search) => {
                    self.search.focus_handle(cx).focus(window);
                    None
                }
                Some(Overlay::CreatePoll) => {
                    self.content_ui.question.focus_handle(cx).focus(window);
                    None
                }
                Some(_) => Some("close-overlay".into()),
                None if self.prejoin.is_some() => Some("prejoin-back".into()),
                None => None,
            };
            if let Some(id) = id
                && let Some(focus) = self.control_focus(id, cx)
            {
                focus.focus(window);
            }
            self.navigation.reveal.request();
            // The first frame registers the new focus ancestry. Request again
            // after it exists; otherwise the first scroll owner cannot yet
            // recognize its focused descendant and drops the request.
            let reveal = self.navigation.reveal.clone();
            window.on_next_frame(move |window, _| {
                reveal.request();
                window.refresh();
            });
        }
        if self.pending_feedback_focus && !self.update_restart_pending {
            self.pending_feedback_focus = false;
            if !incoming
                && self.overlay.is_none()
                && let Some(focus) = self.control_focus("dismiss-feedback", cx)
            {
                focus.focus(window);
                let reveal = self.navigation.reveal.clone();
                reveal.request();
                window.on_next_frame(move |window, _| {
                    reveal.request();
                    window.refresh();
                });
            }
        }
        self.finish_onboarding_focus(window, cx);
        self.finish_search_focus(window, cx);
        view
    }
}

// The call-chat dialog owns the same editor as the docked call chat. Other
// dialogs and incoming-call priority always exclude editor input and dispatch.
fn composer_allowed(overlay: Option<Overlay>, prejoin: bool, incoming: bool) -> bool {
    !prejoin && !incoming && matches!(overlay, None | Some(Overlay::CallChat))
}
fn docked_call_chat(wide: bool, open: bool, overlay: Option<Overlay>) -> bool {
    wide && open && overlay != Some(Overlay::CallChat)
}

#[cfg(test)]
mod interaction_tests {
    use super::{Overlay, composer_allowed, docked_call_chat};
    #[test]
    fn call_chat_retains_one_editor_owner_through_resize_and_modal_exclusion() {
        for wide in [false, true] {
            assert!(!docked_call_chat(wide, true, Some(Overlay::CallChat)));
            assert!(composer_allowed(Some(Overlay::CallChat), false, false));
        }
        assert!(docked_call_chat(true, true, None));
        assert!(!docked_call_chat(false, true, None));
        for modal in [Overlay::Settings, Overlay::Search, Overlay::Accounts] {
            assert!(!composer_allowed(Some(modal), false, false));
        }
        assert!(!composer_allowed(Some(Overlay::CallChat), false, true));
        assert!(!composer_allowed(None, true, false));
    }
}
