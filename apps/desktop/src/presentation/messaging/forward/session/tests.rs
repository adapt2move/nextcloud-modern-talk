use super::*;
use talk_core::ui::conversation::{Availability, RemoteState};
use talk_core::ui::messaging::{MessageMetadata, MessagePanel, Reaction, RichObject};
use talk_core::ui::presentation::{
    AccountConnectionState, AccountView, ConversationKind, ConversationListState,
    ConversationOrganization, FeatureSupport, FileView, LoginPhase, LoginView, NativeStoreState,
    Presence, RequestId, StorageStatus,
};
use talk_core::ui::{
    AccountId, AccountScope, ConversationId, HostId, Page, RuntimeGeneration, Text,
};

fn room(local: u128) -> ConversationRef {
    let account = AccountId::from_local(1);
    ConversationRef {
        account: AccountScope {
            account,
            runtime: RuntimeGeneration::from_local(1),
        },
        conversation: ConversationId {
            account,
            host: HostId::from_local(1),
            local,
        },
    }
}
fn page<T, const N: usize>(values: impl IntoIterator<Item = T>) -> Page<T, N> {
    let mut page = Page::empty();
    for value in values {
        assert!(page.try_push(value).is_ok());
    }
    page
}
fn message() -> MessageView {
    MessageView {
        local_id: 7,
        author: Text::new("Synthetic sender").unwrap(),
        initials: Text::new("S").unwrap(),
        time_label: Text::new("10:00").unwrap(),
        body: Text::new("The exact source message").unwrap(),
        own: false,
        delivery: DeliveryState::Received,
        attachments: Page::empty(),
        reply_count: 0,
        metadata: MessageMetadata {
            timestamp: 1,
            author_key: Some(Text::new("synthetic-sender").unwrap()),
            ..Default::default()
        },
    }
}
fn snapshot() -> PresentationSnapshot {
    let source = room(1);
    let row = |context| ConversationView {
        context,
        navigation_key: None,
        title: Text::new("Synthetic conversation").unwrap(),
        initials: Text::new("S").unwrap(),
        subtitle: Text::new("").unwrap(),
        updated_label: Text::new("").unwrap(),
        kind: ConversationKind::Group,
        unread: 0,
        favorite: None,
        organization: ConversationOrganization {
            classified: Some(false),
            ..Default::default()
        },
        tags: None,
        archived: Some(false),
        archive: Decision::Unknown,
        presence: Presence::Unknown,
        participants: None,
        send: Decision::Allowed,
        attach: Decision::Denied,
        draft_clipboard: Decision::Allowed,
        call: Decision::Denied,
        call_scope: None,
    };
    let mut panel = MessagePanel::new(source);
    panel.features = page([(
        MessageFeature::Forward,
        Availability {
            implemented: true,
            server: FeatureSupport::Supported,
            allowed: Decision::Allowed,
        },
    )]);
    PresentationSnapshot {
        content_panel: Default::default(),
        create_panel: None,
        revision: 1,
        login: LoginView {
            phase: LoginPhase::Ready,
            attempt: None,
            server_label: Text::new("").unwrap(),
            account_label: Text::new("").unwrap(),
            native_store: NativeStoreState::Unknown,
            storage: StorageStatus::NotSaved,
            storage_failure: None,
        },
        accounts: page([AccountView {
            scope: source.account,
            navigation_key: None,
            connection: AccountConnectionState::Connected,
            connection_failure: None,
            label: Text::new("Synthetic account").unwrap(),
            server_label: Text::new("example.invalid").unwrap(),
            self_name: Text::new("Synthetic self").unwrap(),
            initials: Text::new("S").unwrap(),
            presence: Presence::Available,
            storage: StorageStatus::NotSaved,
        }]),
        saved_accounts: Page::empty(),
        account_removal: None,
        active_account: Some(source.account),
        conversations: page([row(source), row(room(2))]),
        conversation_list: ConversationListState {
            content: ContentState::Ready,
            complete: true,
        },
        tags: Page::empty(),
        tag_state: ContentState::Ready,
        organization_result: None,
        selected: Some(source),
        content: ContentState::Ready,
        messages: page([message()]),
        files: Page::empty(),
        participants: Page::empty(),
        conversation_panel: None,
        message_panel: Some(panel),
        call: None,
        submission: None,
        mutation: None,
    }
}
fn opened(snapshot: &PresentationSnapshot) -> (Session, Target, Confirmation) {
    let mut session = Session::default();
    let target = session.open(room(1), &message()).unwrap();
    assert_eq!(session.target(), Some(target));
    assert!(session.source_current(snapshot));
    assert!(session.select(target, room(2)));
    (
        session,
        target,
        Confirmation {
            target,
            destination: room(2),
        },
    )
}

