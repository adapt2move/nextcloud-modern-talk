use super::*;
use talk_core::preferences::account::{AccountPreferenceResult, AccountPreferences};

fn pending_login() -> (Accounts, RequestId, AccountId, LoginAttemptId) {
    let mut model = model();
    let request = RequestId(17);
    let account = AccountId::from_local(3);
    let attempt = LoginAttemptId::from_local(23);
    model.login = Some(Login {
        request,
        begin: ConnectionRequestId(1),
        owner: Some((account, attempt)),
        poll_at: Instant::now(),
        automatic_save_attempted: false,
        automatic_save_pending: None,
        automatic_save_choice: talk_core::identity::StorageChoice::Native,
        save_request: None,
    });
    model.snapshot.login.phase = LoginPhase::BrowserWaiting;
    model.snapshot.login.attempt = Some(request);
    model.snapshot.login.server_label = text("https://cloud.example.test");
    (model, request, account, attempt)
}

#[test]
fn authorized_login_schedules_one_native_save_before_showing_any_fallback() {
    let (mut model, request, account, attempt) = pending_login();
    let done = ConnectionCompletion {
        cursor: ConnectionCursor::default(),
        request: ConnectionRequestId(2),
        account: Some(account),
        attempt: Some(attempt),
        captured_scope: None,
        cancel_requested: false,
        scope_is_current: false,
        result: Ok(RuntimeReply::Authorized { account, attempt }),
    };
    model.complete(Purpose::Login(request, LoginWork::Poll), &done);
    assert_eq!(model.snapshot.login.storage, StorageStatus::Saving);
    assert_eq!(model.snapshot.login.storage_failure, None);
    let deadline = model.login.as_ref().unwrap().automatic_save_pending;
    assert!(deadline.is_some());
    assert_eq!(
        model.login.as_ref().unwrap().automatic_save_choice,
        talk_core::identity::StorageChoice::Native
    );
    model.complete(Purpose::Login(request, LoginWork::Poll), &done);
    assert_eq!(
        model.login.as_ref().unwrap().automatic_save_pending,
        deadline
    );
    assert_eq!(
        model.snapshot.login.server_label.as_str(),
        "https://cloud.example.test"
    );
    assert_eq!(
        model.submit(UiIntent::ChooseStorage {
            attempt: request,
            choice: StorageChoice::PrivateFile
        }),
        Err(SubmitError::Busy)
    );
}

#[test]
fn only_current_save_failure_exposes_fallback_and_no_session_only_login() {
    let (mut model, request, account, attempt) = pending_login();
    model.login.as_mut().unwrap().automatic_save_attempted = true;
    model.login.as_mut().unwrap().save_request = Some(ConnectionRequestId(9));
    model.snapshot.login.phase = LoginPhase::ChooseStorage;
    model.snapshot.login.storage = StorageStatus::Saving;
    let mut done = ConnectionCompletion {
        cursor: ConnectionCursor::default(),
        request: ConnectionRequestId(9),
        account: Some(account),
        attempt: Some(LoginAttemptId::from_local(99)),
        captured_scope: None,
        cancel_requested: false,
        scope_is_current: false,
        result: Err(IdentityError::Storage {
            backend: StorageBackend::MacKeychain,
            failure: StorageFailure::AccessDenied,
        }),
    };
    model.complete(Purpose::Login(request, LoginWork::Save), &done);
    assert_eq!(model.snapshot.login.storage, StorageStatus::Saving);
    done.attempt = Some(attempt);
    done.cancel_requested = true;
    model.complete(Purpose::Login(request, LoginWork::Save), &done);
    assert_eq!(model.snapshot.login.storage, StorageStatus::Saving);
    done.cancel_requested = false;
    // A late browser error from this attempt cannot replace the saving state.
    model.complete(Purpose::Login(request, LoginWork::Browser), &done);
    assert_eq!(model.snapshot.login.storage, StorageStatus::Saving);
    model.complete(Purpose::Login(request, LoginWork::Save), &done);
    assert_eq!(model.snapshot.login.storage, StorageStatus::NotSaved);
    assert_eq!(model.snapshot.login.native_store, NativeStoreState::Denied);
    assert_eq!(
        model.snapshot.login.storage_failure,
        Some(StorageSaveFailure::NativeDenied)
    );
    assert_eq!(
        model.submit(UiIntent::ChooseStorage {
            attempt: request,
            choice: StorageChoice::SessionOnly
        }),
        Err(SubmitError::Unavailable(UnavailableReason::Unsupported))
    );
    model.cancel_login();
    model.complete(Purpose::Login(request, LoginWork::Save), &done);
    assert_eq!(model.snapshot.login.phase, LoginPhase::EnterServer);
}

#[test]
fn update_admission_rechecks_nonterminal_calls_and_pending_call_work() {
    use talk_core::preferences::updates::UpdateError;
    use talk_core::ui::presentation::{CallStage, CallView, MediaState, RequiredGate};
    let context = super::super::fixtures::context(1);
    let scope = talk_core::ids::CallScope {
        account: context.account,
        conversation: context.conversation,
        call: talk_core::ids::CallGeneration::from_local(31),
    };
    for stage in [
        CallStage::Incoming,
        CallStage::Outgoing,
        CallStage::Connecting,
        CallStage::Active,
        CallStage::Reconnecting,
        CallStage::Required(RequiredGate::Lobby),
    ] {
        let mut m = model();
        m.snapshot.call = Some(CallView {
            scope,
            title: text("Current call"),
            direct: true,
            stage,
            accept: Decision::Unknown,
            microphone: MediaState::Off,
            camera: MediaState::Off,
            share: MediaState::Unavailable,
            participants: Page::empty(),
        });
        assert_eq!(m.seal_for_update(), Err(UpdateError::ActiveCall));
        assert!(!m.call_actions.update_sealed());
    }
    let mut m = model();
    m.pending.push(Pending {
        id: ConnectionRequestId(3),
        purpose: Purpose::Call(scope),
    });
    assert_eq!(m.seal_for_update(), Err(UpdateError::ActiveCall));
    m.pending.clear();
    assert_eq!(m.seal_for_update(), Ok(()));
    m.abort_update_admission();
    assert!(!m.call_actions.update_sealed());
}

pub(super) fn model() -> Accounts {
    Accounts {
        content_owner: content::ContentOwner::default(),
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
        removal: None,
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
        background: background::Background::default(),
        history: history::Cache::default(),
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
