//! Compile regression: the renderer names every payload through its one allowed surface.
use talk_core::ui::{
    AccountId, AccountScope, CallGeneration, CallScope, ConversationId, ConversationRow, Decision,
    HostId, LimitExceeded, MediaIntent, Page, PersistenceState, RuntimeGeneration, Text,
    UserIntent, VaultGeneration, ViewSnapshot,
};

fn join_label(row: &ConversationRow) -> &'static str {
    match row.can_join {
        Decision::Allowed => "allowed",
        Decision::Denied => "denied",
        Decision::Unknown => "unknown",
        Decision::Stale => "stale",
    }
}

fn persistence_label(state: PersistenceState) -> &'static str {
    match state {
        PersistenceState::Unconfigured => "unconfigured",
        PersistenceState::Stored(_) => "stored",
        PersistenceState::SessionOnly => "session only",
        PersistenceState::SessionOnlyCleanupPending => "cleanup pending",
        PersistenceState::RestartSuppressionUnconfirmed => "suppression unconfirmed",
        PersistenceState::RecoveryRequired => "recovery required",
    }
}

fn label(value: &str) -> Result<Text<256>, LimitExceeded> {
    Text::new(value)
}

#[test]
fn bounded_snapshot_decisions_persistence_and_typed_intents_are_public() {
    let account = AccountId::from_local(1);
    let scope = AccountScope {
        account,
        runtime: RuntimeGeneration::from_local(2),
    };
    let conversation = ConversationId {
        account,
        host: HostId::from_local(3),
        local: 4,
    };
    let call = CallScope {
        account: scope,
        conversation,
        call: CallGeneration::from_local(5),
    };
    let mut snapshot = ViewSnapshot {
        revision: 1,
        account: Some(scope),
        availability: None,
        persistence: PersistenceState::Stored(VaultGeneration::from_local(6)),
        conversations: Page::empty(),
        messages: Page::empty(),
    };
    snapshot
        .conversations
        .try_push(ConversationRow {
            id: conversation,
            label: label("synthetic").unwrap(),
            can_join: Decision::Unknown,
        })
        .unwrap();
    assert_eq!(join_label(&snapshot.conversations.as_slice()[0]), "unknown");
    assert_eq!(persistence_label(snapshot.persistence), "stored");
    let intents = [
        UserIntent::Logout(account),
        UserIntent::SelectConversation(conversation),
        UserIntent::AcceptDirect {
            call,
            media: MediaIntent::MicrophoneOffCameraOff,
        },
    ];
    assert!(matches!(
        intents[2], UserIntent::AcceptDirect { call: selected, .. } if selected == call
    ));
}