#[test]
fn selection_requires_confirmation_and_old_callbacks_cannot_act_on_reopened_source() {
    let snapshot = snapshot();
    let mut session = Session::default();
    let target = session.open(room(1), &message()).unwrap();
    let confirmation = Confirmation {
        target,
        destination: room(2),
    };
    assert_eq!(
        session.confirm(confirmation, &snapshot),
        Err(Denial::Selection)
    );
    assert!(session.select(target, room(2)));
    assert_eq!(session.confirm(confirmation, &snapshot), Ok(()));
    session.close();
    assert_eq!(session.target(), None);
    assert_eq!(
        session.confirm(confirmation, &snapshot),
        Err(Denial::Source)
    );
    let new_target = session.open(room(1), &message()).unwrap();
    assert_ne!(new_target, target);
    assert!(!session.select(target, room(2)));
    assert_eq!(
        session.confirm(confirmation, &snapshot),
        Err(Denial::Source)
    );
    assert!(session.active.as_ref().unwrap().destination.is_none());
    assert!(session.select(new_target, room(2)));
    assert_eq!(
        session.confirm(confirmation, &snapshot),
        Err(Denial::Source)
    );
    session.generation = u64::MAX;
    assert!(session.open(room(1), &message()).is_none());
    assert!(session.active.is_none());
}

#[test]
fn accepted_policy_loss_retires_selection_before_restoration_without_a_render() {
    let mut current = snapshot();
    let (mut session, target, confirmation) = opened(&current);
    let mut denied = current.clone();
    denied.revision += 1;
    denied.message_panel.as_mut().unwrap().features = page([(
        MessageFeature::Forward,
        Availability {
            implemented: true,
            server: FeatureSupport::Supported,
            allowed: Decision::Denied,
        },
    )]);
    let mut restored = current.clone();
    restored.revision = denied.revision + 1;

    // The accepted-snapshot hook reconciles each projection immediately. No
    // paint or activation occurs between the policy loss and its restoration.
    for accepted in [denied, restored] {
        assert!(accepted.revision > current.revision);
        current = accepted;
        session.reconcile(&current);
    }

    assert!(source(&current, room(1), target.message).is_some());
    assert_eq!(session.target(), None);
    assert!(!session.select(target, room(2)));
    assert_eq!(session.confirm(confirmation, &current), Err(Denial::Source));
    let reopened = session.open(room(1), &message()).unwrap();
    assert_ne!(reopened, target);
    assert!(session.active.as_ref().unwrap().destination.is_none());
}

#[test]
fn source_changes_retire_approval_even_if_later_projection_resembles_original() {
    for changed in [
        {
            let mut m = message();
            m.body = Text::new("New source text").unwrap();
            m
        },
        {
            let mut m = message();
            m.metadata.edited = true;
            m
        },
        {
            let mut m = message();
            m.metadata.deleted = true;
            m
        },
        {
            let mut m = message();
            m.author = Text::new("Different author").unwrap();
            m
        },
        {
            let mut m = message();
            m.metadata.author_key = None;
            m
        },
        {
            let mut m = message();
            m.metadata.timestamp = 2;
            m
        },
    ] {
        let mut snapshot = snapshot();
        let (mut session, _, confirmation) = opened(&snapshot);
        snapshot.messages = page([changed]);
        assert_eq!(
            session.confirm(confirmation, &snapshot),
            Err(Denial::Source)
        );
        session.reconcile(&snapshot);
        snapshot.messages = page([message()]);
        assert_eq!(
            session.confirm(confirmation, &snapshot),
            Err(Denial::Source)
        );
    }
    let mut snapshot = snapshot();
    let (mut session, _, confirmation) = opened(&snapshot);
    snapshot.messages = Page::empty();
    session.reconcile(&snapshot);
    assert_eq!(
        session.confirm(confirmation, &snapshot),
        Err(Denial::Source)
    );
}

