//! Explicit development scenarios only. No server, credential or media operation occurs.
mod content;
mod dialog;
pub(super) use content::images as content_images;
use talk_core::ui::presentation::{
    AccountView, CallStage, CallStreamView, CallTileKind, CallView, Completion, ContentState,
    ConversationKind, ConversationRef, ConversationView, DeliveryState, FeatureSupport, FileView,
    LoginFailure, LoginPhase, LoginView, MediaState, NativeDesktopPort, NativeStoreState,
    ParticipantView, Presence, PresentationSnapshot, RequestId, RequiredGate, ScreenShareState,
    StorageStatus, SubmissionResult, UiIntent,
};
use talk_core::ui::{
    AccountId, AccountScope, CallGeneration, CallScope, ConversationId, Decision, HostId, Page,
    RuntimeGeneration, SubmitError, Text, UnavailableReason, UserIntent,
};

/// A deterministic device inventory for the call-device scenario. It contains
/// no physical device labels or capture handles.
pub(super) fn call_devices() -> talk_core::preferences::devices::DeviceSnapshot {
    use talk_core::preferences::{Revision, RuntimeState, devices::*};
    let mut devices = Page::empty();
    for (id, label, kind) in [
        (
            "fixture-mic-studio",
            "Studio microphone",
            DeviceKind::Microphone,
        ),
        (
            "fixture-mic-laptop",
            "Laptop microphone",
            DeviceKind::Microphone,
        ),
        ("fixture-camera", "Laptop camera", DeviceKind::Camera),
    ] {
        if let Ok(id) = DeviceId::new(id) {
            let _ = devices.try_push(DeviceInfo {
                id,
                label: text(label),
                kind,
                is_default: false,
            });
        }
    }
    DeviceSnapshot {
        runtime: RuntimeState::Running,
        session: None,
        inventory_revision: Revision(1),
        devices,
        microphone_permission: PermissionState::Unknown,
        camera_permission: PermissionState::Unknown,
        microphone: SelectionState::Unknown,
        speaker: SelectionState::Unknown,
        camera: SelectionState::Unknown,
        microphone_level: None,
        speaker_level: None,
        microphone_preview: PreviewState::Stopped,
        camera_preview: PreviewState::Stopped,
        speaker_test: PreviewState::Stopped,
    }
}

