//! Positive replacement for the isolated diagnostic presentation file.
//! Exercises the five BAR1-02 private-type errors plus bounded value construction.
use talk_core::ui::{
    AccountId, CallScope, ConversationId, ConversationRow, Decision, DesktopPort, LimitExceeded,
    MediaIntent, Page, PersistenceState, Text, UserIntent,
};

pub fn describe(backend: &mut dyn DesktopPort) {
    if let Some(snapshot) = backend.take_snapshot() {
        let _label = match snapshot.persistence {
            PersistenceState::Unconfigured => "unconfigured",
            PersistenceState::Stored(_) => "stored",
            PersistenceState::SessionOnly => "session only",
            PersistenceState::SessionOnlyCleanupPending => "cleanup pending",
            PersistenceState::RestartSuppressionUnconfirmed => "suppression unconfirmed",
            PersistenceState::RecoveryRequired => "recovery required",
        };
        for row in snapshot.conversations.as_slice() {
            let _available = match row.can_join {
                Decision::Allowed => true,
                Decision::Denied | Decision::Unknown | Decision::Stale => false,
            };
        }
    }
}

#[allow(dead_code)]
pub fn typed_intents(
    account: AccountId,
    call: CallScope,
    conversation: ConversationId,
) -> [UserIntent; 3] {
    [
        UserIntent::Logout(account),
        UserIntent::SelectConversation(conversation),
        UserIntent::AcceptDirect { call, media: MediaIntent::MicrophoneOffCameraOff },
    ]
}

#[allow(dead_code)]
pub fn bounded_rows(id: ConversationId) -> Result<Page<ConversationRow, 1>, LimitExceeded> {
    let mut rows = Page::empty();
    rows.try_push(ConversationRow { id, label: Text::new("synthetic")?, can_join: Decision::Unknown })?;
    Ok(rows)
}
