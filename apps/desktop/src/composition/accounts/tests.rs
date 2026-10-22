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
        server: ServerInput::new("https://cloud.example.test/nextcloud").unwrap(),
        reservation: Some(account),
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
fn login_retry_keeps_the_bound_reservation_through_rejected_admission() {
    let (mut model, _, account, _) = pending_login();
    let server = model.login.as_ref().unwrap().server.clone();
    model.login_failed(IdentityError::RecoveryRequired);
    for request in [RequestId(18), RequestId(19)] {
        // No worker is configured in this state fixture. Even an admission
        // failure must preserve the retry target instead of reserving account B.
        assert!(
            model
                .submit(UiIntent::BeginLogin {
                    request,
                    server: server.clone(),
                })
                .is_err()
        );
        let target = model.replace.as_ref().expect("same-account retry target");
        assert_eq!(target.account, account);
        assert_eq!(target.server.as_ref(), Some(&server));
        assert_eq!(
            target.save_choice,
            talk_core::identity::StorageChoice::Native
        );
    }
    model.submit(UiIntent::NewLogin).unwrap();
    assert!(model.replace.is_none());
    assert!(
        model
            .submit(UiIntent::BeginLogin {
                request: RequestId(20),
                server,
            })
            .is_err()
    );
    assert!(
        model.replace.is_none(),
        "NewLogin cannot inherit a retry account"
    );
}

#[test]
fn typed_login_retry_preserves_installation_and_replacement_after_repeated_worker_failure() {
    let (mut model, _, account, _) = pending_login();
    let server = model.login.as_ref().unwrap().server.clone();
    model.login_failed(IdentityError::Request(ErrorKind::Transport));
    for request in [RequestId(18), RequestId(19)] {
        assert!(model.submit(UiIntent::RetryLogin { request }).is_err());
        let target = model.replace.as_ref().unwrap();
        assert_eq!(target.account, account);
        assert_eq!(target.server.as_ref(), Some(&server));
        assert_eq!(model.login_retry_server.as_ref(), Some(&server));
        assert!(matches!(model.snapshot.login.phase, LoginPhase::Failed(_)));
        assert_eq!(
            model.snapshot.login.server_label.as_str(),
            "https://cloud.example.test"
        );
    }
    model.submit(UiIntent::NewLogin).unwrap();
    assert!(model.login_retry_server.is_none());
    assert_eq!(
        model.submit(UiIntent::RetryLogin {
            request: RequestId(20)
        }),
        Err(SubmitError::StaleScope)
    );
    assert!(model.replace.is_none());
}

#[test]
fn rejected_begin_can_change_server_without_an_admitted_attempt() {
    for busy in [false, true] {
        let mut model = model();
        if busy {
            for id in 1..=16 {
                model.pending.push(Pending {
                    id: ConnectionRequestId(id),
                    purpose: Purpose::Login(RequestId(id), LoginWork::Begin),
                });
            }
        }
        let server = ServerInput::new("https://cloud.example.test/nextcloud").unwrap();
        for request in [RequestId(20), RequestId(21)] {
            assert!(
                model
                    .submit(UiIntent::BeginLogin {
                        request,
                        server: server.clone()
                    })
                    .is_err()
            );
            assert!(matches!(model.snapshot.login.phase, LoginPhase::Failed(_)));
            assert!(model.snapshot.login.attempt.is_none());
            assert!(model.login.is_none());
            assert_eq!(model.login_retry_server.as_ref(), Some(&server));
        }
        // The failure page uses NewLogin when no admitted attempt can be cancelled.
        model.submit(UiIntent::NewLogin).unwrap();
        assert_eq!(model.snapshot.login.phase, LoginPhase::EnterServer);
        assert!(model.login_retry_server.is_none());
        assert!(model.replace.is_none());
        assert_eq!(
            model.submit(UiIntent::RetryLogin {
                request: RequestId(22)
            }),
            Err(SubmitError::StaleScope)
        );
    }
}