pub const SCENARIOS: &[&str] = &[
    "server",
    "validating",
    "browser",
    "storage",
    "ready",
    "expired",
    "login-network",
    "login-admission",
    "login-server-denied",
    "login-tls",
    "login-denied",
    "login-browser-error",
    "login-storage-error",
    "login-account-store-busy",
    "login-account-store-device-changed",
    "login-account-store-error",
    "storage-locked",
    "storage-denied",
    "storage-saving",
    "storage-cleanup",
    "storage-recovery",
    "chat",
    "chat-grouped",
    "chat-busy",
    "chat-send-pending",
    "chat-reactions",
    "chat-reactions-pending",
    "chat-reactions-denied",
    "reaction-authors",
    "loading-conversations",
    "chat-server-denied",
    "chat-server-unavailable",
    "accounts-server-denied",
    "chat-ux",
    "chat-presence",
    "people-card",
    "chat-typing",
    "chat-typing-two",
    "chat-typing-many",
    "status-picker",
    "status-message",
    "chat-ux-call",
    "chat-edit",
    "chat-message-tools",
    "chat-reminder-time",
    "chat-reminder-menu",
    "account-reminders",
    "account-reminders-partial",
    "federation-invitations",
    "federation-pending",
    "chat-schedule-time",
    "chat-schedule-edit",
    "chat-translation",
    "chat-summary-offer",
    "chat-summary-pending",
    "chat-summary-result",
    "lobby-settings",
    "room-avatar",
    "breakout-configure",
    "breakout-create",
    "breakout-free",
    "breakout-assigned",
    "breakout-help",
    "thread-error",
    "thread-list",
    "thread-options",
    "thread-offline",
    "thread-paged",
    "thread-compose",
    "private-reply",
    "sidebar-unread",
    "unread-divider",
    "unread-jump",
    "chat-content",
    "chat-cloud-files",
    "chat-cloud-files-offline",
    "chat-polls",
    "poll-dialog",
    "poll-drafts",
    "poll-details",
    "poll-settings",
    "chat-voice",
    "voice-recording",
    "voice-review",
    "chat-content-progress",
    "chat-content-loading",
    "chat-content-failed",
    "chat-attachments",
    "chat-upload",
    "chat-upload-preview",
    "chat-upload-uncertain",
    "chat-motion",
    "history",
    "history-pagination",
    "history-busy",
    "history-pending",
    "history-uncertain",
    "settings",
    "details",
    "details-ready",
    "participants",
    "files",
    "thread",
    "search",
    "search-navigation",
    "account-search",
    "reference-preview",
    "thread-reference",
    "search-filter",
    "sidebar-room-menu",
    "create34",
    "create35",
    "discover",
    "discover-joining",
    "uncertain",
    "pending",
    "incoming",
    "outgoing",
    "prejoin",
    "call",
    "direct-call",
    "voice",
    "gate",
    "gate-mic",
    "gate-camera",
    "gate-recording",
    "gate-recording-direct",
    "gate-lobby",
    "connecting",
    "reconnecting",
    "call-ended",
    "call-error",
    "call-error-unknown",
    "call-controls",
    "call-gallery",
    "call-participants-raised",
    "call-dock",
    "call-devices",
    "call-speaker",
    "call-screen-stage",
    "call-recording",
    "call-recording-confirm",
    "call-recording-failed",
    "call-end-confirm",
    "call-reactions",
    "call-active",
    "call-requested",
    "sharing",
    "call-denied",
    "room",
    "voice-room",
    "prejoin-voice",
    "offline",
    "connection-reconnecting",
    "offline-with-queue",
    "startup-from-cache",
    "offline-empty",
    "loading",
    "loading-retained",
    "denied",
    "error",
    "empty",
    "busy",
    "long-labels",
    "clipboard-denied",
    "controls",
    "focus-rooms",
    "focus-files",
    "accounts",
    "focus-accounts",
];
fn text<const N: usize>(s: &str) -> Text<N> {
    Text::new(s).expect("reviewed bounded UI fixture")
}
fn page<T, const N: usize>(items: impl IntoIterator<Item = T>) -> Page<T, N> {
    let mut p = Page::empty();
    for item in items {
        p.try_push(item).expect("reviewed fixture count");
    }
    p
}
pub fn scope() -> AccountScope {
    AccountScope {
        account: AccountId::from_local(1),
        runtime: RuntimeGeneration::from_local(1),
    }
}
pub fn context(id: u128) -> ConversationRef {
    ConversationRef {
        account: scope(),
        conversation: ConversationId {
            account: scope().account,
            host: HostId::from_local(1),
            local: id,
        },
    }
}
fn call_scope(id: u128) -> CallScope {
    CallScope {
        account: scope(),
        conversation: context(id).conversation,
        call: CallGeneration::from_local(1),
    }
}
fn participant(id: u128, name: &str, initials: &str, me: bool) -> ParticipantView {
    ParticipantView {
        local_id: id,
        name: text(name),
        initials: text(initials),
        role_label: text(if me { "Du" } else { "Teilnehmend" }),
        presence: Presence::Available,
        self_user: me,
        raised_hand: false,
        speaking: false,
        call_can_mute: false,
        call_microphone: None,
        call_camera: None,
    }
}
fn participants() -> [ParticipantView; 4] {
    [
        participant(1, "Mara Fischer", "MF", false),
        participant(2, "Jonas Weber", "JW", false),
        participant(3, "Alex Sommer", "AS", true),
        participant(4, "Lea Neumann", "LN", false),
    ]
}
fn file() -> FileView {
    FileView {
        local_id: 1,
        name: text("Navigation-v3.fig"),
        detail: text("Figma-Datei · 2,4 MB"),
        download: Decision::Unknown,
    }
}
fn message(
    id: u128,
    author: &str,
    initials: &str,
    time: &str,
    body: &str,
    own: bool,
) -> talk_core::ui::presentation::MessageView {
    talk_core::ui::presentation::MessageView {
        local_id: id,
        author: text(author),
        initials: text(initials),
        time_label: text(time),
        body: text(body),
        own,
        delivery: DeliveryState::Received,
        attachments: Page::empty(),
        reply_count: 0,
        metadata: Default::default(),
    }
}
pub fn snapshot(scenario: Option<&str>) -> PresentationSnapshot {
    if matches!(
        scenario,
        Some("connection-reconnecting" | "offline-with-queue" | "startup-from-cache")
    ) {
        let mut state = snapshot(Some("offline"));
        if let Some(context) = state.selected {
            let mut panel = talk_core::ui::messaging::MessagePanel::new(context);
            panel.queue_available = true;
            panel.queued_actions = u32::from(scenario == Some("offline-with-queue"));
            state.message_panel = Some(panel);
        }
        if scenario == Some("startup-from-cache") {
            state.content = ContentState::Loading;
            state.conversation_list.content = ContentState::Loading;
        }
        return state;
    }

    if scenario == Some("private-reply") {
        let mut state = snapshot(Some("chat-ux"));
        let direct = context(3);
        populate(&mut state, direct);
        state.message_panel = Some(talk_core::ui::messaging::MessagePanel::new(direct));
        return state;
    }
    if scenario == Some("thread-paged") {
        let mut state = snapshot(Some("thread"));
        if let Some(panel) = state.message_panel.as_mut() {
            panel.thread_older = Some(40);
            panel.thread_has_newer = true;
        }
        return state;
    }
    if matches!(scenario, Some("unread-divider" | "unread-jump")) {
        let mut state = snapshot(Some("chat-grouped"));
        let marker = if scenario == Some("unread-divider") {
            3
        } else {
            1
        };
        state.conversations = page(state.conversations.as_slice().iter().cloned().map(
            |mut room| {
                if Some(room.context) == state.selected {
                    room.unread = 3;
                    room.last_read_message = Some(marker);
                    room.unread_direct_mention = Some(scenario == Some("unread-divider"));
                }
                room
            },
        ));
        if scenario == Some("unread-divider") {
            state.messages = page(state.messages.as_slice().iter().cloned().map(|mut row| {
                if row.local_id == 4 {
                    row.metadata.unread_after = Some(marker);
                }
                row
            }));
        }
        return state;
    }
    if scenario == Some("reaction-authors") {
        let mut state = snapshot(Some("chat-reactions"));
        if let Some(panel) = state.message_panel.as_mut() {
            panel.reaction_authors = page([talk_core::ui::messaging::ReactionAuthors {
                message: 4,
                emoji: text("👍"),
                authors: page([
                    text("Mara Fischer"),
                    text("Jonas Weber"),
                    text("Alex Sommer"),
                ]),
            }]);
        }
        return state;
    }
    if scenario == Some("thread-compose") {
        let mut state = snapshot(Some("chat-edit"));
        state.messages = page(state.messages.as_slice().iter().cloned().map(|mut row| {
            row.metadata.rich = Page::empty();
            row.attachments = Page::empty();
            row.reply_count = 0;
            row
        }));
        return state;
    }
    if scenario == Some("sidebar-unread") {
        let mut state = snapshot(Some("chat-ux"));
        state.messages = page(state.messages.as_slice().iter().cloned().map(|mut row| {
            row.metadata.rich = Page::empty();
            row.attachments = Page::empty();
            row.reply_count = 0;
            row
        }));
        return state;
    }
    if scenario == Some("thread-list") {
        let mut state = snapshot(Some("thread"));
        if let Some(context) = state.selected {
            state.conversations = page(state.conversations.as_slice().iter().cloned().map(
                |mut room| {
                    if room.context == context {
                        room.unread = 2;
                        room.last_read_message = Some(1);
                    }
                    room
                },
            ));
        }
        return state;
    }
    if scenario == Some("thread-options") {
        return snapshot(Some("thread"));
    }
    if scenario == Some("thread-offline") {
        let mut state = snapshot(Some("thread"));
        state.content = ContentState::Offline;
        if let Some(panel) = state.message_panel.as_mut() {
            panel.queue_available = true;
            panel.features = Page::empty();
            if let Some(details) = panel.thread_details.as_mut() {
                details.rename = Decision::Unknown;
            }
        }
        return state;
    }
    if matches!(
        scenario,
        Some("chat-typing" | "chat-typing-two" | "chat-typing-many")
    ) {
        let mut state = snapshot(Some("chat-grouped"));
        if let Some(panel) = state.message_panel.as_mut() {
            let count = match scenario {
                Some("chat-typing") => 1,
                Some("chat-typing-two") => 2,
                _ => 3,
            };
            panel.typing = page(
                [
                    ("Mara", "fixture-mara"),
                    ("Jonas", "fixture-jonas"),
                    ("Lea", "fixture-lea"),
                ]
                .into_iter()
                .take(count)
                .map(|(name, key)| talk_core::ui::messaging::TypingPerson {
                    name: text(name),
                    author_key: text(key),
                }),
            );
        }
        return state;
    }
    if scenario == Some("people-card") {
        let mut state = snapshot(Some("chat-grouped"));
        dialog::configure(&mut state);
        let context = state.selected.expect("fixture room");
        let panel = state.conversation_panel.as_mut().expect("fixture details");
        panel.members_loaded = true;
        panel.members = page([talk_core::ui::conversation::Member {
            id: talk_core::ui::conversation::PersonRef { context, local: 71 },
            name: text("Mara Fischer"),
            author_key: Some(text("5:users:mara")),
            presence: Presence::Away,
            status_icon: Some(text("☕")),
            status_message: Some(text("Zurück nach der Kaffeepause")),
            role: talk_core::ui::conversation::MemberRole::Moderator,
            source: talk_core::ui::conversation::DirectorySource::User,
            permissions: Some(511),
            self_user: false,
        }]);
        if let Some(panel) = state.message_panel.as_mut() {
            let _ = panel.features.try_push((
                talk_core::ui::messaging::MessageFeature::Mentions,
                talk_core::ui::conversation::Availability {
                    implemented: true,
                    server: talk_core::ui::presentation::FeatureSupport::Supported,
                    allowed: Decision::Allowed,
                },
            ));
        }
        return state;
    }
    if matches!(scenario, Some("status-picker" | "status-message")) {
        let mut state = snapshot(Some("chat-presence"));
        state.accounts = page(
            state
                .accounts
                .as_slice()
                .iter()
                .cloned()
                .map(|mut account| {
                    account.connection =
                        talk_core::ui::presentation::AccountConnectionState::Connected;
                    account
                }),
        );
        return state;
    }
    if scenario == Some("sidebar-room-menu") {
        return snapshot(Some("details-ready"));
    }
    if scenario == Some("discover-joining") {
        let mut state = snapshot(Some("chat"));
        let context = context(7);
        let mut room = state.conversations.as_slice()[0].clone();
        room.context = context;
        room.navigation_key = None;
        room.title = text("Designrunde");
        room.kind = ConversationKind::Unknown;
        room.send = Decision::Unknown;
        room.attach = Decision::Unknown;
        room.call = Decision::Unknown;
        room.call_scope = None;
        room.draft_clipboard = Decision::Unknown;
        room.favorite = None;
        room.archive = Decision::Unknown;
        room.participants = None;
        room.unread = 0;
        room.unread_mention = None;
        room.notification_muted = None;
        room.organization.category = talk_core::ui::presentation::ConversationCategory::Unknown;
        room.organization.favorites = FeatureSupport::Unknown;
        room.organization.favorite_change = Decision::Unknown;
        room.organization.classified = None;
        state.conversations =
            page(std::iter::once(room).chain(state.conversations.as_slice().iter().cloned()));
        state.selected = Some(context);
        state.content = ContentState::Loading;
        state.messages = Page::empty();
        state.files = Page::empty();
        state.participants = Page::empty();
        state.conversation_panel = None;
        state.message_panel = None;
        state.content_panel = Default::default();
        state.discover_panel = Some(talk_core::ui::discover::Panel {
            account: context.account,
            request: RequestId(1),
            query: text(""),
            support: FeatureSupport::Supported,
            state: talk_core::ui::conversation::RemoteState::Saving(RequestId(1)),
            rooms: page([talk_core::ui::discover::Room {
                context,
                name: text("Designrunde"),
                description: None,
                participants: None,
                join: Decision::Allowed,
            }]),
            loaded: true,
            joining: Some(context),
            uncertain: false,
        });
        return state;
    }
    if scenario == Some("thread-reference") {
        let mut state = snapshot(Some("thread"));
        let context = state.selected.expect("fictional room");
        if let Some(panel) = state.message_panel.as_mut() {
            panel.thread_messages = page(panel.thread_messages.as_slice().iter().cloned().map(
                |mut reply| {
                    if reply.local_id == 40 {
                        reply.body = text("Der Entwurf im Thread: https://example.test/studio");
                    }
                    reply
                },
            ));
        }
        state.reference_previews = page([talk_core::ui::reference::Preview {
            context,
            message: 40,
            ordinal: 0,
            title: text("Studiokonzept im Thread"),
            description: Some(text("Ein fiktiver Link aus der Diskussion.")),
        }]);
        return state;
    }
    if scenario == Some("reference-preview") {
        let mut state = snapshot(Some("chat"));
        let context = state.selected.expect("fictional room");
        let mut first = state.messages.as_slice()[0].clone();
        first.body = text("Hier ist der Entwurf: https://example.test/studio");
        first.attachments = Page::empty();
        first.metadata.rich = Page::empty();
        state.messages =
            page(std::iter::once(first).chain(state.messages.as_slice().iter().skip(1).cloned()));
        state.reference_previews = page([talk_core::ui::reference::Preview {
            context,
            message: 1,
            ordinal: 0,
            title: text("Ein ruhiger Ort für neue Ideen"),
            description: Some(text("Ein fiktiver Einblick in das neue Studiokonzept.")),
        }]);
        state.content_panel.account = Some(context.account);
        state.content_panel.context = Some(context);
        state.content_panel.assets = page([talk_core::ui::content::AssetView {
            target: talk_core::ui::content::AssetTarget::Reference {
                context,
                message: 1,
                ordinal: 0,
            },
            state: talk_core::ui::content::AssetState::Ready { key: 42 },
            emoji: None,
            color: None,
            document_pages: None,
        }]);
        return state;
    }
    if matches!(
        scenario,
        Some("federation-invitations" | "federation-pending")
    ) {
        use talk_core::ui::presentation::{
            FeatureSupport, FederationInvitationView, FederationInvitationsView,
        };
        let mut state = snapshot(Some("chat-ux"));
        if let Some(scope) = state.active_account {
            state.federation_invitations = Some(FederationInvitationsView {
                scope,
                support: FeatureSupport::Supported,
                pending: scenario == Some("federation-pending"),
                failed: false,
                invitations: page([
                    FederationInvitationView {
                        id: 101,
                        room: text("Community planning"),
                        inviter: text("Alex Morgan"),
                    },
                    FederationInvitationView {
                        id: 102,
                        room: text("Design exchange"),
                        inviter: text("Sam Rivera"),
                    },
                ]),
            });
        }
        return state;
    }
    if matches!(
        scenario,
        Some("chat-summary-offer" | "chat-summary-pending" | "chat-summary-result")
    ) {
        use talk_core::ui::{
            conversation::Availability,
            messaging::{MessageFeature, Summary},
            presentation::FeatureSupport,
        };
        let mut state = snapshot(Some("chat-ux"));
        if let Some(panel) = state.message_panel.as_mut() {
            let _ = panel.features.try_push((
                MessageFeature::Summary,
                Availability {
                    implemented: true,
                    server: FeatureSupport::Supported,
                    allowed: Decision::Allowed,
                },
            ));
            panel.summary = Some(match scenario {
                Some("chat-summary-pending") => Summary::Pending {
                    parts: Page::empty(),
                },
                Some("chat-summary-result") => Summary::Ready {
                    parts: page([text(
                        "The team agreed to share the draft on Friday. Mara will collect feedback before the next meeting.",
                    )]),
                    partial: false,
                },
                _ => Summary::Offer {
                    from: 12,
                    unread: 120,
                },
            });
        }
        return state;
    }
    if scenario == Some("chat-translation") {
        use talk_core::ui::{
            conversation::Availability,
            messaging::{MessageFeature, Translation},
            presentation::FeatureSupport,
        };
        let mut state = snapshot(Some("chat-ux"));
        if let Some(message) = state
            .messages
            .as_slice()
            .iter()
            .find(|message| {
                !message.metadata.system
                    && !message.metadata.deleted
                    && matches!(
                        message.delivery,
                        DeliveryState::Received | DeliveryState::ServerAccepted
                    )
                    && !message.body.as_str().trim().is_empty()
                    && message.metadata.rich.as_slice().is_empty()
            })
            .cloned()
            && let Some(panel) = state.message_panel.as_mut()
        {
            let _ = panel.features.try_push((
                MessageFeature::Translation,
                Availability {
                    implemented: true,
                    server: FeatureSupport::Supported,
                    allowed: Decision::Allowed,
                },
            ));
            panel.translation_targets = Some(page([text("de"), text("en"), text("es")]));
            panel.translations = page(["de", "en", "es"].map(|language| Translation {
                message: message.local_id,
                source: message.body.clone(),
                language: text(language),
                text: text("Fictional translated message for the team."),
            }));
        }
        return state;
    }
    if matches!(
        scenario,
        Some("account-reminders" | "account-reminders-partial")
    ) {
        use talk_core::ui::presentation::{
            AccountReminderView, AccountRemindersView, FeatureSupport,
        };
        let mut state = snapshot(Some("chat-ux"));
        let reminders = state
            .conversations
            .as_slice()
            .iter()
            .take(3)
            .enumerate()
            .map(|(index, room)| AccountReminderView {
                context: room.context,
                message: 45 + index as u128,
                preview: Some(text(match index {
                    0 => "Die neue Navigation ist bereit für einen letzten Blick.",
                    1 => "Den Entwurf besprechen wir morgen.",
                    _ => "Ich schicke dir die Notizen später.",
                })),
                timestamp: if index == 0 {
                    1_800_003_600
                } else {
                    1_800_090_000
                },
            })
            .collect::<Vec<_>>();
        state.account_reminders = Some(AccountRemindersView {
            scope: scope(),
            support: FeatureSupport::Supported,
            state: ContentState::Ready,
            complete: scenario != Some("account-reminders-partial"),
            reminders: page(reminders),
        });
        return state;
    }
    if matches!(
        scenario,
        Some("chat-reactions" | "chat-reactions-pending" | "chat-reactions-denied")
    ) {
        let mut state = snapshot(Some("chat-send-pending"));
        if scenario == Some("chat-reactions-denied")
            && let Some(panel) = state.message_panel.as_mut()
        {
            panel.features = page(panel.features.as_slice().iter().copied().map(
                |(feature, mut available)| {
                    if feature == talk_core::ui::messaging::MessageFeature::React {
                        available.allowed = Decision::Denied;
                    }
                    (feature, available)
                },
            ));
        }
        return state;
    }
    if scenario == Some("chat-send-pending") {
        let mut state = snapshot(Some("chat-grouped"));
        state.messages = page(state.messages.as_slice().iter().cloned().map(|mut message| {
            if message.local_id == 5 { message.metadata.edited = true; }
            if message.local_id == 6 {
                message.body = text("Eine längere fiktive Nachricht prüft den Umbruch und den gleichbleibenden Abstand der Bestätigung. Auch mit mehreren Textzeilen bleibt das Häkchen in derselben Spalte wie bei der vorherigen Nachricht.");
            }
            message
        }));
        return state;
    }
    if matches!(
        scenario,
        Some(
            "chat-message-tools"
                | "chat-reminder-time"
                | "chat-reminder-menu"
                | "chat-schedule-time"
                | "chat-schedule-edit"
        )
    ) {
        let mut state = snapshot(Some("chat-ux"));
        let mut rows = state.messages.as_slice().to_vec();
        let mut location = message(
            45,
            "Mara Fischer",
            "MF",
            "10:15",
            "Hier treffen wir uns.",
            false,
        );
        location.metadata.rich = page([talk_core::ui::messaging::RichObject {
            id: Some(text("geo:52.5450511,13.3741463")),
            mime_type: None,
            size: None,
            placeholder: text("location"),
            kind: text("geo-location"),
            name: text("Fiktiver Treffpunkt"),
            link: None,
        }]);
        rows.push(location);
        state.messages = page(rows);
        if let Some(panel) = state.message_panel.as_mut() {
            use talk_core::ui::{
                conversation::Availability,
                messaging::{MessageFeature, Reminder, Scheduled},
                presentation::FeatureSupport,
            };
            for feature in [
                MessageFeature::Reminders,
                MessageFeature::UpcomingReminders,
                MessageFeature::Schedules,
            ] {
                let _ = panel.features.try_push((
                    feature,
                    Availability {
                        implemented: true,
                        server: FeatureSupport::Supported,
                        allowed: Decision::Allowed,
                    },
                ));
            }
            panel.reminders = page([Reminder {
                message: 45,
                timestamp: 1_800_003_600,
            }]);
            panel.scheduled = page([
                Scheduled {
                    id: 41,
                    body: text("Fiktive Nachricht für morgen"),
                    timestamp: 1_800_090_000,
                    failed_original_timestamp: None,
                    editable: true,
                    silent: false,
                },
                Scheduled {
                    id: 42,
                    body: text("Fiktive Nachricht erneut prüfen"),
                    timestamp: 0,
                    failed_original_timestamp: Some(1_800_003_600),
                    editable: true,
                    silent: false,
                },
            ]);
        }
        return state;
    }
    if scenario == Some("chat-busy") {
        return snapshot(Some("chat-grouped"));
    }
    if scenario == Some("loading-conversations") {
        let mut state = snapshot(Some("chat"));
        state.conversations = Page::empty();
        state.selected = None;
        state.messages = Page::empty();
        state.files = Page::empty();
        state.participants = Page::empty();
        state.conversation_panel = None;
        state.message_panel = None;
        state.content_panel = Default::default();
        state.conversation_list.content = ContentState::Loading;
        state.content = ContentState::Loading;
        return state;
    }
    if scenario == Some("chat-grouped") {
        let mut state = snapshot(Some("chat-ux"));
        let base = 1_800_003_600;
        state.messages = page(
            [
                (
                    1,
                    "Mara Fischer",
                    "MF",
                    "Die neue Navigation ist bereit.",
                    false,
                    0,
                ),
                (
                    2,
                    "Mara Fischer",
                    "MF",
                    "Die Gruppen stehen jetzt direkt unter Gespräche.",
                    false,
                    45,
                ),
                (
                    3,
                    "Mara Fischer",
                    "MF",
                    "Und die Zeilen sind wieder kompakt.",
                    false,
                    95,
                ),
                (
                    4,
                    "Alex Sommer",
                    "AS",
                    "Passt. Ich prüfe gerade den Eingabebereich.",
                    true,
                    110,
                ),
                (
                    5,
                    "Alex Sommer",
                    "AS",
                    "Der Fokus hebt jetzt nur den vorhandenen Rand hervor.",
                    true,
                    145,
                ),
                (
                    6,
                    "Alex Sommer",
                    "AS",
                    "Nach einer längeren Pause beginnt eine neue Gruppe.",
                    true,
                    300,
                ),
            ]
            .into_iter()
            .map(|(id, author, initials, body, own, offset)| {
                let mut message = message(id, author, initials, "10:00", body, own);
                message.metadata.author_key =
                    Some(text(if own { "5:users:alex" } else { "5:users:mara" }));
                message.metadata.timestamp = base + offset;
                if id == 4 {
                    message.metadata.reactions = page([
                        talk_core::ui::messaging::Reaction {
                            emoji: text("👍"),
                            count: 3,
                            own: true,
                        },
                        talk_core::ui::messaging::Reaction {
                            emoji: text("❤️"),
                            count: 1,
                            own: false,
                        },
                    ]);
                }
                message
            }),
        );
        if let Some(panel) = state.message_panel.as_mut() {
            panel.common_read = Some(4);
            for feature in [
                talk_core::ui::messaging::MessageFeature::ReadReceipts,
                talk_core::ui::messaging::MessageFeature::React,
            ] {
                let _ = panel.features.try_push((
                    feature,
                    talk_core::ui::conversation::Availability {
                        implemented: true,
                        server: talk_core::ui::presentation::FeatureSupport::Supported,
                        allowed: Decision::Allowed,
                    },
                ));
            }
        }
        return state;
    }
    if scenario == Some("thread") {
        let mut state = snapshot(Some("chat-edit"));
        if let Some(context) = state.selected {
            state.threads.account = Some(context.account);
            state.threads.support = talk_core::ui::presentation::FeatureSupport::Supported;
            state.threads.content = ContentState::Ready;
            state.threads.entries = page([talk_core::ui::presentation::ThreadView {
                context,
                root: 1,
                title: text("Design review"),
                replies: 2,
                last_message: Some(41),
            }]);
        }
        state.messages = page(
            state
                .messages
                .as_slice()
                .iter()
                .cloned()
                .map(|mut message| {
                    message.reply_count = if message.local_id == 1 { 2 } else { 0 };
                    if message.local_id == 1 {
                        message.metadata.thread = Some(1);
                        message.metadata.rich = Page::empty();
                        message.attachments = Page::empty();
                    }
                    message
                }),
        );
        let root = Some(1);
        if let (Some(root), Some(panel)) = (root, state.message_panel.as_mut()) {
            panel.thread_root = Some(root);
            panel.thread_details = Some(talk_core::ui::messaging::ThreadDetails {
                root,
                title: text("Design review"),
                notification_level: Some(0),
                rename: Decision::Allowed,
            });
            for feature in [
                talk_core::ui::messaging::MessageFeature::React,
                talk_core::ui::messaging::MessageFeature::Mentions,
            ] {
                let _ = panel.features.try_push((
                    feature,
                    talk_core::ui::conversation::Availability {
                        implemented: true,
                        server: talk_core::ui::presentation::FeatureSupport::Supported,
                        allowed: Decision::Allowed,
                    },
                ));
            }
            let mut reply = message(
                40,
                "Mara Fischer",
                "MF",
                "10:00",
                "That works for me.",
                false,
            );
            reply.metadata.thread = Some(root);
            reply.metadata.author_key = Some(text("5:users:mara"));
            reply.metadata.replyable = true;
            reply.metadata.timestamp = 1_770_000_000;
            let _ = reply
                .metadata
                .reactions
                .try_push(talk_core::ui::messaging::Reaction {
                    emoji: text("👍"),
                    count: 2,
                    own: true,
                });
            let mut second = message(
                41,
                "Alex Sommer",
                "AS",
                "10:02",
                "I'll update the final details.",
                true,
            );
            second.metadata.thread = Some(root);
            second.metadata.author_key = Some(text("5:users:alex"));
            second.metadata.replyable = true;
            second.metadata.edit = Decision::Allowed;
            second.metadata.timestamp = 1_770_000_120;
            panel.thread_messages = page([reply, second]);
        }
        return state;
    }
    if matches!(scenario, Some("chat-edit" | "thread-error")) {
        let mut state = snapshot(Some("chat-ux"));
        if let Some(panel) = state.message_panel.as_mut() {
            use talk_core::ui::{
                conversation::{Availability, RemoteState},
                messaging::MessageFeature,
            };
            panel.features = page(
                [
                    MessageFeature::Reply,
                    MessageFeature::Edit,
                    MessageFeature::Threads,
                ]
                .map(|feature| {
                    (
                        feature,
                        Availability {
                            implemented: true,
                            server: talk_core::ui::presentation::FeatureSupport::Supported,
                            allowed: Decision::Allowed,
                        },
                    )
                }),
            );
            if scenario == Some("thread-error") {
                panel.thread_root = Some(3);
                panel.thread_state =
                    RemoteState::Failed(RequestId(1), UnavailableReason::PolicyUnknown);
                panel.thread_messages = page(state.messages.as_slice().iter().take(1).cloned());
            }
        }
        state.messages = page(state.messages.as_slice().iter().map(|row| {
            let mut row = row.clone();
            if row.local_id == 3 {
                row.delivery = DeliveryState::Received;
                row.metadata.edit = Decision::Allowed;
            }
            row
        }));
        return state;
    }
    if matches!(
        scenario,
        Some(
            "details-ready"
                | "room-avatar"
                | "lobby-settings"
                | "breakout-configure"
                | "breakout-create"
                | "breakout-free"
                | "breakout-assigned"
                | "breakout-help"
        )
    ) {
        let mut state = snapshot(Some("chat"));
        dialog::configure(&mut state);
        if scenario == Some("lobby-settings")
            && let Some(details) = state
                .conversation_panel
                .as_mut()
                .and_then(|p| p.details.as_mut())
        {
            details.lobby = Some(true);
            details.lobby_timer = Some(1_800_003_600);
        }
        if matches!(
            scenario,
            Some(
                "breakout-configure"
                    | "breakout-create"
                    | "breakout-free"
                    | "breakout-assigned"
                    | "breakout-help"
            )
        ) && let Some(panel) = state.conversation_panel.as_mut()
        {
            use talk_core::ui::conversation::{
                BreakoutMode, BreakoutRoom, DirectorySource, Member, MemberRole, PersonRef,
                RoomSetting,
            };
            let context = panel.context;
            if let Some(details) = panel.details.as_mut() {
                details.breakout_mode = if scenario == Some("breakout-create") {
                    None
                } else {
                    Some(if scenario == Some("breakout-free") {
                        BreakoutMode::Free
                    } else {
                        BreakoutMode::Manual
                    })
                };
                details.breakout_started = Some(true);
                details.breakout_child = scenario == Some("breakout-help");
                if !matches!(scenario, Some("breakout-configure" | "breakout-create")) {
                    details.controls = page(details.controls.as_slice().iter().copied().map(
                        |(key, mut availability)| {
                            if key == RoomSetting::Breakouts {
                                availability.allowed = Decision::Denied;
                            }
                            (key, availability)
                        },
                    ));
                }
            }
            panel.members = page([("Mara Fischer", 11), ("Jonas Weber", 12)].map(
                |(name, local)| Member {
                    id: PersonRef { context, local },
                    name: text(name),
                    role: MemberRole::Member,
                    source: DirectorySource::User,
                    permissions: Some(255),
                    self_user: false,
                },
            ));
            panel.members_loaded = true;
            panel.breakout_rooms = page([
                BreakoutRoom {
                    context: super::fixtures::context(101),
                    name: text("Gruppe 1"),
                    assigned: true,
                },
                BreakoutRoom {
                    context: super::fixtures::context(102),
                    name: text("Gruppe 2"),
                    assigned: false,
                },
            ]);
            if scenario == Some("breakout-assigned") {
                panel.breakout_rooms = page(
                    panel
                        .breakout_rooms
                        .as_slice()
                        .iter()
                        .filter(|room| room.assigned)
                        .cloned(),
                );
            }
            panel.breakout_loaded = true;
            if scenario == Some("breakout-create") {
                panel.breakout_rooms = Page::empty();
                panel.breakout_loaded = false;
            }
        }
        return state;
    }
    if matches!(
        scenario,
        Some(
            "chat-content"
                | "chat-cloud-files"
                | "chat-cloud-files-offline"
                | "chat-polls"
                | "poll-dialog"
                | "poll-drafts"
                | "poll-details"
                | "poll-settings"
                | "chat-voice"
                | "voice-recording"
                | "voice-review"
                | "chat-attachments"
                | "chat-upload"
                | "chat-upload-preview"
                | "chat-upload-uncertain"
                | "chat-content-progress"
                | "chat-content-loading"
                | "chat-content-failed"
        )
    ) {
        let mut state = snapshot(Some("chat-ux"));
        content::configure(&mut state);
        if matches!(
            scenario,
            Some("chat-cloud-files" | "chat-cloud-files-offline")
        ) {
            content::cloud_files(&mut state, "/");
            if scenario == Some("chat-cloud-files-offline") {
                state.content = ContentState::Offline;
                if let Some(browser) = state.content_panel.file_browser.as_mut() {
                    browser.offline = true;
                }
            }
        }
        if matches!(
            scenario,
            Some("chat-polls" | "poll-dialog" | "poll-drafts" | "poll-details" | "poll-settings")
        ) {
            content::polls(&mut state);
        }
        if scenario == Some("poll-settings") {
            dialog::configure(&mut state);
            if let Some(details) = state
                .conversation_panel
                .as_mut()
                .and_then(|panel| panel.details.as_mut())
            {
                details.name = text("Designteam");
            }
        }
        if scenario == Some("poll-details") {
            content::poll_details(&mut state);
        }
        if matches!(
            scenario,
            Some("chat-voice" | "voice-recording" | "voice-review")
        ) {
            content::voice(&mut state);
        }
        if scenario == Some("chat-attachments") {
            content::attachments(&mut state);
        }
        if matches!(scenario, Some("chat-upload" | "chat-upload-uncertain")) {
            content::pending_upload(&mut state, scenario == Some("chat-upload-uncertain"));
        }
        if scenario == Some("chat-upload-preview") {
            content::staged_image(&mut state);
        }
        if matches!(
            scenario,
            Some("chat-content-loading" | "chat-content-failed")
        ) {
            state.content_panel.assets = page(
                state
                    .content_panel
                    .assets
                    .as_slice()
                    .iter()
                    .cloned()
                    .map(|mut asset| {
                        if matches!(
                            asset.target,
                            talk_core::ui::content::AssetTarget::Thumbnail(_)
                                | talk_core::ui::content::AssetTarget::Image(_)
                        ) {
                            asset.state = if scenario == Some("chat-content-loading") {
                                talk_core::ui::content::AssetState::Loading
                            } else {
                                talk_core::ui::content::AssetState::Failed
                            };
                        }
                        asset
                    }),
            );
        }
        if scenario == Some("chat-content-progress") {
            content::transfer(&mut state);
        }
        return state;
    }
    if scenario == Some("chat-motion") {
        let mut state = snapshot(Some("chat-ux"));
        state.messages = page((1..=30).map(|id| {
            message(
                id,
                "Fictional teammate",
                "FT",
                "10:00",
                "Fictional history row for the finite native motion fixture.",
                false,
            )
        }));
        return state;
    }
    if matches!(scenario, Some("chat-ux" | "chat-ux-call")) {
        let mut state = snapshot(Some(if scenario == Some("chat-ux-call") {
            "call"
        } else {
            "chat"
        }));
        configure_chat_ux(&mut state);
        return state;
    }
    if scenario == Some("ready") {
        let mut ready = snapshot(Some("chat"));
        ready.login.storage = StorageStatus::NativeStored;
        ready.login.native_store = NativeStoreState::Available;
        ready.accounts = page(ready.accounts.as_slice().iter().cloned().map(|mut a| {
            a.storage = StorageStatus::NativeStored;
            a
        }));
        return ready;
    }
    let mut s = PresentationSnapshot {
        content_panel: talk_core::ui::content::ContentPanel::default(),
        file_export: None,
        create_panel: None,
        discover_panel: None,
        search_panel: None,
        reference_previews: Page::empty(),
        revision: 1,
        login: LoginView {
            phase: LoginPhase::EnterServer,
            attempt: None,
            server_label: text(""),
            account_label: text(""),
            native_store: NativeStoreState::Unknown,
            storage: StorageStatus::NotSaved,
            storage_failure: None,
        },
        accounts: Page::empty(),
        saved_accounts: Page::empty(),
        account_removal: None,
        active_account: None,
        conversations: Page::empty(),
        threads: Default::default(),
        conversation_list: talk_core::ui::presentation::ConversationListState {
            content: ContentState::Unconfigured,
            complete: false,
        },
        tags: Page::empty(),
        tag_state: ContentState::Unconfigured,
        organization_result: None,
        account_reminders: None,
        federation_invitations: None,
        selected: None,
        content: ContentState::Unconfigured,
        messages: Page::empty(),
        files: Page::empty(),
        participants: Page::empty(),
        conversation_panel: None,
        message_panel: None,
        call: None,
        submission: None,
        mutation: None,
    };
    let Some(scenario) = scenario else {
        return s;
    };
    s.login.server_label = text("cloud.example.test");
    s.login.account_label = text("Alex Sommer");
    s.login.attempt = (scenario != "login-admission"
        && !scenario.starts_with("login-account-store-"))
    .then_some(RequestId(1));
    s.login.phase = match scenario {
        "server" => LoginPhase::EnterServer,
        "validating" => LoginPhase::Validating,
        "browser" => LoginPhase::BrowserWaiting,
        "storage" | "storage-locked" | "storage-denied" | "storage-saving" | "storage-cleanup"
        | "storage-recovery" => LoginPhase::ChooseStorage,
        "login-network" | "login-admission" => LoginPhase::Failed(LoginFailure::Network),
        "login-server-denied" => LoginPhase::Failed(LoginFailure::Connection(
            talk_core::ui::presentation::ConnectionFailure::AccessDenied,
        )),
        "login-tls" => LoginPhase::Failed(LoginFailure::Tls),
        "login-denied" => LoginPhase::Failed(LoginFailure::Denied),
        "login-browser-error" => LoginPhase::Failed(LoginFailure::BrowserUnavailable),
        "login-storage-error" => LoginPhase::Failed(LoginFailure::Storage),
        "login-account-store-busy" => LoginPhase::Failed(LoginFailure::AccountStorage(
            talk_core::ui::presentation::AccountLoadFailure::InUse,
        )),
        "login-account-store-device-changed" => LoginPhase::Failed(LoginFailure::AccountStorage(
            talk_core::ui::presentation::AccountLoadFailure::DeviceChanged,
        )),
        "login-account-store-error" => LoginPhase::Failed(LoginFailure::AccountStorage(
            talk_core::ui::presentation::AccountLoadFailure::RecoveryRequired,
        )),
        "expired" => LoginPhase::Failed(LoginFailure::Expired),
        _ => LoginPhase::Ready,
    };
    s.login.native_store = match scenario {
        "storage-locked" => NativeStoreState::Locked,
        "storage-denied" => NativeStoreState::Denied,
        _ => NativeStoreState::Missing,
    };
    if scenario.starts_with("login-account-store-") {
        s.login.server_label = text("");
        return s;
    }
    s.login.storage = match scenario {
        "storage-saving" => StorageStatus::Saving,
        "storage-cleanup" => StorageStatus::CleanupPending,
        "storage-recovery" => StorageStatus::RecoveryRequired,
        _ => StorageStatus::NotSaved,
    };
    if matches!(
        scenario,
        "server"
            | "validating"
            | "browser"
            | "storage"
            | "expired"
            | "ready"
            | "storage-locked"
            | "storage-denied"
            | "storage-saving"
            | "storage-cleanup"
            | "storage-recovery"
            | "login-network"
            | "login-admission"
            | "login-server-denied"
            | "login-tls"
            | "login-denied"
            | "login-browser-error"
            | "login-storage-error"
    ) {
        return s;
    }
    s.accounts = page([AccountView {
        scope: scope(),
        navigation_key: Some(
            talk_core::preferences::navigation::NavigationAccountKey::new(
                "a1:00000000000000000000000000000001:00000000000000000000000000000002",
            )
            .expect("fixture key"),
        ),
        connection: talk_core::ui::presentation::AccountConnectionState::Disconnected,
        connection_failure: match scenario {
            "chat-server-denied" | "accounts-server-denied" => {
                Some(talk_core::ui::presentation::ConnectionFailure::AccessDenied)
            }
            "chat-server-unavailable" => {
                Some(talk_core::ui::presentation::ConnectionFailure::Unreachable)
            }
            _ => None,
        },
        label: text("Nord Studio"),
        server_label: text("cloud.example.test"),
        self_name: text("Alex Sommer"),
        initials: text("AS"),
        presence: Presence::Available,
        storage: StorageStatus::NotSaved,
    }]);
    s.active_account = Some(scope());
    s.content = ContentState::Ready;
    let rows = [
        (
            1,
            "Produktteam",
            "PT",
            "Mara: Perfekt. Dann nehmen wir das so mit.",
            ConversationKind::Group,
            true,
            0,
        ),
        (
            2,
            "Teamküche",
            "TK",
            "Gesprächsraum · Gespräch öffnen",
            ConversationKind::Group,
            true,
            0,
        ),
        (
            3,
            "Mara Fischer",
            "MF",
            "Hast du später kurz Zeit?",
            ConversationKind::Direct,
            false,
            2,
        ),
        (
            4,
            "Designbesprechung",
            "DB",
            "Nächster Termin am Donnerstag",
            ConversationKind::GuestAccessible,
            false,
            0,
        ),
        (
            5,
            "Jonas Weber",
            "JW",
            "Danke, schaue ich mir an.",
            ConversationKind::Direct,
            false,
            0,
        ),
        (
            6,
            "Notiz an mich",
            "AS",
            "Ideen für die nächste Runde",
            ConversationKind::NoteToSelf,
            false,
            0,
        ),
    ];
    s.conversation_list = talk_core::ui::presentation::ConversationListState {
        content: ContentState::Ready,
        complete: true,
    };
    s.conversations = page(
        rows.map(
            |(id, title, initials, subtitle, kind, favorite, unread)| ConversationView {
                tags: None,
                context: context(id),
                navigation_key: None,
                title: text(title),
                initials: text(initials),
                subtitle: text(subtitle),
                updated_label: text(if id == 1 { "09:47" } else { "Gestern" }),
                kind,
                favorite: Some(favorite),
                organization: talk_core::ui::presentation::ConversationOrganization {
                    category: match id {
                        1 => talk_core::ui::presentation::ConversationCategory::Channel,
                        2 => talk_core::ui::presentation::ConversationCategory::Huddle,
                        4 => talk_core::ui::presentation::ConversationCategory::Conference(
                            talk_core::ui::presentation::ConferencePreset::Standard,
                        ),
                        _ => talk_core::ui::presentation::ConversationCategory::Unknown,
                    },
                    classified: Some(false),
                    ..Default::default()
                },
                archived: Some(id == 6),
                archive: Decision::Unknown,
                unread,
                unread_mention: Some(id == 1),
                notification_muted: Some(id == 4),
                last_activity: Some(100 - id as u64),
                unread_direct_mention: Some(id == 1 && unread > 0),
                last_read_message: (id == 1).then_some(1),
                presence: if scenario == "chat-presence" && id == 3 {
                    Presence::Away
                } else if kind == ConversationKind::Direct {
                    Presence::Available
                } else {
                    Presence::Unknown
                },
                status_icon: (scenario == "chat-presence" && id == 3).then(|| text("☕")),
                status_message: (scenario == "chat-presence" && id == 3)
                    .then(|| text("Kaffeepause")),
                participants: Some(if kind == ConversationKind::Direct {
                    2
                } else {
                    7
                }),
                participant_names: if id == 1 {
                    page([text("Mara Fischer"), text("Jonas Weber")])
                } else {
                    Page::empty()
                },
                send: Decision::Allowed,
                attach: Decision::Unknown,
                draft_clipboard: Decision::Allowed,
                call: if kind == ConversationKind::NoteToSelf {
                    Decision::Denied
                } else {
                    Decision::Allowed
                },
                call_active: Some(id == 2 && matches!(scenario, "call-dock" | "chat-ux-call")),
                call_scope: if kind == ConversationKind::NoteToSelf {
                    None
                } else {
                    Some(call_scope(id))
                },
            },
        ),
    );
    populate(
        &mut s,
        context(
            if matches!(scenario, "direct-call" | "outgoing" | "chat-presence") {
                3
            } else if matches!(scenario, "voice" | "voice-room" | "prejoin-voice") {
                2
            } else if scenario == "room" {
                4
            } else {
                1
            },
        ),
    );
    if scenario == "search-filter" {
        s.conversations = page(s.conversations.as_slice().iter().cloned().map(|mut row| {
            if row.context == context(2) {
                row.unread_mention = None;
            }
            if row.context == context(1) {
                row.unread = 1;
            }
            row
        }));
    }
    match scenario {
        "offline" => s.content = ContentState::Offline,
        "offline-empty" => {
            s.content = ContentState::Offline;
            s.messages = Page::empty();
        }
        "loading-retained" => s.content = ContentState::Loading,
        "loading" => {
            s.content = ContentState::Loading;
            s.messages = Page::empty();
        }
        "denied" => {
            s.content = ContentState::Denied;
            s.messages = Page::empty();
            s.files = Page::empty();
            s.participants = Page::empty();
        }
        "error" => s.content = ContentState::Failed,
        "empty" => s.messages = Page::empty(),
        _ => (),
    }
    if matches!(
        scenario,
        "incoming"
            | "outgoing"
            | "call"
            | "direct-call"
            | "voice"
            | "gate"
            | "gate-mic"
            | "gate-camera"
            | "gate-recording"
            | "gate-recording-direct"
            | "gate-lobby"
            | "connecting"
            | "reconnecting"
            | "call-ended"
            | "call-error"
            | "call-error-unknown"
            | "call-controls"
            | "call-gallery"
            | "call-participants-raised"
            | "call-dock"
            | "call-devices"
            | "call-speaker"
            | "call-screen-stage"
            | "call-recording"
            | "call-recording-confirm"
            | "call-recording-failed"
            | "call-end-confirm"
            | "call-reactions"
            | "sharing"
            | "call-denied"
            | "call-active"
            | "call-requested"
    ) {
        let direct = matches!(
            scenario,
            "incoming" | "outgoing" | "direct-call" | "gate-recording-direct"
        );
        let id = if direct {
            3
        } else if matches!(scenario, "voice" | "call-dock") {
            2
        } else {
            1
        };
        s.call = Some(CallView {
            scope: call_scope(id),
            title: text(if direct {
                "Mara Fischer"
            } else if id == 2 {
                "Teamküche"
            } else {
                "Produktteam"
            }),
            direct,
            stage: match scenario {
                "incoming" => CallStage::Incoming,
                "outgoing" => CallStage::Outgoing,
                "gate" => CallStage::Required(RequiredGate::SecurityPolicy),
                "gate-mic" => CallStage::Required(RequiredGate::MicrophonePermission),
                "gate-camera" => CallStage::Required(RequiredGate::CameraPermission),
                "gate-recording" | "gate-recording-direct" => {
                    CallStage::Required(RequiredGate::RecordingConsent)
                }
                "gate-lobby" => CallStage::Required(RequiredGate::Lobby),
                "connecting" => CallStage::Connecting,
                "reconnecting" => CallStage::Reconnecting,
                "call-ended" => CallStage::Ended,
                "call-error" | "call-error-unknown" => CallStage::Failed,
                _ => CallStage::Active,
            },
            accept: Decision::Allowed,
            microphone: if scenario == "call-active" {
                MediaState::Active
            } else if scenario == "call-requested" {
                MediaState::Requested
            } else if scenario == "call-controls" {
                MediaState::Off
            } else if scenario == "call-denied" {
                MediaState::Denied
            } else {
                MediaState::Unavailable
            },
            camera: if scenario == "call-active" {
                MediaState::Active
            } else if scenario == "call-requested" {
                MediaState::Requested
            } else if scenario == "call-controls" {
                MediaState::Off
            } else if scenario == "call-denied" {
                MediaState::Denied
            } else {
                MediaState::Unavailable
            },
            share: if scenario == "sharing" {
                MediaState::Active
            } else if scenario == "call-controls" {
                MediaState::Off
            } else {
                MediaState::Unavailable
            },
            sharing: if scenario == "sharing" {
                ScreenShareState::Sharing
            } else {
                ScreenShareState::Idle
            },
            streams: Page::empty(),
            recording: match scenario {
                "call-recording" => talk_core::ui::presentation::CallRecordingState::Active,
                "call-recording-failed" => talk_core::ui::presentation::CallRecordingState::Failed,
                _ => talk_core::ui::presentation::CallRecordingState::Unknown,
            },
            recording_control: match scenario {
                "call-participants-raised" | "call-recording-confirm" => Some(true),
                "call-recording" => Some(false),
                _ => None,
            },
            recording_pending: false,
            recording_uncertain: false,
            hand_supported: matches!(
                scenario,
                "call-gallery" | "call-participants-raised" | "call-speaker" | "call-reactions"
            ),
            hand_raised: false,
            reactions: if matches!(
                scenario,
                "call-gallery" | "call-participants-raised" | "call-speaker" | "call-reactions"
            ) {
                page([text("👍"), text("🎉"), text("❤️")])
            } else {
                Page::empty()
            },
            latest_reaction: if scenario == "call-reactions" {
                Some(talk_core::ui::presentation::CallReactionView {
                    sequence: 1,
                    participant: Some(1),
                    emoji: text("🎉"),
                })
            } else {
                None
            },
            invitees_loaded: scenario == "call-participants-raised",
            invitees: if scenario == "call-participants-raised" {
                page([
                    talk_core::ui::presentation::CallInviteeView {
                        local_id: 1001,
                        ring_allowed: true,
                        name: text("Robin Winter"),
                        initials: text("RW"),
                    },
                    talk_core::ui::presentation::CallInviteeView {
                        local_id: 1002,
                        ring_allowed: false,
                        name: text("Mina Roth"),
                        initials: text("MR"),
                    },
                ])
            } else {
                Page::empty()
            },
            ring_status: None,
            mute_status: None,
            can_end_everyone: matches!(scenario, "call-end-confirm" | "call-participants-raised"),
            end_pending: false,
            end_failed: false,
            participants: page(
                if matches!(
                    scenario,
                    "call-gallery"
                        | "call-participants-raised"
                        | "call-screen-stage"
                        | "call-speaker"
                        | "call-reactions"
                ) {
                    let mut people = participants().to_vec();
                    people[0].raised_hand = true;
                    people[0].call_microphone = Some(false);
                    people[0].call_camera = Some(false);
                    people[1].call_microphone = Some(true);
                    people[1].speaking = true;
                    people[1].call_can_mute = scenario == "call-participants-raised";
                    people[1].call_camera = Some(false);
                    people[3].raised_hand = true;
                    if scenario != "call-participants-raised" {
                        people.extend([
                            participant(5, "Nora Beck", "NB", false),
                            participant(6, "Emil Hartmann", "EH", false),
                            participant(7, "Samira Wolf", "SW", false),
                            participant(8, "Luis Berger", "LB", false),
                        ]);
                    }
                    people
                } else if direct || id == 2 {
                    vec![participants()[0].clone(), participants()[2].clone()]
                } else {
                    participants().to_vec()
                },
            ),
        });
    }
    if let Some(call) = s.call.as_mut() {
        call.streams =
            page(
                call.participants
                    .as_slice()
                    .iter()
                    .take(15)
                    .map(|person| CallStreamView {
                        local_id: person.local_id,
                        participant: person.local_id,
                        participant_name: person.name.clone(),
                        kind: CallTileKind::Camera,
                        dimensions: None,
                        flowing: false,
                    }),
            );
        if scenario == "call-screen-stage" {
            let _ = call.streams.try_push(CallStreamView {
                local_id: 9001,
                participant: 1,
                participant_name: text("Mara Fischer"),
                kind: CallTileKind::Screen,
                dimensions: Some((1280, 720)),
                flowing: false,
            });
        }
    }
    if scenario == "gate-lobby" {
        dialog::configure(&mut s);
        if let Some(details) = s
            .conversation_panel
            .as_mut()
            .and_then(|panel| panel.details.as_mut())
        {
            details.lobby = Some(true);
            details.lobby_timer = Some(1_800_003_600);
        }
    }
    if scenario == "call-error-unknown" {
        let mut rows = s.conversations.as_slice().to_vec();
        rows[0].call = Decision::Unknown;
        s.conversations = page(rows);
    }
    if scenario == "clipboard-denied" {
        let mut rows = s.conversations.as_slice().to_vec();
        rows[0].draft_clipboard = Decision::Unknown;
        s.conversations = page(rows);
    }
    if scenario == "long-labels" {
        let mut rows = s.conversations.as_slice().to_vec();
        rows[0].title =
            text("Produktteam · Internationale Zusammenarbeit und barrierefreie Kommunikation");
        s.conversations = page(rows);
    }
    if scenario.starts_with("history") {
        s.messages = page((1..=40).map(|n| {
            message(
                100 + n,
                "Mara Fischer",
                "MF",
                "09:47",
                &format!(
                    "Geladene Nachricht {n:02} · {}",
                    if n == 40 {
                        "Neuester geladener Stand."
                    } else {
                        "Fiktiver Verlauf für die lokale Navigation."
                    }
                ),
                false,
            )
        }));
        if scenario == "history-pagination"
            && let Some(context) = s.selected
        {
            let mut panel = talk_core::ui::messaging::MessagePanel::new(context);
            panel.older = Some(101);
            s.message_panel = Some(panel);
        }
    }
    if scenario == "account-search" {
        s.accounts = page(s.accounts.as_slice().iter().cloned().map(|mut account| {
            account.connection = talk_core::ui::presentation::AccountConnectionState::Connected;
            account
        }));
        s.search_panel = Some(talk_core::ui::search::Panel {
            account: scope(),
            request: RequestId(1),
            term: text("Navigation"),
            filters: talk_core::ui::search::Filters {
                conversation: None,
                person: None,
                from_self: false,
                since: None,
                until: None,
            },
            state: talk_core::ui::conversation::RemoteState::Idle,
            hits: page([
                talk_core::ui::search::Hit {
                    context: context(1),
                    message: 1,
                    title: text("Mara Fischer"),
                    excerpt: text("Die neue Navigation ist bereit für einen letzten Blick."),
                },
                talk_core::ui::search::Hit {
                    context: context(1),
                    message: 2,
                    title: text("Jonas Weber"),
                    excerpt: text("Können wir die Navigation noch einmal prüfen?"),
                },
                talk_core::ui::search::Hit {
                    context: context(2),
                    message: 8,
                    title: text("Alex Sommer"),
                    excerpt: text("Die Navigation ist im Entwurf aktualisiert."),
                },
            ]),
            loaded: true,
            more: true,
        });
    }
    if scenario == "discover" {
        s.accounts = page(s.accounts.as_slice().iter().cloned().map(|mut account| {
            account.connection = talk_core::ui::presentation::AccountConnectionState::Connected;
            account
        }));
        s.discover_panel = Some(talk_core::ui::discover::Panel {
            account: scope(),
            request: RequestId(1),
            query: text(""),
            support: talk_core::ui::presentation::FeatureSupport::Supported,
            state: talk_core::ui::conversation::RemoteState::Idle,
            rooms: page([
                talk_core::ui::discover::Room {
                    context: context(7),
                    name: text("Designrunde"),
                    description: Some(text(
                        "Ideen, Entwürfe und offene Fragen aus dem Designteam.",
                    )),
                    participants: Some(18),
                    join: Decision::Allowed,
                },
                talk_core::ui::discover::Room {
                    context: context(8),
                    name: text("Kaffeepause"),
                    description: Some(text("Ein Ort für kurze Gespräche zwischendurch.")),
                    participants: Some(9),
                    join: Decision::Allowed,
                },
                talk_core::ui::discover::Room {
                    context: context(9),
                    name: text("Community-Projekte"),
                    description: Some(text("Gemeinsam an neuen Projekten arbeiten.")),
                    participants: None,
                    join: Decision::Allowed,
                },
            ]),
            loaded: true,
            joining: None,
            uncertain: false,
        });
    }
    if matches!(scenario, "create34" | "create35") {
        use talk_core::ui::conversation::{
            AccountPerson, AccountPersonRef, CreatePanel, CreationPreset,
        };
        let mut panel = CreatePanel::new(scope());
        panel.group = Decision::Allowed;
        panel.extended = talk_core::ui::presentation::FeatureSupport::Supported;
        panel.public = talk_core::ui::presentation::FeatureSupport::Supported;
        panel.private_allowed = true;
        panel.public_allowed = true;
        panel.invites = page([
            AccountPerson {
                id: AccountPersonRef {
                    account: scope(),
                    local: 1,
                },
                name: text("Mara Fischer"),
                label: text("Design"),
            },
            AccountPerson {
                id: AccountPersonRef {
                    account: scope(),
                    local: 2,
                },
                name: text("Jonas Weber"),
                label: text("Produkt"),
            },
        ]);
        panel.presets = page([
            CreationPreset {
                id: text("webinar"),
                name: text("Webinar"),
                description: text("Für moderierte Veranstaltungen."),
            },
            CreationPreset {
                id: text("presentation"),
                name: text("Präsentation"),
                description: text("Für gemeinsame Präsentationen."),
            },
        ]);
        s.create_panel = Some(panel);
    }
    s
}
fn populate(s: &mut PresentationSnapshot, selected: ConversationRef) {
    s.selected = Some(selected);
    s.participants = page(participants());
    s.files = page([file()]);
    if selected.conversation.local == 1 {
        let mut first = message(
            1,
            "Mara Fischer",
            "MF",
            "09:41",
            "Guten Morgen! Die neue Navigation ist bereit für einen letzten Blick.",
            false,
        );
        first.attachments = page([file()]);
        let mut second = message(
            2,
            "Jonas Weber",
            "JW",
            "09:43",
            "Sieht gut aus. Können wir die Suche noch mit einem Tastenkürzel öffnen?",
            false,
        );
        second.reply_count = 3;
        s.messages = page([
            first,
            second,
            message(
                3,
                "Alex Sommer",
                "AS",
                "09:45",
                "Ja, ist drin. Ich habe auch den Wechsel zwischen den Gesprächen vereinfacht.",
                true,
            ),
            message(
                4,
                "Mara Fischer",
                "MF",
                "09:47",
                "Perfekt. Dann nehmen wir das so mit.",
                false,
            ),
        ]);
    } else {
        s.messages = page([message(
            5,
            "Mara Fischer",
            "MF",
            "09:51",
            "Hier ist Platz für unser Gespräch. Diese Inhalte sind fiktive UI-Beispiele.",
            false,
        )]);
    }
}

