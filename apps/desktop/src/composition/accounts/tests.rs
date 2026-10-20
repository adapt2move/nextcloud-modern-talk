use super::*;
use talk_core::preferences::account::{AccountPreferenceResult, AccountPreferences};

fn model() -> Accounts {
    Accounts {
        media: talk_adapters::media::MediaService::new(),
        call_ui: None,
        call_poll_at: Instant::now(),
        call_events: calls::CallEvents::new(),
        call_actions: std::sync::Arc::new(calls::CallActions::new()),
        connection: Err(IdentityError::WorkerUnavailable),
        cursor: ConnectionCursor::default(),
        next_request: 0,
        pending: Vec::new(),
        login: None,
        replace: None,
        snapshot: super::super::fixtures::snapshot(None),
        dirty: true,
        load_rooms: None,
        load_tags: None,
        preferences: Vec::new(),
        failures: Vec::new(),
        unresolved: Vec::new(),
        room_invite: None,
        message_search: None,
        notices: notifications::Notices::default(),
        picker: None,
        picker_open: false,
        message_poll_at: Instant::now(),
        message_poll_request: 0,
    }
}
fn completion(scope: AccountScope, current: bool, reply: ProtocolReply) -> ConnectionCompletion {
    ConnectionCompletion {
        cursor: ConnectionCursor::default(),
        request: ConnectionRequestId(1),
        account: Some(scope.account),
        attempt: None,
        captured_scope: Some(scope),
        cancel_requested: false,
        scope_is_current: current,
        result: Ok(RuntimeReply::Protocol(reply)),
    }
}
#[test]
fn retired_scope_cannot_repopulate_visible_history_or_mark_room_list_complete() {
    let mut m = model();
    let context = super::super::fixtures::context(1);
    m.snapshot.active_account = Some(context.account);
    m.snapshot.selected = Some(context);
    m.snapshot.message_panel = Some(talk_core::ui::messaging::MessagePanel::new(context));
    let operation = OperationId::from_local(11);
    let done = completion(
        context.account,
        false,
        ProtocolReply::Messaging {
            conversation: context.conversation,
            operation,
            result: protocol::messaging::MessagingResult::Messages {
                messages: Page::empty(),
                cursor: None,
                common_read: None,
            },
        },
    );
    m.complete(
        Purpose::Message(RequestId(1), context, operation, messaging::Work::History),
        &done,
    );
    assert!(m.snapshot.messages.as_slice().is_empty());
    let done = completion(
        context.account,
        false,
        ProtocolReply::Conversations(protocol::ConversationList {
            scope: context.account,
            rooms: Page::empty(),
        }),
    );
    m.complete(Purpose::Rooms(context.account), &done);
    assert!(!m.snapshot.conversation_list.complete);
}
#[test]
fn uncertain_send_keeps_original_operation_and_blocks_replay_until_reconciled() {
    let mut m = model();
    let context = super::super::fixtures::context(1);
    let request = RequestId(7);
    let operation = OperationId::from_local(99);
    m.unresolved.push(Unresolved {
        request,
        context,
        operation,
    });
    let done = completion(
        context.account,
        true,
        ProtocolReply::Send(protocol::SendReceipt {
            operation,
            state: SendState::Uncertain,
        }),
    );
    m.complete(Purpose::Send(request, context, operation), &done);
    assert_eq!(m.snapshot.submission.unwrap().result, Completion::Uncertain);
    m.submit(UiIntent::AcknowledgeSubmission { request, context })
        .unwrap();
    assert_eq!(m.unresolved[0].operation, operation);
    assert_eq!(
        m.submit(UiIntent::SendDraft {
            request: RequestId(8),
            context,
            body: text("Same draft")
        }),
        Err(SubmitError::Busy)
    );
    let done = completion(
        context.account,
        true,
        ProtocolReply::Send(protocol::SendReceipt {
            operation,
            state: SendState::ServerAccepted(protocol::MessageId {
                conversation: context.conversation,
                sequence: 42,
            }),
        }),
    );
    m.complete(Purpose::Send(request, context, operation), &done);
    assert!(m.unresolved.is_empty());
    assert_eq!(m.snapshot.submission.unwrap().request, request);
    assert_eq!(
        m.snapshot.submission.unwrap().result,
        Completion::ServerAccepted
    );
}
#[test]
fn stale_preference_reply_preserves_confirmed_values_and_cannot_enable_a_toggle() {
    let mut m = model();
    let scope = super::super::fixtures::scope();
    let request = PreferenceRequest(3);
    let values = AccountPreferences {
        scope,
        send_read_receipts: Some(false),
        share_typing_status: None,
        attachment_folder: None,
        attachments_allowed: None,
        conversation_subfolders: None,
    };
    m.preferences.push(AccountPreferenceSnapshot {
        scope,
        values: Some(values.clone()),
        state: AccountPreferenceState::Saving { request },
    });
    let mut incoming = values.clone();
    incoming.send_read_receipts = Some(true);
    let done = completion(
        scope,
        false,
        ProtocolReply::Preferences(AccountPreferenceResult {
            request,
            values: incoming,
        }),
    );
    m.complete(Purpose::Preferences(scope, request), &done);
    assert_eq!(m.account_snapshot(scope).values, Some(values));
    assert_eq!(
        m.account_snapshot(scope).state,
        AccountPreferenceState::Failed {
            request,
            error: AccountPreferenceError::Stale
        }
    );
}
