//! Explicit development scenarios only. No server, credential or media operation occurs.
mod content;
mod dialog;
pub(super) use content::images as content_images;
use talk_core::ui::presentation::{
    AccountView, CallStage, CallView, Completion, ContentState, ConversationKind, ConversationRef,
    ConversationView, DeliveryState, FileView, LoginFailure, LoginPhase, LoginView, MediaState,
    NativeDesktopPort, NativeStoreState, ParticipantView, Presence, PresentationSnapshot,
    RequestId, RequiredGate, StorageStatus, SubmissionResult, UiIntent,
};
use talk_core::ui::{
    AccountId, AccountScope, CallGeneration, CallScope, ConversationId, Decision, HostId, Page,
    RuntimeGeneration, SubmitError, Text, UnavailableReason, UserIntent,
};

pub const SCENARIOS: &[&str] = &[
    "server",
    "validating",
    "browser",
    "storage",
    "ready",
    "expired",
    "login-network",
    "login-tls",
    "login-denied",
    "login-browser-error",
    "login-storage-error",
    "storage-locked",
    "storage-denied",
    "storage-saving",
    "storage-cleanup",
    "storage-recovery",
    "chat",
    "chat-ux",
    "chat-ux-call",
    "chat-edit",
    "thread-error",
    "chat-content",
    "chat-content-progress",
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
    "create34",
    "create35",
    "discover",
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
    "gate-lobby",
    "connecting",
    "reconnecting",
    "call-ended",
    "call-error",
    "call-error-unknown",
    "call-controls",
    "call-active",
    "call-requested",
    "sharing",
    "call-denied",
    "room",
    "voice-room",
    "prejoin-voice",
    "offline",
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
    if scenario == Some("details-ready") {
        let mut state = snapshot(Some("chat"));
        dialog::configure(&mut state);
        return state;
    }
    if matches!(scenario, Some("chat-content" | "chat-content-progress")) {
        let mut state = snapshot(Some("chat-ux"));
        content::configure(&mut state);
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
        create_panel: None,
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
        conversation_list: talk_core::ui::presentation::ConversationListState {
            content: ContentState::Unconfigured,
            complete: false,
        },
        tags: Page::empty(),
        tag_state: ContentState::Unconfigured,
        organization_result: None,
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
    s.login.attempt = Some(RequestId(1));
    s.login.phase = match scenario {
        "server" => LoginPhase::EnterServer,
        "validating" => LoginPhase::Validating,
        "browser" => LoginPhase::BrowserWaiting,
        "storage" | "storage-locked" | "storage-denied" | "storage-saving" | "storage-cleanup"
        | "storage-recovery" => LoginPhase::ChooseStorage,
        "login-network" => LoginPhase::Failed(LoginFailure::Network),
        "login-tls" => LoginPhase::Failed(LoginFailure::Tls),
        "login-denied" => LoginPhase::Failed(LoginFailure::Denied),
        "login-browser-error" => LoginPhase::Failed(LoginFailure::BrowserUnavailable),
        "login-storage-error" => LoginPhase::Failed(LoginFailure::Storage),
        "expired" => LoginPhase::Failed(LoginFailure::Expired),
        _ => LoginPhase::Ready,
    };
    s.login.native_store = match scenario {
        "storage-locked" => NativeStoreState::Locked,
        "storage-denied" => NativeStoreState::Denied,
        _ => NativeStoreState::Missing,
    };
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
            | "login-tls"
            | "login-denied"
            | "login-browser-error"
            | "login-storage-error"
    ) {
        return s;
    }
    s.accounts = page([AccountView {
        scope: scope(),
        navigation_key: None,
        connection: talk_core::ui::presentation::AccountConnectionState::Disconnected,
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
                presence: if kind == ConversationKind::Direct {
                    Presence::Available
                } else {
                    Presence::Unknown
                },
                participants: Some(if kind == ConversationKind::Direct {
                    2
                } else {
                    7
                }),
                send: Decision::Allowed,
                attach: Decision::Unknown,
                draft_clipboard: Decision::Allowed,
                call: if kind == ConversationKind::NoteToSelf {
                    Decision::Denied
                } else {
                    Decision::Allowed
                },
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
        context(if matches!(scenario, "direct-call" | "outgoing") {
            3
        } else if matches!(scenario, "voice" | "voice-room" | "prejoin-voice") {
            2
        } else if scenario == "room" {
            4
        } else {
            1
        }),
    );
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
            | "gate-lobby"
            | "connecting"
            | "reconnecting"
            | "call-ended"
            | "call-error"
            | "call-error-unknown"
            | "call-controls"
            | "sharing"
            | "call-denied"
            | "call-active"
            | "call-requested"
    ) {
        let direct = matches!(scenario, "incoming" | "outgoing" | "direct-call");
        let id = if direct {
            3
        } else if scenario == "voice" {
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
                "gate-recording" => CallStage::Required(RequiredGate::RecordingConsent),
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
            participants: page(if direct || id == 2 {
                vec![participants()[0].clone(), participants()[2].clone()]
            } else {
                participants().to_vec()
            }),
        });
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
        if row.local_id == 2 {
            row.metadata.deleted = true;
        }
        if row.local_id == 3 {
            row.delivery = DeliveryState::Pending;
        }
    }
    let mut panel = MessagePanel::new(context);
    panel.features = page(
        [
            MessageFeature::Reply,
            MessageFeature::Search,
            MessageFeature::Files,
            MessageFeature::Pins,
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
    thread_fixture: Option<bool>,
    snapshot: PresentationSnapshot,
    dirty: bool,
    enabled: bool,
    busy: bool,
    uncertain: bool,
    pending: bool,
    intents: u8,
    chat_ux_send_diagnostics: u8,
    account_probe: bool,
    focus_probe: Option<bool>, // false: rooms; true: files
    probe_step: u8,
    chat_ux: bool,
    chat_motion: bool,
    history_pagination: bool,
    chat_content: bool,
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
            thread_fixture: match scenario {
                Some("chat-edit") => Some(false),
                Some("thread-error") => Some(true),
                _ => None,
            },
            account_probe,
            focus_probe,
            probe_step: 0,
            chat_ux: matches!(
                scenario,
                Some("chat-ux" | "chat-ux-call" | "chat-edit" | "thread-error")
            ),
            chat_motion: scenario == Some("chat-motion"),
            history_pagination: scenario == Some("history-pagination"),
            chat_content: matches!(scenario, Some("chat-content" | "chat-content-progress")),
            snapshot: initial,
            dirty: true,
            enabled: scenario.is_some(),
            busy: matches!(scenario, Some("busy" | "history-busy")),
            uncertain: matches!(scenario, Some("uncertain" | "history-uncertain")),
            pending: matches!(scenario, Some("pending" | "history-pending")),
            intents: 0,
            chat_ux_send_diagnostics: 0,
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
        match intent {
            UiIntent::Content {
                account, action, ..
            } if self.chat_content => {
                if self.snapshot.active_account != Some(account) {
                    return Err(SubmitError::StaleScope);
                }
                content::action(&mut self.snapshot, action)?;
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
            UiIntent::CancelLogin(attempt) => {
                if self.snapshot.login.attempt != Some(attempt) {
                    return Err(SubmitError::StaleScope);
                }
                self.snapshot.login.phase = LoginPhase::EnterServer;
            }
            UiIntent::ReopenBrowser(_) | UiIntent::ChooseStorage { .. } => {
                return Err(SubmitError::Unavailable(UnavailableReason::Unconfigured));
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
            } if self.chat_ux => {
                use talk_core::ui::messaging::{MessageAction, SearchResult};
                if Some(context) != self.snapshot.selected {
                    return Err(SubmitError::StaleScope);
                }
                let panel = self
                    .snapshot
                    .message_panel
                    .as_mut()
                    .ok_or(SubmitError::StaleScope)?;
                match action {
                    MessageAction::Thread(root) if self.thread_fixture.is_some() => {
                        panel.thread_root = Some(root);
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
                    participants: if direct || call.conversation.local == 2 {
                        page([participants()[0].clone(), participants()[2].clone()])
                    } else {
                        page(participants())
                    },
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
}