/// Explicit, finite interaction fixture. No server result or native operation is
/// inferred from these samples; unsupported writes keep their existing failure.
fn configure_chat_ux(snapshot: &mut PresentationSnapshot) {
    use talk_core::ui::conversation::Availability;
    use talk_core::ui::messaging::{MessageFeature, MessagePanel, RichObject};
    use talk_core::ui::presentation::{ConversationCategory, FeatureSupport};
    let Some(context) = snapshot.selected else {
        return;
    };
    snapshot.conversations = page(snapshot.conversations.as_slice().iter().cloned().map(
        |mut row| {
            row.organization.favorites = FeatureSupport::Supported;
            row.organization.favorite_change = Decision::Allowed;
            if row.context.conversation.local == 1 {
                row.title = text("Designteam");
                row.organization.category = ConversationCategory::Unknown;
                row.participants = Some(2);
            }
            row
        },
    ));
    let mut rows = snapshot.messages.as_slice().to_vec();
    for row in &mut rows {
        row.metadata.replyable = true;
        row.metadata.unread_after = row.local_id.checked_sub(1).filter(|id| *id > 0);
        if row.local_id == 2 {
            row.metadata.deleted = true;
        }
        if row.local_id == 3 {
            row.delivery = DeliveryState::Pending;
        }
    }
    let mut panel = MessagePanel::new(context);
    panel.pin_change = Decision::Allowed;
    panel.pins_loaded = true;
    panel.features = page(
        [
            MessageFeature::Reply,
            MessageFeature::Search,
            MessageFeature::Files,
            MessageFeature::Pins,
            MessageFeature::MarkUnread,
            MessageFeature::UnreadFrom,
        ]
        .map(|feature| {
            (
                feature,
                Availability {
                    implemented: true,
                    server: FeatureSupport::Supported,
                    allowed: Decision::Allowed,
                },
            )
        }),
    );
    panel.file_limit = 10 * 1024 * 1024;
    if let Some(first) = rows.first_mut() {
        first.metadata.rich = page([RichObject {
            id: None,
            mime_type: Some(text("image/png")),
            size: None,
            placeholder: text("file0"),
            kind: text("file"),
            name: text("Team-skizze.png"),
            link: None,
        }]);
        panel.shared_files = page([first.clone()]);
    }
    if let Some(last) = rows.last() {
        panel.pinned_messages = page([last.clone()]);
    }
    snapshot.messages = page(rows);
    snapshot.message_panel = Some(panel);
}