#[test]
fn new_login_retry_retains_full_address_without_inventing_a_replacement() {
    let mut model = model();
    let server = ServerInput::new("https://cloud.example.test/nextcloud").unwrap();
    assert!(
        model
            .submit(UiIntent::BeginLogin {
                request: RequestId(1),
                server: server.clone()
            })
            .is_err()
    );
    assert!(
        model
            .submit(UiIntent::RetryLogin {
                request: RequestId(2)
            })
            .is_err()
    );
    assert_eq!(model.login_retry_server, Some(server));
    assert!(model.replace.is_none());
}

#[test]
fn login_retry_never_reuses_another_installation_or_explicitly_cancelled_target() {
    for server in [
        "https://cloud.example.test/other-installation",
        "https://cloud.example.test:8443/nextcloud",
        "https://other.example.test/nextcloud",
    ] {
        for retained_after_rejection in [false, true] {
            let (mut model, _, _, _) = pending_login();
            if retained_after_rejection {
                let previous = model.login.as_ref().unwrap().server.clone();
                model.prepare_login(&previous);
            }
            assert!(
                model
                    .submit(UiIntent::BeginLogin {
                        request: RequestId(18),
                        server: ServerInput::new(server).unwrap(),
                    })
                    .is_err()
            );
            assert!(
                model.replace.is_none(),
                "a changed destination starts fresh"
            );
        }
    }
    let (mut model, request, _, _) = pending_login();
    model.submit(UiIntent::CancelLogin(request)).unwrap();
    assert!(model.login.is_none());
    assert!(model.replace.is_none());
}

#[test]
fn failed_browser_begin_retains_its_reserved_account_and_rejects_late_authorization() {
    let (mut model, request, account, attempt) = pending_login();
    let login = model.login.as_mut().unwrap();
    login.owner = None;
    login.reservation = None;
    let server = login.server.clone();
    model.snapshot.login.phase = LoginPhase::Validating;
    let mut done = ConnectionCompletion {
        cursor: ConnectionCursor::default(),
        request: ConnectionRequestId(1),
        account: Some(account),
        attempt: None,
        captured_scope: None,
        cancel_requested: false,
        scope_is_current: false,
        result: Err(IdentityError::Request(ErrorKind::Transport)),
    };
    model.complete(Purpose::Login(request, LoginWork::Begin), &done);
    assert_eq!(
        model.snapshot.login.phase,
        LoginPhase::Failed(LoginFailure::Connection(ConnectionFailure::Unreachable))
    );
    assert_eq!(model.login.as_ref().unwrap().reservation, Some(account));
    model.prepare_login(&server);
    assert_eq!(model.replace.as_ref().unwrap().account, account);
    done.attempt = Some(attempt);
    done.result = Ok(RuntimeReply::Authorized { account, attempt });
    model.complete(Purpose::Login(request, LoginWork::Poll), &done);
    assert!(model.login.is_none());
    assert!(model.pending.is_empty());
    assert_eq!(model.snapshot.login.phase, LoginPhase::EnterServer);
}

#[test]
fn login_retry_preserves_a_deliberate_file_choice_without_inventing_one() {
    let (mut model, _, _, _) = pending_login();
    let server = model.login.as_ref().unwrap().server.clone();
    model.login.as_mut().unwrap().automatic_save_choice =
        talk_core::identity::StorageChoice::PrivateFile;
    model.prepare_login(&server);
    model.prepare_login(&server);
    assert_eq!(
        model.replace.as_ref().unwrap().save_choice,
        talk_core::identity::StorageChoice::PrivateFile
    );
    model.submit(UiIntent::NewLogin).unwrap();
    model.prepare_login(&server);
    assert!(
        model.replace.is_none(),
        "a new login still defaults to native storage"
    );
}