#[test]
fn body_must_be_complete_plain_text_without_hidden_attachments() {
    let mut snapshot = snapshot();
    let (session, _, confirmation) = opened(&snapshot);
    for changed in [
        {
            let mut m = message();
            m.metadata.truncated = true;
            m
        },
        {
            let mut m = message();
            m.metadata.system = true;
            m
        },
        {
            let mut m = message();
            m.delivery = DeliveryState::Pending;
            m
        },
        {
            let mut m = message();
            m.attachments = page([FileView {
                local_id: 1,
                name: Text::new("fixture.txt").unwrap(),
                detail: Text::new("").unwrap(),
                download: Decision::Allowed,
            }]);
            m
        },
        {
            let mut m = message();
            m.metadata.rich = page([RichObject {
                id: None,
                mime_type: None,
                size: None,
                placeholder: Text::new("file").unwrap(),
                kind: Text::new("file").unwrap(),
                name: Text::new("fixture.txt").unwrap(),
                link: None,
            }]);
            m
        },
    ] {
        snapshot.messages = page([changed]);
        assert_eq!(
            session.confirm(confirmation, &snapshot),
            Err(Denial::Source)
        );
    }
    // Reaction updates are unrelated to the content the user approved.
    let mut current = message();
    current.metadata.reactions = page([Reaction {
        emoji: Text::new("👍").unwrap(),
        count: 2,
        own: false,
    }]);
    snapshot.messages = page([current]);
    assert_eq!(session.confirm(confirmation, &snapshot), Ok(()));
}

#[test]
fn account_host_room_runtime_and_exposure_changes_never_keep_authority() {
    let baseline = snapshot();
    let (session, target, confirmation) = opened(&baseline);
    for changed in [
        {
            let mut s = baseline.clone();
            s.selected = Some(room(2));
            s
        },
        {
            let mut s = baseline.clone();
            s.active_account = None;
            s
        },
        {
            let mut s = baseline.clone();
            s.accounts = Page::empty();
            s
        },
        {
            let mut s = baseline.clone();
            let mut context = room(1);
            context.account.runtime = RuntimeGeneration::from_local(2);
            s.selected = Some(context);
            s
        },
        {
            let mut s = baseline.clone();
            s.content = ContentState::Denied;
            s
        },
        {
            let mut s = baseline.clone();
            s.message_panel = None;
            s
        },
        {
            let mut s = baseline.clone();
            s.message_panel.as_mut().unwrap().features = Page::empty();
            s
        },
        {
            let mut s = baseline.clone();
            s.conversations = page(s.conversations.as_slice().iter().cloned().map(|mut r| {
                if r.context == room(1) {
                    r.organization.classified = Some(true);
                }
                r
            }));
            s
        },
        {
            let mut s = baseline.clone();
            s.conversations = page(s.conversations.as_slice().iter().cloned().map(|mut r| {
                if r.context == room(1) {
                    r.organization.classified = None;
                }
                r
            }));
            s
        },
    ] {
        assert_eq!(session.confirm(confirmation, &changed), Err(Denial::Source));
    }
    let mut session = session;
    for destination in [
        room(1),
        {
            let mut r = room(2);
            r.account.runtime = RuntimeGeneration::from_local(2);
            r
        },
        {
            let mut r = room(2);
            r.account.account = AccountId::from_local(2);
            r.conversation.account = r.account.account;
            r
        },
        {
            let mut r = room(2);
            r.conversation.host = HostId::from_local(2);
            r
        },
    ] {
        assert!(!session.select(target, destination));
    }
}