/// Three finite projection transitions driven only by explicit development F6:
/// reorder, remove focused item, then replace account/runtime/host scope.
fn focus_snapshot(files: bool, step: u8) -> PresentationSnapshot {
    let mut s = snapshot(Some("chat"));
    let mut rows = s.conversations.as_slice().to_vec();
    if !files && step == 1 {
        rows.swap(2, 4);
    }
    if !files && step >= 2 {
        rows.retain(|r| r.context.conversation.local != 3);
    }
    if step == 3 {
        let account = AccountScope {
            account: AccountId::from_local(2),
            runtime: RuntimeGeneration::from_local(2),
        };
        let mut accounts = s.accounts.as_slice().to_vec();
        accounts[0].scope = account;
        s.accounts = page(accounts);
        s.active_account = Some(account);
        for row in &mut rows {
            row.context.account = account;
            row.context.conversation.account = account.account;
            row.context.conversation.host = HostId::from_local(2);
            row.call_scope = None;
            row.call = Decision::Unknown;
        }
        s.selected = Some(rows[0].context);
    }
    s.conversations = page(rows);
    if files {
        let mut first = file();
        first.download = Decision::Allowed;
        let second = FileView {
            local_id: 2,
            name: text("Konzept.pdf"),
            detail: text("PDF · 1 MB"),
            download: Decision::Allowed,
        };
        let entries = match step {
            0 => vec![first, second],
            1 => vec![second, first],
            _ => vec![second],
        };
        let mut messages = s.messages.as_slice().to_vec();
        messages[0].attachments = page(entries.clone());
        s.messages = page(messages);
        s.files = page(entries);
    }
    s
}