#[test]
fn login_recovery_conflict_and_provider_failure_have_distinct_safe_states() {
    for (work, error, phase, storage, failure) in [
        (
            LoginWork::Poll,
            IdentityError::Request(ErrorKind::Conflict),
            LoginPhase::Failed(LoginFailure::AccountExists),
            StorageStatus::NotSaved,
            None,
        ),
        (
            LoginWork::Poll,
            IdentityError::RecoveryRequired,
            LoginPhase::Failed(LoginFailure::Storage),
            StorageStatus::RecoveryRequired,
            Some(StorageSaveFailure::RecoveryRequired),
        ),
        (
            LoginWork::Save,
            IdentityError::SuppressionUnconfirmed,
            LoginPhase::Failed(LoginFailure::Storage),
            StorageStatus::RecoveryRequired,
            Some(StorageSaveFailure::SuppressionUnconfirmed),
        ),
        (
            LoginWork::Save,
            IdentityError::Request(ErrorKind::Uncertain),
            LoginPhase::Failed(LoginFailure::Storage),
            StorageStatus::RecoveryRequired,
            Some(StorageSaveFailure::Uncertain),
        ),
        (
            LoginWork::Save,
            IdentityError::Storage {
                backend: StorageBackend::MacKeychain,
                failure: StorageFailure::AccessDenied,
            },
            LoginPhase::ChooseStorage,
            StorageStatus::NotSaved,
            Some(StorageSaveFailure::NativeDenied),
        ),
    ] {
        let (mut model, request, account, attempt) = pending_login();
        if work == LoginWork::Save {
            model.login.as_mut().unwrap().save_request = Some(ConnectionRequestId(9));
            model.snapshot.login.storage = StorageStatus::Saving;
            model.snapshot.login.phase = LoginPhase::ChooseStorage;
        }
        let done = ConnectionCompletion {
            cursor: ConnectionCursor::default(),
            request: ConnectionRequestId(9),
            account: Some(account),
            attempt: Some(attempt),
            captured_scope: None,
            cancel_requested: false,
            scope_is_current: false,
            result: Err(error),
        };
        model.complete(Purpose::Login(request, work), &done);
        assert_eq!(model.snapshot.login.phase, phase);
        assert_eq!(model.snapshot.login.storage, storage);
        assert_eq!(model.snapshot.login.storage_failure, failure);
        assert!(model.pending.is_empty(), "failure never submits a fallback");
        if storage == StorageStatus::RecoveryRequired {
            assert_eq!(
                model.submit(UiIntent::ChooseStorage {
                    attempt: request,
                    choice: StorageChoice::PrivateFile,
                }),
                Err(SubmitError::Busy),
                "unreconciled storage blocks a fallback"
            );
        }
    }
}

#[test]
fn login_diagnostics_distinguish_pre_save_conflict_provider_and_recovery_failures() {
    let (_, _, account, attempt) = pending_login();
    for (work, error, operation, outcome, code, native) in [
        (
            LoginWork::Poll,
            IdentityError::Request(ErrorKind::Conflict),
            "login_poll",
            "failed",
            "conflict",
            false,
        ),
        (
            LoginWork::Save,
            IdentityError::RecoveryRequired,
            "login_save",
            "failed",
            "storage_unavailable",
            false,
        ),
        (
            LoginWork::Save,
            IdentityError::SuppressionUnconfirmed,
            "login_save",
            "uncertain",
            "storage_unavailable",
            false,
        ),
        (
            LoginWork::Save,
            IdentityError::Request(ErrorKind::Uncertain),
            "login_save",
            "uncertain",
            "uncertain",
            false,
        ),
        (
            LoginWork::Save,
            IdentityError::Storage {
                backend: StorageBackend::MacKeychain,
                failure: StorageFailure::AccessDenied,
            },
            "login_save",
            "failed",
            "storage_unavailable",
            true,
        ),
    ] {
        let done = ConnectionCompletion {
            cursor: ConnectionCursor::default(),
            request: ConnectionRequestId(9),
            account: Some(account),
            attempt: Some(attempt),
            captured_scope: None,
            cancel_requested: false,
            scope_is_current: false,
            result: Err(error),
        };
        let line = diagnostics::test_line(login_completion_event(work, &done).unwrap());
        assert!(line.contains("\"request\":9"));
        assert!(line.contains(&format!("\"operation\":\"{operation}\"")));
        assert!(line.contains(&format!("\"outcome\":\"{outcome}\"")));
        assert!(line.contains(&format!("\"error\":\"{code}\"")));
        if native {
            assert!(line.contains("\"storage_backend\":\"mac_keychain\""));
            assert!(line.contains("\"storage_failure\":\"access_denied\""));
        }
        assert_eq!(line.matches(':').count(), if native { 11 } else { 9 });
        assert!(
            !line.contains("account")
                && !line.contains("attempt")
                && !line.contains("http")
                && !line.contains("password")
        );
    }
}