#[test]
fn destination_is_revalidated_after_reconnect_without_dropping_selection() {
    let mut snapshot = snapshot();
    let (session, _, confirmation) = opened(&snapshot);
    snapshot.content = ContentState::Offline;
    assert!(session.source_current(&snapshot));
    assert_eq!(
        session.confirm(confirmation, &snapshot),
        Err(Denial::Connection)
    );
    snapshot.content = ContentState::Ready;
    snapshot.conversation_list.content = ContentState::Loading;
    assert_eq!(
        session.confirm(confirmation, &snapshot),
        Err(Denial::Connection)
    );
    snapshot.conversation_list.content = ContentState::Ready;
    for permission in [Decision::Denied, Decision::Unknown, Decision::Stale] {
        snapshot.conversations = page(snapshot.conversations.as_slice().iter().cloned().map(
            |mut r| {
                if r.context == room(2) {
                    r.send = permission;
                }
                r
            },
        ));
        assert_eq!(
            session.confirm(confirmation, &snapshot),
            Err(Denial::Destination)
        );
        assert_eq!(session.active.as_ref().unwrap().destination, Some(room(2)));
    }
    snapshot.conversations = page(snapshot.conversations.as_slice().iter().cloned().map(
        |mut r| {
            r.send = Decision::Allowed;
            r
        },
    ));
    assert_eq!(session.confirm(confirmation, &snapshot), Ok(()));
    snapshot.message_panel.as_mut().unwrap().state = RemoteState::Saving(RequestId(8));
    assert_eq!(
        session.confirm(confirmation, &snapshot),
        Err(Denial::Connection)
    );
}

#[test]
fn offline_confirmation_uses_queue_admission_and_rechecks_the_current_destination() {
    let mut snapshot = snapshot();
    let (session, _, confirmation) = opened(&snapshot);
    snapshot.content = ContentState::Offline;
    snapshot.conversation_list.content = ContentState::Offline;
    assert_eq!(
        session.confirm(confirmation, &snapshot),
        Err(Denial::Connection)
    );
    let panel = snapshot.message_panel.as_mut().unwrap();
    panel.queue_available = true;
    panel.state = RemoteState::Saving(RequestId(9));
    assert_eq!(session.confirm(confirmation, &snapshot), Ok(()));
    snapshot.content = ContentState::Ready;
    snapshot.conversation_list.content = ContentState::Ready;
    snapshot.conversations = page(snapshot.conversations.as_slice().iter().cloned().map(
        |mut r| {
            if r.context == room(2) {
                r.send = Decision::Denied;
            }
            r
        },
    ));
    assert_eq!(
        session.confirm(confirmation, &snapshot),
        Err(Denial::Destination)
    );
    assert_eq!(session.active.as_ref().unwrap().destination, Some(room(2)));
}

#[test]
fn bounded_destinations_require_current_same_account_send_and_known_policy() {
    let mut snapshot = snapshot();
    let template = snapshot.conversations.as_slice()[1].clone();
    snapshot.conversations = page((1..=100).map(|id| {
        let mut r = template.clone();
        r.context = room(id);
        r
    }));
    assert_eq!(destinations(&snapshot, room(1)).count(), 99);
    for classified in [None, Some(true)] {
        snapshot.conversations = page(snapshot.conversations.as_slice().iter().cloned().map(
            |mut r| {
                r.organization.classified = classified;
                r
            },
        ));
        assert_eq!(destinations(&snapshot, room(1)).count(), 0);
    }
}

#[test]
fn focus_returns_to_preserved_surface_only_for_the_same_current_context() {
    for surface in [
        ReturnSurface::Timeline,
        ReturnSurface::CallChat,
        ReturnSurface::Thread,
    ] {
        assert_eq!(
            return_surface(surface, room(1), Some(room(1)), true),
            Some(surface)
        );
        assert_eq!(return_surface(surface, room(1), Some(room(2)), true), None);
        assert_eq!(return_surface(surface, room(1), Some(room(1)), false), None);
    }
    assert_eq!(next_row(None, 0, "down"), None);
    assert_eq!(next_row(None, 100, "down"), Some(0));
    assert_eq!(next_row(Some(0), 100, "up"), Some(99));
    assert_eq!(next_row(Some(99), 100, "down"), Some(0));
    assert_eq!(next_row(Some(4), 100, "home"), Some(0));
    assert_eq!(next_row(Some(4), 100, "end"), Some(99));
}