// These are explicitly LIVE fictional runtime scopes, not dormant/stored accounts.
fn account_snapshot(step: u8) -> PresentationSnapshot {
    let mut s = snapshot(Some("chat"));
    let template = s.accounts.as_slice()[0].clone();
    let mut accounts = Vec::with_capacity(3);
    for (id, label, name, initials) in [
        (1, "Nord Studio", "Alex Sommer", "AS"),
        (2, "Werkstatt", "Samira Winter", "SW"),
        (3, "Projektbüro", "Robin Keller", "RK"),
    ] {
        let mut a = template.clone();
        a.scope = AccountScope {
            account: AccountId::from_local(id),
            runtime: RuntimeGeneration::from_local(if step == 3 && id == 3 { 2 } else { 1 }),
        };
        a.label = text(label);
        a.self_name = text(name);
        a.initials = text(initials);
        accounts.push(a);
    }
    if step == 1 {
        accounts.swap(1, 2);
    }
    if step >= 2 {
        accounts.retain(|a| a.scope.account.as_local() != 2);
    }
    if step == 3 {
        let active = accounts
            .iter()
            .find(|a| a.scope.account.as_local() == 3)
            .unwrap()
            .scope;
        s.active_account = Some(active);
        let mut rows = s.conversations.as_slice().to_vec();
        for row in &mut rows {
            row.context.account = active;
            row.context.conversation.account = active.account;
            row.context.conversation.host = HostId::from_local(3);
            row.call_scope = None;
            row.call = Decision::Unknown;
        }
        s.selected = Some(rows[0].context);
        s.conversations = page(rows);
    }
    s.accounts = page(accounts);
    s
}