#[test]
fn legacy_bound_accounts_without_credentials_offer_same_account_reauthentication() {
    use talk_adapters::identity::{AccountPhase, ConnectionAccount, RecoverySelection};
    use talk_core::identity::{ProviderBinding, QualifiedItem, Settlement};
    use talk_core::ids::{ProfileId, VaultGeneration};
    let account = AccountId::from_local(3);
    let item = QualifiedItem {
        account,
        generation: VaultGeneration::from_local(7),
        provider: ProviderBinding {
            backend: StorageBackend::MacKeychain,
            profile: ProfileId::from_local(11),
            store: 13,
        },
    };
    let summary = AccountSummary {
        account,
        navigation_key: talk_core::preferences::navigation::NavigationAccountKey::new(
            "a1:00000000000000000000000000000001:00000000000000000000000000000002",
        )
        .unwrap(),
        server_label: Some(text("cloud.example.test")),
        account_label: Some(text("Synthetic")),
        attempt: None,
        scope: None,
        phase: AccountPhase::Disconnected,
        display_name: text(""),
        storage: StoredState::Suppressed,
        cleanup_pending: false,
        suppression_unconfirmed: false,
        connection_degraded: false,
        connection_error: None,
        recovery: None,
        revision: 1,
    };
    let mut settled_absent = summary.clone();
    settled_absent.storage = StoredState::Unresolved;
    settled_absent.recovery = Some(RecoverySelection {
        attempted: item,
        previous: None,
        revision: 1,
        settlement: Settlement::SettledAbsent,
    });
    let mut unknown_writer = settled_absent.clone();
    unknown_writer.recovery.as_mut().unwrap().settlement = Settlement::Unknown;
    unknown_writer.cleanup_pending = true;
    let mut retained_previous = settled_absent.clone();
    retained_previous.recovery.as_mut().unwrap().previous = Some(QualifiedItem {
        generation: VaultGeneration::from_local(5),
        ..item
    });
    let mut unconfirmed = settled_absent.clone();
    unconfirmed.suppression_unconfirmed = true;
    let mut cancelled_writer = summary.clone();
    cancelled_writer.cleanup_pending = true;
    let mut still_owned = settled_absent.clone();
    still_owned.attempt = Some(LoginAttemptId::from_local(17));
    for (summary, state, expected) in [
        (
            summary,
            SavedAccountState::Dormant,
            SavedAccountPhase::ReauthenticationRequired,
        ),
        (
            settled_absent,
            SavedAccountState::RecoveryRequired,
            SavedAccountPhase::ReauthenticationRequired,
        ),
        (
            unknown_writer,
            SavedAccountState::RecoveryRequired,
            SavedAccountPhase::RecoveryRequired,
        ),
        (
            retained_previous,
            SavedAccountState::RecoveryRequired,
            SavedAccountPhase::RecoveryRequired,
        ),
        (
            unconfirmed,
            SavedAccountState::RecoveryRequired,
            SavedAccountPhase::RecoveryRequired,
        ),
        (
            cancelled_writer,
            SavedAccountState::Dormant,
            SavedAccountPhase::RecoveryRequired,
        ),
        (
            still_owned,
            SavedAccountState::RecoveryRequired,
            SavedAccountPhase::RecoveryRequired,
        ),
    ] {
        let mut model = model();
        model.sync_accounts(&ConnectionSnapshot {
            state: ConnectionState::Ready,
            accounts: page([ConnectionAccount { summary, state }]),
            pending: Page::empty(),
            retained: 0,
        });
        let saved = &model.snapshot.saved_accounts.as_slice()[0];
        assert_eq!(saved.id, account, "legacy identity remains visible");
        assert_eq!(saved.phase, expected);
        assert!(model.snapshot.accounts.as_slice().is_empty());
        assert!(model.snapshot.active_account.is_none());
        if expected == SavedAccountPhase::ReauthenticationRequired {
            // This is the existing account chooser's typed reconnect action.
            model.submit(UiIntent::Reauthenticate(account)).unwrap();
            model.prepare_login(&ServerInput::new("https://cloud.example.test/nextcloud").unwrap());
            assert_eq!(model.replace.as_ref().unwrap().account, account);
        }
    }
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
fn login_activation_handoff_releases_only_the_current_save() {
    let (mut model, request, account, attempt) = pending_login();
    let save = ConnectionRequestId(9);
    model.login.as_mut().unwrap().save_request = Some(save);
    model.snapshot.login.phase = LoginPhase::ChooseStorage;
    model.snapshot.login.storage = StorageStatus::Saving;
    let scope = AccountScope {
        account,
        runtime: talk_core::ids::RuntimeGeneration::from_local(31),
    };
    let summary = AccountSummary {
        account,
        navigation_key: talk_core::preferences::navigation::NavigationAccountKey::new(
            "a1:00000000000000000000000000000001:00000000000000000000000000000002",
        )
        .unwrap(),
        server_label: Some(text("cloud.example.test")),
        account_label: Some(text("Synthetic")),
        attempt: None,
        scope: Some(scope),
        phase: talk_adapters::identity::AccountPhase::Active,
        display_name: text("Synthetic"),
        storage: StoredState::Unknown,
        cleanup_pending: false,
        suppression_unconfirmed: false,
        connection_degraded: false,
        connection_error: None,
        recovery: None,
        revision: 1,
    };
    let mut done = ConnectionCompletion {
        cursor: ConnectionCursor::default(),
        request: save,
        account: Some(account),
        attempt: Some(attempt),
        captured_scope: None,
        cancel_requested: false,
        scope_is_current: false,
        result: Ok(RuntimeReply::Active {
            summary,
            probe: protocol::AccountProbe {
                scope,
                display_name: text("Synthetic"),
                family: protocol::ServerFamily::Nextcloud35,
                talk_available: true,
                group_creation: talk_core::policy::Decision::Unknown,
                message_limit: None,
                reference_correlation: true,
                channel_semantics: false,
            },
        }),
    };
    model.complete(Purpose::Login(request, LoginWork::Save), &done);
    assert_eq!(model.login.as_ref().unwrap().save_request, Some(save));
    assert_ne!(model.snapshot.login.phase, LoginPhase::Ready);
    done.scope_is_current = true;
    done.request = ConnectionRequestId(8);
    model.complete(Purpose::Login(request, LoginWork::Save), &done);
    assert_eq!(model.login.as_ref().unwrap().save_request, Some(save));
    done.request = save;
    model.complete(Purpose::Login(request, LoginWork::Save), &done);
    assert_eq!(model.snapshot.login.phase, LoginPhase::Ready);
    assert_eq!(model.login.as_ref().unwrap().save_request, None);
    // The completed account is now owned by explicit logout/removal, rather
    // than a later cancellation or replacement of this login screen.
    model.cancel_login();
    assert_eq!(model.snapshot.login.phase, LoginPhase::EnterServer);
    assert!(model.login.is_none());
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
        diagnostic_connection: None,
        next_request: 0,
        pending: Vec::new(),
        login: None,
        login_retry_server: None,
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

#[test]
fn refused_login_retains_the_safe_cause_for_the_saved_account_row() {
    let (mut model, request, account, _) = pending_login();
    model.login.as_mut().unwrap().owner = None;
    model.login.as_mut().unwrap().reservation = None;
    model.snapshot.login.phase = LoginPhase::Validating;
    let done = ConnectionCompletion {
        cursor: ConnectionCursor::default(),
        request: ConnectionRequestId(1),
        account: Some(account),
        attempt: None,
        captured_scope: None,
        cancel_requested: false,
        scope_is_current: false,
        result: Err(IdentityError::Request(ErrorKind::ServerAccessDenied)),
    };
    model.complete(Purpose::Login(request, LoginWork::Begin), &done);
    let expected = LoginFailure::Connection(ConnectionFailure::AccessDenied);
    assert_eq!(model.snapshot.login.phase, LoginPhase::Failed(expected));
    assert_eq!(model.failures, vec![(account, expected)]);
    assert_eq!(model.login.as_ref().unwrap().reservation, Some(account));
}