pub struct DevelopmentPort {
    reaction_fixture: Option<bool>,
    reaction_diagnostics: u8,
    thread_fixture: Option<bool>,
    snapshot: PresentationSnapshot,
    dirty: bool,
    enabled: bool,
    busy: bool,
    uncertain: bool,
    pending: bool,
    intents: u8,
    chat_ux_send_diagnostics: u8,
    hand_diagnostics: u8,
    consent_diagnostics: u8,
    busy_read_diagnostics: u8,
    account_probe: bool,
    focus_probe: Option<bool>, // false: rooms; true: files
    probe_step: u8,
    chat_ux: bool,
    chat_motion: bool,
    history_pagination: bool,
    chat_content: bool,
    people_fixture: bool,
}
impl DevelopmentPort {
    pub fn new(scenario: Option<&str>) -> Self {
        let focus_probe = match scenario {
            Some("focus-rooms") => Some(false),
            Some("focus-files") => Some(true),
            _ => None,
        };
        let account_probe = scenario == Some("focus-accounts");
        let initial = if account_probe || scenario == Some("accounts") {
            account_snapshot(0)
        } else if let Some(files) = focus_probe {
            focus_snapshot(files, 0)
        } else {
            snapshot(scenario)
        };
        Self {
            reaction_fixture: matches!(
                scenario,
                Some(
                    "chat-reactions"
                        | "chat-reactions-pending"
                        | "chat-reactions-denied"
                        | "reaction-authors"
                )
            )
            .then_some(scenario == Some("chat-reactions-pending")),
            reaction_diagnostics: 0,
            thread_fixture: match scenario {
                Some(
                    "chat-edit" | "thread" | "thread-list" | "thread-options" | "thread-offline"
                    | "thread-paged",
                ) => Some(false),
                Some("thread-error") => Some(true),
                _ => None,
            },
            account_probe,
            focus_probe,
            probe_step: 0,
            chat_ux: matches!(
                scenario,
                Some(
                    "chat-ux"
                        | "chat-ux-call"
                        | "chat-edit"
                        | "thread"
                        | "thread-error"
                        | "thread-list"
                        | "thread-options"
                        | "thread-offline"
                        | "thread-paged"
                        | "chat-send-pending"
                        | "unread-divider"
                        | "unread-jump"
                        | "reaction-authors"
                )
            ),
            chat_motion: scenario == Some("chat-motion"),
            people_fixture: scenario == Some("people-card"),
            history_pagination: scenario == Some("history-pagination"),
            chat_content: matches!(
                scenario,
                Some(
                    "chat-content"
                        | "chat-cloud-files"
                        | "chat-cloud-files-offline"
                        | "chat-polls"
                        | "poll-dialog"
                        | "poll-drafts"
                        | "poll-details"
                        | "poll-settings"
                        | "chat-voice"
                        | "voice-recording"
                        | "voice-review"
                        | "chat-attachments"
                        | "chat-upload"
                        | "chat-upload-preview"
                        | "chat-upload-uncertain"
                        | "chat-content-progress"
                        | "chat-content-loading"
                        | "chat-content-failed"
                )
            ),
            snapshot: initial,
            dirty: true,
            enabled: scenario.is_some(),
            busy: matches!(scenario, Some("busy" | "history-busy" | "chat-busy")),
            uncertain: matches!(scenario, Some("uncertain" | "history-uncertain")),
            pending: matches!(
                scenario,
                Some("pending" | "history-pending" | "chat-send-pending")
            ),
            intents: 0,
            chat_ux_send_diagnostics: 0,
            hand_diagnostics: 0,
            consent_diagnostics: 0,
            busy_read_diagnostics: 0,
        }
    }
}
impl NativeDesktopPort for DevelopmentPort {
    fn try_submit(&mut self, intent: UiIntent) -> Result<(), SubmitError> {
        if !self.enabled {
            return Err(SubmitError::Unavailable(UnavailableReason::Unconfigured));
        }
        // This sink consumes synchronously; a bounded counter is diagnostic, no intent backlog/log.
        self.intents = self.intents.saturating_add(1);
        // At most 32 generic admission diagnostics: only local IDs
        // and input lengths, never draft content, credentials or backend data.
        if self.intents <= 32 {
            // Explicit fixture-only observation of every port admission attempt.
            // Only a bounded ordinal, no payload or server-derived text.
            println!("UI_DISPATCH ordinal={}", self.intents);
            match &intent {
                UiIntent::SelectAccount(a) => println!(
                    "UI_FOCUS_PROBE select account={} runtime={}",
                    a.account.as_local(),
                    a.runtime.as_local()
                ),
                UiIntent::OpenConversation(c) => println!(
                    "UI_FOCUS_PROBE open account={} room={}",
                    c.account.account.as_local(),
                    c.conversation.local
                ),
                UiIntent::DownloadFile { context, local_id } => println!(
                    "UI_FOCUS_PROBE download account={} file={}",
                    context.account.account.as_local(),
                    local_id
                ),
                UiIntent::SendDraft {
                    request,
                    context,
                    body,
                } => println!(
                    "UI_INTENT send request={} account={} room={} bytes={} lines={}",
                    request.0,
                    context.account.account.as_local(),
                    context.conversation.local,
                    body.as_str().len(),
                    body.as_str().split('\n').count()
                ),
                UiIntent::Legacy(UserIntent::AcceptDirect { call, .. }) => {
                    println!("UI_INTENT accept-direct room={}", call.conversation.local)
                }
                UiIntent::JoinConference { call, .. } => {
                    println!("UI_INTENT join-conference room={}", call.conversation.local)
                }
                UiIntent::Legacy(UserIntent::Leave(call)) => {
                    println!("UI_INTENT leave room={}", call.conversation.local)
                }
                UiIntent::SetMicrophone { call, .. } => {
                    println!("UI_INTENT microphone room={}", call.conversation.local)
                }
                UiIntent::SetCamera { call, .. } => {
                    println!("UI_INTENT camera room={}", call.conversation.local)
                }
                UiIntent::ShareScreen(call) => {
                    println!("UI_INTENT share room={}", call.conversation.local)
                }
                _ => (),
            }
        }
        // Background fixture dispatches can exhaust the general quota before
        // the native call-panel keyboard probe runs. This remains bounded and
        // records only a local room identity and the explicit hand state.
        if self.hand_diagnostics < 8
            && let UiIntent::SetCallHand { call, raised } = &intent
        {
            self.hand_diagnostics += 1;
            println!(
                "UI_INTENT hand room={} raised={raised}",
                call.conversation.local
            );
        }
        if self.consent_diagnostics < 4
            && let UiIntent::ResolveRequiredGate {
                call,
                gate: RequiredGate::RecordingConsent,
            } = &intent
        {
            self.consent_diagnostics += 1;
            println!(
                "UI_INTENT recording-consent room={}",
                call.conversation.local
            );
        }
        // Content probes can exhaust the general quota before a user sends.
        // Reserve four send diagnostics for the explicit chat interaction fixture.
        if self.chat_ux
            && self.chat_ux_send_diagnostics < 4
            && let UiIntent::Message {
                context,
                action: talk_core::ui::messaging::MessageAction::Send { body, target, .. },
                ..
            } = &intent
        {
            self.chat_ux_send_diagnostics += 1;
            println!(
                "UI_CHAT_UX send room={} bytes={} reply={:?} thread={:?}",
                context.conversation.local,
                body.as_str().len(),
                target.reply_to,
                target.thread,
            );
        }
        if self.busy
            && matches!(
                &intent,
                UiIntent::Message {
                    action: talk_core::ui::messaging::MessageAction::MarkRead(_),
                    ..
                }
            )
        {
            if self.busy_read_diagnostics < 4 {
                self.busy_read_diagnostics += 1;
                println!("UI_BUSY automatic_read_rejected=true no_backend=true");
            }
            return Err(SubmitError::Busy);
        }
        match intent {
            UiIntent::RefreshThreads(account) if self.snapshot.threads.account == Some(account) => {
                self.snapshot.threads.content = ContentState::Ready;
            }
            UiIntent::PageThreads { account, offset }
                if self.snapshot.threads.account == Some(account) =>
            {
                self.snapshot.threads.offset = offset;
            }
            UiIntent::MarkRoomUnread { context, .. }
                if self.snapshot.active_account == Some(context.account) =>
            {
                self.snapshot.conversations = page(
                    self.snapshot
                        .conversations
                        .as_slice()
                        .iter()
                        .cloned()
                        .map(|mut room| {
                            if room.context == context {
                                room.unread = room.unread.max(1);
                            }
                            room
                        }),
                );
            }
            UiIntent::Message {
                context,
                action: talk_core::ui::messaging::MessageAction::ReactionAuthors { message, emoji },
                ..
            } if self.reaction_fixture.is_some() && self.snapshot.selected == Some(context) => {
                let panel = self
                    .snapshot
                    .message_panel
                    .as_mut()
                    .ok_or(SubmitError::StaleScope)?;
                panel.reaction_authors = page([talk_core::ui::messaging::ReactionAuthors {
                    message,
                    emoji,
                    authors: page([text("Mara Fischer"), text("Alex Sommer")]),
                }]);
            }
            UiIntent::Message {
                request,
                context,
                action:
                    talk_core::ui::messaging::MessageAction::React {
                        message,
                        emoji,
                        add,
                    },
            } if self.reaction_fixture.is_some() => {
                use talk_core::ui::{
                    conversation::RemoteState,
                    messaging::{MessageFeature, Reaction},
                };
                if self.snapshot.selected != Some(context) {
                    return Err(SubmitError::StaleScope);
                }
                let panel = self
                    .snapshot
                    .message_panel
                    .as_mut()
                    .ok_or(SubmitError::StaleScope)?;
                if !panel.availability(MessageFeature::React).enabled() {
                    return Err(SubmitError::Unavailable(
                        UnavailableReason::PermissionDenied,
                    ));
                }
                if panel.reaction_pending.is_some() {
                    return Err(SubmitError::Busy);
                }
                let row = self
                    .snapshot
                    .messages
                    .as_slice()
                    .iter()
                    .find(|row| {
                        row.local_id == message && !row.metadata.deleted && !row.metadata.system
                    })
                    .ok_or(SubmitError::StaleScope)?;
                if self.reaction_fixture == Some(true) {
                    panel.reaction_pending = Some(message);
                    panel.state = RemoteState::Saving(request);
                } else {
                    let mut updated = row.clone();
                    let mut counts = row.metadata.reactions.as_slice().to_vec();
                    if let Some(reaction) = counts.iter_mut().find(|r| r.emoji == emoji) {
                        if add && !reaction.own {
                            reaction.count += 1;
                            reaction.own = true;
                        }
                        if !add && reaction.own {
                            reaction.count = reaction.count.saturating_sub(1);
                            reaction.own = false;
                        }
                    } else if add {
                        counts.push(Reaction {
                            emoji: emoji.clone(),
                            count: 1,
                            own: true,
                        });
                    }
                    counts.retain(|reaction| reaction.count > 0);
                    updated.metadata.reactions = page(counts);
                    self.snapshot.messages =
                        page(self.snapshot.messages.as_slice().iter().map(|row| {
                            if row.local_id == message {
                                updated.clone()
                            } else {
                                row.clone()
                            }
                        }));
                    panel.state = RemoteState::Idle;
                    panel.result = Some((request, Completion::ServerAccepted));
                }
                if self.reaction_diagnostics < 8 {
                    self.reaction_diagnostics += 1;
                    println!(
                        "UI_REACTION_FIXTURE message={message} emoji={} add={add} pending={} no_backend=true",
                        emoji.as_str(),
                        self.reaction_fixture == Some(true)
                    );
                }
            }
            UiIntent::Content {
                request,
                account,
                action,
            } if self.chat_content => {
                if self.snapshot.active_account != Some(account) {
                    return Err(SubmitError::StaleScope);
                }
                content::action(&mut self.snapshot, request, action)?;
            }
            UiIntent::Refresh(_) if self.chat_motion => {
                self.probe_step = (self.probe_step + 1).min(14);
                let mut rows = self.snapshot.messages.as_slice().to_vec();
                let step = self.probe_step;
                match step {
                    1..=5 | 10 => {
                        let own = step == 2;
                        let id = if step == 10 {
                            36
                        } else {
                            30 + u128::from(step)
                        };
                        if !rows.iter().any(|row| row.local_id == id) {
                            rows.push(message(
                                id,
                                if own {
                                    "Fictional own projection"
                                } else {
                                    "Fictional teammate"
                                },
                                if own { "FO" } else { "FT" },
                                "10:01",
                                if own {
                                    "Own message projection fixture; no backend send or acceptance."
                                } else {
                                    "New incoming message projection fixture."
                                },
                                own,
                            ));
                        }
                    }
                    6 => {
                        if let Some(row) = rows.last_mut() {
                            row.body = text("Edited fictional row, same stable message ID.");
                            row.metadata.edited = true;
                        }
                    }
                    7 => self.snapshot.content = ContentState::Loading,
                    8 => self.snapshot.content = ContentState::Ready,
                    9 => rows.insert(
                        0,
                        message(
                            0,
                            "Fictional teammate",
                            "FT",
                            "09:00",
                            "Older loaded history fixture; no new-message entry.",
                            false,
                        ),
                    ),
                    11 => {
                        let mut event =
                            message(37, "", "", "10:02", "Fictional call ended.", false);
                        event.metadata.system = true;
                        rows.push(event);
                    }
                    12 => {
                        if let Some(row) = rows.iter_mut().find(|row| row.local_id == 36) {
                            row.body = text(
                                "Edited fictional message with a changed height.\n\
                                 The reader is already at the bottom.\n\
                                 The viewport must remain at the end.\n\
                                 This is a presentation fixture, not a server write.",
                            );
                            row.metadata.edited = true;
                        }
                    }
                    13 => {
                        if let Some(row) = rows.iter_mut().find(|row| row.local_id == 36) {
                            row.body = text("Fictional message removed.");
                            row.metadata.deleted = true;
                        }
                    }
                    14 if !rows.is_empty() => {
                        rows.remove(0);
                    }
                    _ => (),
                }
                self.snapshot.messages = page(rows);
                println!(
                    "UI_MESSAGE_FIXTURE step={step} rows={} own_projection={} no_backend=true",
                    self.snapshot.messages.as_slice().len(),
                    step == 2
                );
            }
            UiIntent::Refresh(_) if self.account_probe => {
                self.probe_step = (self.probe_step + 1).min(3);
                let revision = self.snapshot.revision;
                self.snapshot = account_snapshot(self.probe_step);
                self.snapshot.revision = revision;
            }
            UiIntent::SelectAccount(a) if self.account_probe => {
                if !self
                    .snapshot
                    .accounts
                    .as_slice()
                    .iter()
                    .any(|row| row.scope == a)
                {
                    return Err(SubmitError::StaleScope);
                }
                return Err(SubmitError::Unavailable(UnavailableReason::Unsupported));
            }
            UiIntent::Refresh(_) if self.focus_probe.is_some() => {
                self.probe_step = (self.probe_step + 1).min(3);
                let revision = self.snapshot.revision;
                self.snapshot = focus_snapshot(self.focus_probe.unwrap(), self.probe_step);
                self.snapshot.revision = revision;
            }
            UiIntent::OpenConversation(c) => {
                if !self
                    .snapshot
                    .conversations
                    .as_slice()
                    .iter()
                    .any(|r| r.context == c)
                {
                    return Err(SubmitError::StaleScope);
                }
                populate(&mut self.snapshot, c);
                if self.chat_ux {
                    configure_chat_ux(&mut self.snapshot);
                }
                if self.chat_content {
                    content::configure(&mut self.snapshot);
                }
            }
            UiIntent::Create {
                account,
                request,
                action: talk_core::ui::conversation::CreateAction::DirectMember(person),
            } if self.people_fixture => {
                if self.snapshot.active_account != Some(account)
                    || self.snapshot.selected != Some(person.context)
                    || !self
                        .snapshot
                        .conversation_panel
                        .as_ref()
                        .is_some_and(|panel| {
                            panel.context == person.context
                                && panel.members_loaded
                                && !panel.details_stale
                                && panel
                                    .members
                                    .as_slice()
                                    .iter()
                                    .any(|member| member.id == person && !member.self_user)
                        })
                {
                    return Err(SubmitError::StaleScope);
                }
                let direct = self
                    .snapshot
                    .conversations
                    .as_slice()
                    .iter()
                    .find(|row| {
                        row.kind == ConversationKind::Direct && row.title.as_str() == "Mara Fischer"
                    })
                    .map(|row| row.context)
                    .ok_or(SubmitError::StaleScope)?;
                populate(&mut self.snapshot, direct);
                let mut panel = talk_core::ui::conversation::CreatePanel::new(account);
                panel.result = Some(talk_core::ui::conversation::CreationResult {
                    request,
                    completion: Completion::ServerAccepted,
                    conversation: Some(direct),
                    incomplete_invitations: false,
                });
                self.snapshot.create_panel = Some(panel);
            }
            UiIntent::AcknowledgeCreation { account, request } if self.people_fixture => {
                if let Some(panel) = self.snapshot.create_panel.as_mut()
                    && panel.account == account
                    && panel.result.is_some_and(|result| result.request == request)
                {
                    panel.result = None;
                }
            }
            UiIntent::SetFavorite {
                context, favorite, ..
            } if self.chat_ux => {
                if Some(context) != self.snapshot.selected {
                    return Err(SubmitError::StaleScope);
                }
                let mut rows = self.snapshot.conversations.as_slice().to_vec();
                let row = rows
                    .iter_mut()
                    .find(|r| r.context == context)
                    .ok_or(SubmitError::StaleScope)?;
                row.favorite = Some(favorite);
                self.snapshot.conversations = page(rows);
                println!(
                    "UI_CHAT_UX favorite room={} value={favorite}",
                    context.conversation.local
                );
            }
            UiIntent::BeginLogin { request, .. } => {
                self.snapshot.login.phase = LoginPhase::BrowserWaiting;
                self.snapshot.login.attempt = Some(request);
            }
            UiIntent::NewLogin => {
                self.snapshot.login.phase = LoginPhase::EnterServer;
                self.snapshot.login.attempt = None;
            }
            UiIntent::CancelLogin(attempt) => {
                if self.snapshot.login.attempt != Some(attempt) {
                    return Err(SubmitError::StaleScope);
                }
                self.snapshot.login.phase = LoginPhase::EnterServer;
            }
            UiIntent::ReopenBrowser(_) | UiIntent::ChooseStorage { .. } => {
                return Err(SubmitError::Unavailable(UnavailableReason::Unconfigured));
            }
            UiIntent::Message {
                request,
                context,
                action: talk_core::ui::messaging::MessageAction::Send { body, target },
            } if self.snapshot.content == ContentState::Offline
                && self.thread_fixture.is_some() =>
            {
                if self.snapshot.selected != Some(context) || self.snapshot.submission.is_some() {
                    return Err(SubmitError::Busy);
                }
                let panel = self
                    .snapshot
                    .message_panel
                    .as_mut()
                    .ok_or(SubmitError::StaleScope)?;
                let root = target
                    .thread
                    .filter(|root| panel.thread_root == Some(*root))
                    .ok_or(SubmitError::StaleScope)?;
                let mut reply = message(42, "Alex Sommer", "AS", "10:03", body.as_str(), true);
                reply.metadata.thread = Some(root);
                reply.delivery = DeliveryState::Queued;
                panel
                    .thread_messages
                    .try_push(reply)
                    .map_err(|_| SubmitError::Busy)?;
                self.snapshot.messages = page(
                    self.snapshot
                        .messages
                        .as_slice()
                        .iter()
                        .cloned()
                        .map(|mut message| {
                            if message.local_id == root {
                                message.reply_count = message.reply_count.saturating_add(1);
                            }
                            message
                        }),
                );
                self.snapshot.submission = Some(SubmissionResult {
                    request,
                    context,
                    result: Completion::Queued,
                });
                println!("UI_THREAD_FIXTURE offline_reply_queued=true no_backend=true");
            }
            UiIntent::SendDraft {
                request, context, ..
            }
            | UiIntent::Message {
                request,
                context,
                action: talk_core::ui::messaging::MessageAction::Send { .. },
            } => {
                if self.busy || self.snapshot.submission.is_some() {
                    return Err(SubmitError::Busy);
                }
                if self.snapshot.selected != Some(context) {
                    return Err(SubmitError::StaleScope);
                }
                if !self.pending {
                    self.snapshot.submission = Some(SubmissionResult {
                        request,
                        context,
                        result: if self.uncertain {
                            Completion::Uncertain
                        } else {
                            Completion::Failed(UnavailableReason::Unconfigured)
                        },
                    });
                }
            }
            UiIntent::AcknowledgeSubmission { request, context } => {
                if self
                    .snapshot
                    .submission
                    .is_some_and(|r| r.request == request && r.context == context)
                {
                    self.snapshot.submission = None;
                }
            }
            UiIntent::Message {
                request,
                context,
                action: talk_core::ui::messaging::MessageAction::Older,
            } if self.history_pagination => {
                if Some(context) != self.snapshot.selected {
                    return Err(SubmitError::StaleScope);
                }
                let panel = self
                    .snapshot
                    .message_panel
                    .as_mut()
                    .filter(|panel| panel.context == context && panel.older == Some(101))
                    .ok_or(SubmitError::StaleScope)?;
                // One finite prepend through the real UI intent. This fixture
                // proves viewport behavior only, never server pagination.
                let mut rows: Vec<_> = (96..=100)
                    .map(|id| {
                        message(
                            id,
                            "Mara Fischer",
                            "MF",
                            "09:40",
                            "Ältere fiktive Nachricht für die lokale Navigation.",
                            false,
                        )
                    })
                    .collect();
                rows.extend_from_slice(self.snapshot.messages.as_slice());
                self.snapshot.messages = page(rows);
                panel.older = None;
                panel.history = talk_core::ui::conversation::RemoteState::Idle;
                println!(
                    "UI_HISTORY_FIXTURE request={} prepended=5 rows=45 no_backend=true",
                    request.0
                );
            }
            UiIntent::Message {
                request,
                context,
                action,
            } if self.chat_ux || self.chat_content => {
                use talk_core::ui::messaging::{MessageAction, SearchResult};
                if Some(context) != self.snapshot.selected {
                    return Err(SubmitError::StaleScope);
                }
                let offline = self.snapshot.content == ContentState::Offline;
                let panel = self
                    .snapshot
                    .message_panel
                    .as_mut()
                    .ok_or(SubmitError::StaleScope)?;
                match action {
                    MessageAction::MarkUnread | MessageAction::MarkUnreadFrom(_) => {
                        panel.result = Some((request, Completion::ServerAccepted));
                        self.snapshot.conversations =
                            page(self.snapshot.conversations.as_slice().iter().cloned().map(
                                |mut row| {
                                    if row.context == context {
                                        row.unread = row.unread.max(1);
                                    }
                                    row
                                },
                            ));
                    }
                    MessageAction::StageFiles(paths) => {
                        use talk_core::ui::messaging::{UploadState, UploadView};
                        let mut uploads = panel.uploads.as_slice().to_vec();
                        if uploads.len() + paths.as_slice().len() > 8 {
                            return Err(SubmitError::Busy);
                        }
                        for path in paths.as_slice() {
                            let name = std::path::Path::new(path.as_str())
                                .file_name()
                                .and_then(|name| name.to_str())
                                .unwrap_or("file");
                            uploads.push(UploadView {
                                id: 1000 + uploads.len() as u64,
                                name: text(name),
                                bytes: None,
                                state: UploadState::Selected,
                                submitted: false,
                                transferred: 0,
                                uploaded: false,
                                server_message: None,
                            });
                        }
                        panel.uploads = page(uploads);
                    }
                    MessageAction::StageClipboardImage { name, image } => {
                        use talk_core::ui::messaging::{UploadState, UploadView};
                        let mut uploads = panel.uploads.as_slice().to_vec();
                        if uploads.len() >= 8 {
                            return Err(SubmitError::Busy);
                        }
                        uploads.push(UploadView {
                            id: 1000 + uploads.len() as u64,
                            name,
                            bytes: Some(image.as_slice().len() as u64),
                            state: UploadState::Selected,
                            submitted: false,
                            transferred: 0,
                            uploaded: false,
                            server_message: None,
                        });
                        panel.uploads = page(uploads);
                    }
                    MessageAction::RemoveFile(id) => {
                        panel.uploads = page(
                            panel
                                .uploads
                                .as_slice()
                                .iter()
                                .filter(|file| file.id != id)
                                .cloned(),
                        )
                    }
                    MessageAction::SendFiles { caption, .. } => {
                        // Presentation-only pending state, never a fabricated upload receipt.
                        panel.files_queued = Some(request);
                        let mut first = true;
                        for file in panel.uploads.as_slice().iter().filter(|f| !f.submitted) {
                            let row = message(
                                file.message_id(),
                                "Fictional author",
                                "FA",
                                "10:00",
                                if first { caption.as_str() } else { "" },
                                true,
                            );
                            first = false;
                            let _ = self.snapshot.messages.try_push(row);
                        }
                        panel.uploads_running = true;
                        panel.uploads =
                            page(panel.uploads.as_slice().iter().cloned().map(|mut file| {
                                file.state = talk_core::ui::messaging::UploadState::Uploading;
                                file.submitted = true;
                                file
                            }));
                    }
                    MessageAction::Thread(root) if self.thread_fixture.is_some() => {
                        panel.thread_root = Some(root);
                        panel.thread_older = None;
                        panel.thread_has_newer = false;
                        panel.thread_state = if self.thread_fixture == Some(true) {
                            talk_core::ui::conversation::RemoteState::Failed(
                                request,
                                UnavailableReason::PolicyUnknown,
                            )
                        } else {
                            talk_core::ui::conversation::RemoteState::Idle
                        };
                        let mut row = message(
                            40,
                            "Mara Fischer",
                            "MF",
                            "10:00",
                            "Fictional thread reply for native input checks.",
                            false,
                        );
                        row.metadata.replyable = true;
                        row.metadata.thread = Some(root);
                        panel.thread_messages = page([row]);
                    }
                    MessageAction::ThreadOlder(root)
                        if self.thread_fixture.is_some()
                            && panel.thread_root == Some(root)
                            && panel.thread_older.is_some() =>
                    {
                        let mut row = message(
                            20,
                            "Jonas Weber",
                            "JW",
                            "09:55",
                            "Earlier fictional reply.",
                            false,
                        );
                        row.metadata.thread = Some(root);
                        panel.thread_messages = page([row]);
                        panel.thread_older = None;
                        panel.thread_has_newer = true;
                        panel.thread_state = talk_core::ui::conversation::RemoteState::Idle;
                    }
                    MessageAction::ThreadInfo(root) if self.thread_fixture.is_some() => {
                        panel.thread_details = Some(talk_core::ui::messaging::ThreadDetails {
                            root,
                            title: text("Design review"),
                            notification_level: Some(0),
                            rename: Decision::Allowed,
                        });
                    }
                    MessageAction::RenameThread { root, title }
                        if self.thread_fixture.is_some() =>
                    {
                        if let Some(details) = panel
                            .thread_details
                            .as_mut()
                            .filter(|details| details.root == root)
                        {
                            details.title = title;
                            panel.result = Some((
                                request,
                                if offline {
                                    Completion::Queued
                                } else {
                                    Completion::ServerAccepted
                                },
                            ));
                        }
                    }
                    MessageAction::ThreadNotifications { root, level }
                        if self.thread_fixture.is_some() =>
                    {
                        if let Some(details) = panel
                            .thread_details
                            .as_mut()
                            .filter(|details| details.root == root)
                        {
                            details.notification_level = Some(level);
                            panel.thread_following_intent = Some(level != 3);
                            panel.result = Some((
                                request,
                                if offline {
                                    Completion::Queued
                                } else {
                                    Completion::ServerAccepted
                                },
                            ));
                        }
                    }
                    MessageAction::Search(query) => {
                        let query = query.as_str().to_lowercase();
                        panel.search = page(
                            self.snapshot
                                .messages
                                .as_slice()
                                .iter()
                                .filter(|m| {
                                    !m.metadata.deleted
                                        && m.body.as_str().to_lowercase().contains(&query)
                                })
                                .map(|m| SearchResult {
                                    id: m.local_id,
                                    title: text(m.author.as_str()),
                                    excerpt: m.body.clone(),
                                }),
                        );
                        panel.search_loaded = true;
                        println!(
                            "UI_CHAT_UX search room={} results={}",
                            context.conversation.local,
                            panel.search.as_slice().len()
                        );
                    }
                    MessageAction::Context(id) => {
                        if !self
                            .snapshot
                            .messages
                            .as_slice()
                            .iter()
                            .any(|m| m.local_id == id)
                        {
                            return Err(SubmitError::StaleScope);
                        }
                        panel.located = Some((request, id));
                        println!(
                            "UI_CHAT_UX locate room={} message={id}",
                            context.conversation.local
                        );
                    }
                    MessageAction::LocateUnreadAfter(marker) => {
                        let id = self
                            .snapshot
                            .messages
                            .as_slice()
                            .iter()
                            .filter(|row| row.local_id > marker && !row.metadata.pin_notice)
                            .map(|row| row.local_id)
                            .min()
                            .ok_or(SubmitError::StaleScope)?;
                        panel.context_load = Some(talk_core::ui::messaging::ContextLoad {
                            target: marker,
                            state: talk_core::ui::conversation::RemoteState::Idle,
                        });
                        panel.located = Some((request, id));
                    }
                    MessageAction::Files | MessageAction::Pins => {
                        println!("UI_CHAT_UX shared room={}", context.conversation.local);
                    }
                    _ => return Err(SubmitError::Unavailable(UnavailableReason::Unsupported)),
                }
            }
            UiIntent::Legacy(UserIntent::PrepareConference(call)) => {
                if !self
                    .snapshot
                    .conversations
                    .as_slice()
                    .iter()
                    .any(|r| r.call_scope == Some(call) && r.kind != ConversationKind::Direct)
                {
                    return Err(SubmitError::StaleScope);
                }
            }
            UiIntent::Legacy(UserIntent::AcceptDirect { call, .. }) => {
                let Some(view) = self.snapshot.call.as_mut() else {
                    return Err(SubmitError::StaleScope);
                };
                if view.scope != call || view.stage != CallStage::Incoming || !view.direct {
                    return Err(SubmitError::StaleScope);
                }
                view.stage = CallStage::Connecting;
                populate(
                    &mut self.snapshot,
                    ConversationRef {
                        account: call.account,
                        conversation: call.conversation,
                    },
                );
            }
            UiIntent::StartDirect { call, .. }
            | UiIntent::JoinConference { call, .. }
            | UiIntent::Legacy(UserIntent::JoinPrepared { call, .. }) => {
                let Some(row) = self
                    .snapshot
                    .conversations
                    .as_slice()
                    .iter()
                    .find(|r| r.call_scope == Some(call))
                else {
                    return Err(SubmitError::StaleScope);
                };
                let title = row.title.clone();
                let direct = row.kind == ConversationKind::Direct;
                let context = row.context;
                populate(&mut self.snapshot, context);
                self.snapshot.call = Some(CallView {
                    scope: call,
                    title,
                    direct,
                    stage: CallStage::Connecting,
                    accept: Decision::Unknown,
                    microphone: MediaState::Unavailable,
                    camera: MediaState::Unavailable,
                    share: MediaState::Unavailable,
                    sharing: ScreenShareState::Idle,
                    streams: Page::empty(),
                    recording: talk_core::ui::presentation::CallRecordingState::Unknown,
                    recording_control: None,
                    recording_pending: false,
                    recording_uncertain: false,
                    hand_supported: false,
                    hand_raised: false,
                    reactions: Page::empty(),
                    latest_reaction: None,
                    invitees_loaded: false,
                    invitees: Page::empty(),
                    ring_status: None,
                    mute_status: None,
                    can_end_everyone: false,
                    end_pending: false,
                    end_failed: false,
                    participants: if direct || call.conversation.local == 2 {
                        page([participants()[0].clone(), participants()[2].clone()])
                    } else {
                        page(participants())
                    },
                });
            }
            UiIntent::SetCallRecording { call, start } => {
                let view = self
                    .snapshot
                    .call
                    .as_mut()
                    .filter(|view| {
                        view.scope == call
                            && view.recording_control == Some(start)
                            && !view.recording_pending
                            && !view.recording_uncertain
                    })
                    .ok_or(SubmitError::StaleScope)?;
                view.recording_pending = true;
            }
            UiIntent::EndCallForEveryone(call) => {
                let view = self
                    .snapshot
                    .call
                    .as_mut()
                    .filter(|view| view.scope == call && view.can_end_everyone)
                    .ok_or(SubmitError::StaleScope)?;
                view.end_pending = true;
            }
            UiIntent::MuteCallParticipant { call, target } => {
                let view = self
                    .snapshot
                    .call
                    .as_mut()
                    .filter(|view| {
                        view.scope == call
                            && view
                                .participants
                                .as_slice()
                                .iter()
                                .any(|person| person.local_id == target && person.call_can_mute)
                    })
                    .ok_or(SubmitError::StaleScope)?;
                view.mute_status = Some(talk_core::ui::presentation::CallRingStatus {
                    target,
                    outcome: talk_core::ui::presentation::CallRingOutcome::Pending,
                });
            }
            UiIntent::RingCallParticipant { call, target } => {
                let view = self
                    .snapshot
                    .call
                    .as_mut()
                    .filter(|view| {
                        view.scope == call
                            && view
                                .invitees
                                .as_slice()
                                .iter()
                                .any(|person| person.local_id == target && person.ring_allowed)
                    })
                    .ok_or(SubmitError::StaleScope)?;
                view.ring_status = Some(talk_core::ui::presentation::CallRingStatus {
                    target,
                    outcome: talk_core::ui::presentation::CallRingOutcome::Pending,
                });
            }
            UiIntent::ResolveRequiredGate {
                call,
                gate: RequiredGate::RecordingConsent,
            } => {
                let view = self
                    .snapshot
                    .call
                    .as_mut()
                    .filter(|view| {
                        view.scope == call
                            && view.stage == CallStage::Required(RequiredGate::RecordingConsent)
                    })
                    .ok_or(SubmitError::StaleScope)?;
                // A fixture may advance to connecting; it never grants media.
                view.stage = CallStage::Connecting;
            }
            UiIntent::SetCallHand { call, raised } => {
                let view = self
                    .snapshot
                    .call
                    .as_mut()
                    .filter(|view| view.scope == call && view.hand_supported)
                    .ok_or(SubmitError::StaleScope)?;
                view.hand_raised = raised;
            }
            UiIntent::SendCallReaction { call, emoji } => {
                let view = self
                    .snapshot
                    .call
                    .as_mut()
                    .filter(|view| view.scope == call && view.reactions.as_slice().contains(&emoji))
                    .ok_or(SubmitError::StaleScope)?;
                let sequence = view
                    .latest_reaction
                    .as_ref()
                    .map_or(Some(1), |reaction| reaction.sequence.checked_add(1))
                    .ok_or(SubmitError::Busy)?;
                view.latest_reaction = Some(talk_core::ui::presentation::CallReactionView {
                    sequence,
                    participant: None,
                    emoji,
                });
            }
            UiIntent::Legacy(UserIntent::Leave(call)) | UiIntent::DeclineDirect(call) => {
                if self.snapshot.call.as_ref().is_none_or(|c| c.scope != call) {
                    return Err(SubmitError::StaleScope);
                }
                self.snapshot.call = None;
            }
            UiIntent::Conversation {
                context,
                action: talk_core::ui::conversation::ConversationAction::Read,
                ..
            } if self
                .snapshot
                .conversation_panel
                .as_ref()
                .is_some_and(|p| p.context == context) =>
            {
                // Only the explicit details-ready scenario supplies this panel.
                // No server or write is simulated by this layout fixture.
            }
            _ => return Err(SubmitError::Unavailable(UnavailableReason::Unsupported)),
        }
        self.snapshot.revision += 1;
        self.dirty = true;
        Ok(())
    }
    fn take_snapshot(&mut self) -> Option<PresentationSnapshot> {
        if self.dirty {
            self.dirty = false;
            Some(self.snapshot.clone())
        } else {
            None
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use talk_core::ui::MediaIntent;

    #[test]
    fn person_fixture_resolves_only_a_current_member_to_the_direct_room() {
        let mut port = DevelopmentPort::new(Some("people-card"));
        let account = port.snapshot.active_account.unwrap();
        let person = port
            .snapshot
            .conversation_panel
            .as_ref()
            .unwrap()
            .members
            .as_slice()[0]
            .id;
        let mut stale = person;
        stale.local ^= 1;
        let request = |member| UiIntent::Create {
            account,
            request: RequestId(71),
            action: talk_core::ui::conversation::CreateAction::DirectMember(member),
        };
        assert_eq!(
            port.try_submit(request(stale)),
            Err(SubmitError::StaleScope)
        );
        assert_eq!(port.try_submit(request(person)), Ok(()));
        let direct = port.snapshot.selected.unwrap();
        assert_eq!(
            port.snapshot
                .create_panel
                .as_ref()
                .unwrap()
                .result
                .unwrap()
                .conversation,
            Some(direct)
        );
        assert_eq!(
            port.snapshot
                .conversations
                .as_slice()
                .iter()
                .find(|row| row.context == direct)
                .unwrap()
                .kind,
            ConversationKind::Direct
        );
    }
    #[test]
    fn reaction_fixture_reconciles_counts_and_keeps_pending_and_denied_distinct() {
        use talk_core::ui::{conversation::RemoteState, messaging::MessageAction};
        let mut fixture = DevelopmentPort::new(Some("chat-reactions"));
        let context = fixture.snapshot.selected.unwrap();
        let intent = |request, add| UiIntent::Message {
            request: RequestId(request),
            context,
            action: MessageAction::React {
                message: 4,
                emoji: text("👍"),
                add,
            },
        };
        assert_eq!(fixture.try_submit(intent(1, false)), Ok(()));
        let reaction = &fixture.snapshot.messages.as_slice()[3]
            .metadata
            .reactions
            .as_slice()[0];
        assert_eq!((reaction.count, reaction.own), (2, false));
        assert_eq!(fixture.try_submit(intent(2, true)), Ok(()));
        let reaction = &fixture.snapshot.messages.as_slice()[3]
            .metadata
            .reactions
            .as_slice()[0];
        assert_eq!((reaction.count, reaction.own), (3, true));
        let mut pending = DevelopmentPort::new(Some("chat-reactions-pending"));
        assert_eq!(pending.try_submit(intent(3, false)), Ok(()));
        assert_eq!(
            pending.snapshot.message_panel.as_ref().unwrap().state,
            RemoteState::Saving(RequestId(3))
        );
        assert_eq!(pending.try_submit(intent(4, false)), Err(SubmitError::Busy));
        let mut denied = DevelopmentPort::new(Some("chat-reactions-denied"));
        assert_eq!(
            denied.try_submit(intent(5, true)),
            Err(SubmitError::Unavailable(
                UnavailableReason::PermissionDenied
            ))
        );
    }
    #[test]
    fn live_account_focus_probe_never_reuses_a_runtime_on_removal_or_switch() {
        let original = account_snapshot(0);
        let reordered = account_snapshot(1);
        let removed = account_snapshot(2);
        let replaced = account_snapshot(3);
        let second = original.accounts.as_slice()[1].scope;
        assert_eq!(reordered.accounts.as_slice()[2].scope, second);
        assert!(
            !removed
                .accounts
                .as_slice()
                .iter()
                .any(|a| a.scope == second)
        );
        assert_ne!(
            original.accounts.as_slice()[2].scope,
            replaced.accounts.as_slice()[1].scope
        );
        for s in [&original, &reordered, &removed, &replaced] {
            assert!(
                s.conversations
                    .as_slice()
                    .iter()
                    .all(|r| Some(r.context.account) == s.active_account)
            );
            assert!(
                s.accounts
                    .as_slice()
                    .iter()
                    .any(|a| Some(a.scope) == s.active_account)
            );
        }
        let mut port = DevelopmentPort::new(Some("focus-accounts"));
        for _ in 0..3 {
            port.try_submit(UiIntent::Refresh(context(1))).unwrap();
        }
        assert_eq!(
            port.try_submit(UiIntent::SelectAccount(second)),
            Err(SubmitError::StaleScope)
        );
    }
    #[test]
    fn explicit_focus_projection_transitions_are_bounded_and_scope_consistent() {
        for files in [false, true] {
            for step in 0..4 {
                let s = focus_snapshot(files, step);
                assert!(
                    s.conversations
                        .as_slice()
                        .iter()
                        .all(|r| Some(r.context.account) == s.active_account)
                );
                assert!(s.selected.is_some_and(|selected| {
                    s.conversations
                        .as_slice()
                        .iter()
                        .any(|r| r.context == selected)
                }));
                let rows = s.conversations.as_slice();
                assert!(rows.len() <= 6);
                if !files && step == 1 {
                    assert_eq!(rows[4].context.conversation.local, 3);
                }
                if !files && step >= 2 {
                    assert!(rows.iter().all(|r| r.context.conversation.local != 3));
                }
                if files {
                    assert_eq!(
                        s.files.as_slice()[0].local_id,
                        if step == 0 { 1 } else { 2 }
                    );
                }
                if step == 3 {
                    assert_eq!(s.active_account.unwrap().account.as_local(), 2);
                }
            }
        }
    }
    #[test]
    fn no_scenario_never_simulates_authorization_or_messages() {
        let mut port = DevelopmentPort::new(None);
        let initial = port.take_snapshot().unwrap();
        assert!(initial.accounts.as_slice().is_empty());
        assert_eq!(
            port.try_submit(UiIntent::BeginLogin {
                request: RequestId(1),
                server: talk_core::ui::presentation::ServerInput::new("example.test").unwrap()
            }),
            Err(SubmitError::Unavailable(UnavailableReason::Unconfigured))
        );
        assert!(port.take_snapshot().is_none());
    }
    #[test]
    fn direct_accept_moves_only_to_connecting_and_has_no_capture() {
        let mut port = DevelopmentPort::new(Some("incoming"));
        let initial = port.take_snapshot().unwrap();
        let call = initial.call.unwrap().scope;
        let mut stale = call;
        stale.call = CallGeneration::from_local(2);
        assert_eq!(
            port.try_submit(UiIntent::Legacy(UserIntent::AcceptDirect {
                call: stale,
                media: MediaIntent::MicrophoneOnCameraOff
            })),
            Err(SubmitError::StaleScope)
        );
        assert!(port.take_snapshot().is_none());
        port.try_submit(UiIntent::Legacy(UserIntent::AcceptDirect {
            call,
            media: MediaIntent::MicrophoneOnCameraOff,
        }))
        .unwrap();
        let result = port.take_snapshot().unwrap().call.unwrap();
        assert_eq!(result.stage, CallStage::Connecting);
        assert_eq!(result.camera, MediaState::Unavailable);
        assert_eq!(result.microphone, MediaState::Unavailable);
        assert!(port.take_snapshot().is_none());
    }
    #[test]
    fn busy_has_no_completion_and_local_admission_never_fabricates_history() {
        let mut busy = DevelopmentPort::new(Some("busy"));
        busy.take_snapshot();
        let intent = UiIntent::SendDraft {
            request: RequestId(7),
            context: context(1),
            body: text("Test draft"),
        };
        assert_eq!(busy.try_submit(intent.clone()), Err(SubmitError::Busy));
        assert!(busy.take_snapshot().is_none());
        let mut port = DevelopmentPort::new(Some("chat"));
        let before = port.take_snapshot().unwrap();
        port.try_submit(intent).unwrap();
        let result = port.take_snapshot().unwrap();
        assert_eq!(before.messages, result.messages);
        assert_eq!(
            result.submission.unwrap().result,
            Completion::Failed(UnavailableReason::Unconfigured)
        );
        port.try_submit(UiIntent::AcknowledgeSubmission {
            request: RequestId(6),
            context: context(1),
        })
        .unwrap();
        assert!(port.take_snapshot().unwrap().submission.is_some());
        port.try_submit(UiIntent::AcknowledgeSubmission {
            request: RequestId(7),
            context: context(1),
        })
        .unwrap();
        assert!(port.take_snapshot().unwrap().submission.is_none());
    }
    #[test]
    fn conference_preparation_requires_matching_group_scope() {
        let mut port = DevelopmentPort::new(Some("chat"));
        port.take_snapshot();
        assert_eq!(
            port.try_submit(UiIntent::Legacy(UserIntent::PrepareConference(call_scope(
                3
            )))),
            Err(SubmitError::StaleScope)
        );
        port.try_submit(UiIntent::Legacy(UserIntent::PrepareConference(call_scope(
            1,
        ))))
        .unwrap();
        assert!(port.take_snapshot().unwrap().call.is_none());
    }
    #[test]
    fn fixture_screen_stage_uses_stream_identity_and_metadata() {
        let mut snapshot = snapshot(Some("call-screen-stage"));
        let call = snapshot.call.as_mut().unwrap();
        let screen = call.screen_stage().unwrap();
        assert_eq!(screen.tile.local_id, 9001);
        assert_eq!(screen.dimensions, Some((1280, 720)));
        assert!(!screen.flowing);
        assert!(
            call.participants
                .as_slice()
                .iter()
                .all(|p| p.local_id != screen.tile.local_id)
        );
        let participant = call.participants.as_slice()[0].local_id;
        let mut streams = call.streams.as_slice().to_vec();
        let camera = streams
            .iter_mut()
            .find(|s| s.participant == participant && s.kind == CallTileKind::Camera)
            .unwrap();
        camera.local_id = 9002;
        call.streams = Page::empty();
        for stream in streams {
            call.streams.try_push(stream).unwrap();
        }
        assert_eq!(
            call.camera_tile(participant),
            Some(talk_core::ui::presentation::CallTileId {
                local_id: 9002,
                kind: CallTileKind::Camera
            })
        );
        call.streams = talk_core::ui::Page::empty();
        assert!(call.screen_stage().is_none());
        assert!(call.camera_tile(participant).is_none());
    }
    #[test]
    fn offline_thread_fixture_projects_the_locally_queued_reply() {
        use talk_core::ui::messaging::{ComposeTarget, MessageAction};
        let mut fixture = DevelopmentPort::new(Some("thread-offline"));
        let context = fixture.snapshot.selected.unwrap();
        let request = RequestId(70);
        assert_eq!(
            fixture.try_submit(UiIntent::Message {
                request,
                context,
                action: MessageAction::Send {
                    body: text("Offline reply"),
                    target: ComposeTarget {
                        thread: Some(1),
                        ..Default::default()
                    }
                },
            }),
            Ok(())
        );
        let snapshot = fixture.take_snapshot().unwrap();
        assert_eq!(snapshot.submission.unwrap().result, Completion::Queued);
        let row = snapshot
            .message_panel
            .unwrap()
            .thread_messages
            .as_slice()
            .last()
            .unwrap()
            .clone();
        assert_eq!(row.body.as_str(), "Offline reply");
        assert_eq!(row.metadata.thread, Some(1));
        assert_eq!(row.delivery, DeliveryState::Queued);
        assert_eq!(
            snapshot
                .messages
                .as_slice()
                .iter()
                .find(|row| row.local_id == 1)
                .unwrap()
                .reply_count,
            3
        );
    }
}
