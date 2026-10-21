use super::*;
use crate::identity::{
    AccountConnection, ConnectionCompletion, ConnectionCursor, ConnectionRequestId,
    ConnectionState, RuntimeCommand, RuntimeConfig, RuntimeReply, SavedAccountState,
};
use std::sync::atomic::Ordering;
use talk_core::{
    bounded::Text,
    identity::StorageChoice,
    ids::AccountId,
    ports::{ErrorKind, ProtocolReply, ProtocolRequest},
};

async fn ready(connection: &mut AccountConnection) {
    let until = tokio::time::Instant::now() + std::time::Duration::from_secs(5);
    loop {
        let update = connection.poll(ConnectionCursor::default()).unwrap();
        if update.snapshot.state == ConnectionState::Ready {
            return;
        }
        assert_eq!(update.snapshot.state, ConnectionState::Starting);
        assert!(tokio::time::Instant::now() < until);
        tokio::time::sleep(std::time::Duration::from_millis(5)).await;
    }
}

async fn completed(
    connection: &mut AccountConnection,
    cursor: ConnectionCursor,
    request: u64,
) -> ConnectionCompletion {
    let until = tokio::time::Instant::now() + std::time::Duration::from_secs(5);
    loop {
        let update = connection.poll(cursor).unwrap();
        if let Some(done) = update
            .completions
            .as_slice()
            .iter()
            .find(|done| done.request == ConnectionRequestId(request))
        {
            return done.clone();
        }
        assert!(
            tokio::time::Instant::now() < until,
            "completion must remain observable"
        );
        tokio::time::sleep(std::time::Duration::from_millis(5)).await;
    }
}

async fn begin_connection_login(
    connection: &mut AccountConnection,
    server: &Server,
    cursor: ConnectionCursor,
    request: u64,
    replace: Option<AccountId>,
) -> (AccountId, LoginAttemptId, ConnectionCursor) {
    connection
        .try_submit(
            ConnectionRequestId(request),
            RuntimeCommand::Begin {
                server: Text::new(&server.origin).unwrap(),
                replace,
            },
        )
        .unwrap();
    let done = completed(connection, cursor, request).await;
    let Ok(RuntimeReply::Begun { account, attempt }) = done.result else {
        panic!("browser login must begin");
    };
    (account, attempt, done.cursor)
}

#[tokio::test]
async fn login_retry_reuses_bound_account_after_native_nonwrite_and_restores_explicit_file_save() {
    for major in [34, 35] {
        let server = Server::start(major).await;
        let fixture = Fixture::new();
        let config = RuntimeConfig::isolated_fixture(fixture.0.clone());
        let faults = config.faults.clone();
        let mut connection = AccountConnection::start(config).unwrap();
        ready(&mut connection).await;
        let (account, old_attempt, cursor) = begin_connection_login(
            &mut connection,
            &server,
            ConnectionCursor::default(),
            1,
            None,
        )
        .await;
        server.authorized.store(true, Ordering::Release);
        connection
            .try_submit(
                ConnectionRequestId(2),
                RuntimeCommand::Poll {
                    account,
                    attempt: old_attempt,
                },
            )
            .unwrap();
        let authorized = completed(&mut connection, cursor, 2).await;
        assert!(matches!(
            authorized.result,
            Ok(RuntimeReply::Authorized { .. })
        ));

        // Known non-write injection runs before the native adapter. This test
        // neither contacts nor qualifies the machine's native credential store.
        faults.not_written.store(true, Ordering::Release);
        connection
            .try_submit(
                ConnectionRequestId(3),
                RuntimeCommand::Save {
                    account,
                    attempt: old_attempt,
                    choice: StorageChoice::Native,
                },
            )
            .unwrap();
        let failed = completed(&mut connection, authorized.cursor, 3).await;
        assert!(matches!(
            failed.result,
            Err(IdentityError::Storage {
                failure: StorageFailure::AccessDenied,
                ..
            })
        ));
        connection.cancel_request(ConnectionRequestId(3)).unwrap();
        connection.cancel_attempt(account, old_attempt).unwrap();
        let (same, attempt, cursor) =
            begin_connection_login(&mut connection, &server, failed.cursor, 4, Some(account)).await;
        assert_eq!(same, account);
        assert_ne!(attempt, old_attempt);
        assert_eq!(connection.snapshot().accounts.as_slice().len(), 1);
        assert!(connection.cancel_attempt(account, old_attempt).is_err());
        connection
            .try_submit(
                ConnectionRequestId(5),
                RuntimeCommand::Poll { account, attempt },
            )
            .unwrap();
        let authorized = completed(&mut connection, cursor, 5).await;
        assert!(matches!(
            authorized.result,
            Ok(RuntimeReply::Authorized { .. })
        ));
        // The fallback is a separate, explicit command, not the result of denial.
        connection
            .try_submit(
                ConnectionRequestId(6),
                RuntimeCommand::Save {
                    account,
                    attempt,
                    choice: StorageChoice::PrivateFile,
                },
            )
            .unwrap();
        let saved = completed(&mut connection, authorized.cursor, 6).await;
        let Ok(RuntimeReply::Active { summary, .. }) = saved.result else {
            panic!("explicit file save must activate the same account");
        };
        assert_eq!(summary.account, account);
        assert!(matches!(summary.storage, StoredState::Eligible(item)
            if item.provider.backend == StorageBackend::PrivateFile));
        let stored = summary.storage;
        assert!(saved.scope_is_current);
        assert!(connection.join());

        let mut restored =
            AccountConnection::start(RuntimeConfig::isolated_fixture(fixture.0.clone())).unwrap();
        ready(&mut restored).await;
        assert_eq!(restored.snapshot().accounts.as_slice().len(), 1);
        restored
            .try_submit(ConnectionRequestId(1), RuntimeCommand::Restore { account })
            .unwrap();
        let done = completed(&mut restored, ConnectionCursor::default(), 1).await;
        assert!(
            matches!(done.result, Ok(RuntimeReply::Active { summary, .. })
            if summary.account == account && summary.storage == stored)
        );
        assert!(done.scope_is_current);
        assert!(restored.join());
    }
}

#[tokio::test]
async fn duplicate_browser_identity_reports_conflict_before_save_and_preserves_existing_account() {
    for major in [34, 35] {
        let server = Server::start(major).await;
        let fixture = Fixture::new();
        let config = RuntimeConfig::isolated_fixture(fixture.0.clone());
        let faults = config.faults.clone();
        let mut connection = AccountConnection::start(config).unwrap();
        ready(&mut connection).await;
        let (account, attempt, cursor) = begin_connection_login(
            &mut connection,
            &server,
            ConnectionCursor::default(),
            1,
            None,
        )
        .await;
        server.authorized.store(true, Ordering::Release);
        connection
            .try_submit(
                ConnectionRequestId(2),
                RuntimeCommand::Poll { account, attempt },
            )
            .unwrap();
        let authorized = completed(&mut connection, cursor, 2).await;
        assert!(matches!(
            authorized.result,
            Ok(RuntimeReply::Authorized { .. })
        ));
        connection
            .try_submit(
                ConnectionRequestId(3),
                RuntimeCommand::Save {
                    account,
                    attempt,
                    choice: StorageChoice::PrivateFile,
                },
            )
            .unwrap();
        let saved = completed(&mut connection, authorized.cursor, 3).await;
        let Ok(RuntimeReply::Active {
            summary: original, ..
        }) = saved.result
        else {
            panic!("initial account must be saved");
        };
        let (duplicate, attempt, cursor) =
            begin_connection_login(&mut connection, &server, saved.cursor, 4, None).await;
        assert_ne!(duplicate, account);
        connection
            .try_submit(
                ConnectionRequestId(5),
                RuntimeCommand::Poll {
                    account: duplicate,
                    attempt,
                },
            )
            .unwrap();
        let conflict = completed(&mut connection, cursor, 5).await;
        assert!(
            matches!(
                conflict.result,
                Err(IdentityError::Request(ErrorKind::Conflict))
            ),
            "a canonical duplicate is not a metadata/credential storage failure"
        );
        assert_eq!(
            faults.settled.load(Ordering::Acquire),
            1,
            "duplicate authorization cannot dispatch another provider write"
        );
        let snapshot = connection.snapshot();
        let retained = snapshot
            .accounts
            .as_slice()
            .iter()
            .find(|a| a.summary.account == account)
            .unwrap();
        assert_eq!(retained.summary.storage, original.storage);
        assert_eq!(retained.summary.scope, original.scope);
        assert!(
            connection
                .account_cancellation(original.scope.unwrap())
                .is_ok()
        );
        assert!(
            snapshot
                .accounts
                .as_slice()
                .iter()
                .find(|a| a.summary.account == duplicate)
                .unwrap()
                .summary
                .scope
                .is_none()
        );
        connection
            .try_submit(
                ConnectionRequestId(6),
                RuntimeCommand::StorageStatus { account: duplicate },
            )
            .unwrap();
        let status = completed(&mut connection, conflict.cursor, 6).await;
        assert!(
            matches!(status.result, Ok(RuntimeReply::StorageStatus { items, .. })
            if items.as_slice().is_empty())
        );
        assert!(connection.join());
    }
}

#[tokio::test]
async fn cold_legacy_bound_account_without_credential_can_explicitly_reauthorize() {
    let server = Server::start(35).await;
    for settled_nonwrite in [false, true] {
        let fixture = Fixture::new();
        let account = {
            let mut ledger = fixture.ledger();
            let account = ledger.reserve(server.origin.clone()).unwrap().id;
            ledger.bind_user(account, "synthetic-user").unwrap();
            ledger
                .set_account_label(account, &Text::new("Synthetic").unwrap())
                .unwrap();
            if settled_nonwrite {
                let op = ledger
                    .stage(
                        account,
                        LoginAttemptId::from_local(random().unwrap()),
                        super::super::provider::native_backend(),
                        "Synthetic.Login@Example.invalid",
                    )
                    .unwrap();
                // Legacy durable intent, confirmed absent; no provider is called.
                ledger.settle(op, false).unwrap();
            }
            account
        };
        let mut connection =
            AccountConnection::start(RuntimeConfig::isolated_fixture(fixture.0.clone())).unwrap();
        ready(&mut connection).await;
        let before = connection.snapshot();
        assert_eq!(before.accounts.as_slice().len(), 1);
        let old = &before.accounts.as_slice()[0];
        assert_eq!(old.summary.account, account);
        assert!(old.summary.scope.is_none() && old.summary.attempt.is_none());
        assert!(!matches!(old.summary.storage, StoredState::Eligible(_)));
        if settled_nonwrite {
            assert_eq!(
                old.summary.recovery.unwrap().settlement,
                talk_core::identity::Settlement::SettledAbsent
            );
            assert!(old.summary.recovery.unwrap().previous.is_none());
        }
        let (same, attempt, cursor) = begin_connection_login(
            &mut connection,
            &server,
            ConnectionCursor::default(),
            1,
            Some(account),
        )
        .await;
        assert_eq!(same, account);
        server.authorized.store(true, Ordering::Release);
        connection
            .try_submit(
                ConnectionRequestId(2),
                RuntimeCommand::Poll { account, attempt },
            )
            .unwrap();
        let authorized = completed(&mut connection, cursor, 2).await;
        assert!(matches!(
            authorized.result,
            Ok(RuntimeReply::Authorized { .. })
        ));
        connection
            .try_submit(
                ConnectionRequestId(3),
                RuntimeCommand::Save {
                    account,
                    attempt,
                    choice: StorageChoice::PrivateFile,
                },
            )
            .unwrap();
        let saved = completed(&mut connection, authorized.cursor, 3).await;
        assert!(
            matches!(saved.result, Ok(RuntimeReply::Active { summary, .. })
            if summary.account == account
                && matches!(summary.storage, StoredState::Eligible(item)
                    if item.provider.backend == StorageBackend::PrivateFile))
        );
        assert!(saved.scope_is_current);
        assert_eq!(connection.snapshot().accounts.as_slice().len(), 1);
        assert!(connection.join());
    }
}

#[tokio::test]
async fn account_connection_retains_results_and_distinguishes_dormant_restore() {
    for major in [34, 35] {
        let server = super::super::test_server::Server::start(major).await;
        let fixture = Fixture::new();
        let mut connection =
            AccountConnection::start(RuntimeConfig::isolated_fixture(fixture.0.clone())).unwrap();
        ready(&mut connection).await;
        connection
            .try_submit(
                ConnectionRequestId(1),
                RuntimeCommand::Begin {
                    server: Text::new(&server.origin).unwrap(),
                    replace: None,
                },
            )
            .unwrap();
        let begun = completed(&mut connection, ConnectionCursor::default(), 1).await;
        let RuntimeReply::Begun { account, attempt } = begun.result.unwrap() else {
            panic!("begin");
        };
        assert_eq!(begun.account, Some(account));
        assert_eq!(begun.attempt, Some(attempt));
        // Dropping delivered DTOs does not drop the retained underlying Observation.
        assert_eq!(
            connection
                .poll(ConnectionCursor::default())
                .unwrap()
                .completions
                .as_slice()
                .len(),
            1
        );
        server.authorized.store(true, Ordering::Release);
        connection
            .try_submit(
                ConnectionRequestId(2),
                RuntimeCommand::Poll { account, attempt },
            )
            .unwrap();
        let authorized = completed(&mut connection, begun.cursor, 2).await;
        assert!(matches!(
            authorized.result,
            Ok(RuntimeReply::Authorized { .. })
        ));
        assert_eq!(
            connection.snapshot().accounts.as_slice()[0].state,
            SavedAccountState::AwaitingStorageChoice
        );
        connection
            .try_submit(
                ConnectionRequestId(3),
                RuntimeCommand::Save {
                    account,
                    attempt,
                    choice: StorageChoice::PrivateFile,
                },
            )
            .unwrap();
        let saved = completed(&mut connection, authorized.cursor, 3).await;
        let Ok(RuntimeReply::Active { summary, .. }) = &saved.result else {
            panic!("save");
        };
        let scope = summary.scope.unwrap();
        assert_ne!(
            connection.new_operation_id(scope).unwrap(),
            connection.new_operation_id(scope).unwrap()
        );
        let key = summary.navigation_key.clone();
        assert_eq!(
            summary.account_label.as_ref().unwrap().as_str(),
            "Synthetic"
        );
        assert!(
            !summary
                .server_label
                .as_ref()
                .unwrap()
                .as_str()
                .contains("/cloud")
        );
        assert!(saved.scope_is_current);
        connection
            .try_submit(
                ConnectionRequestId(4),
                RuntimeCommand::Request {
                    scope,
                    request: ProtocolRequest::InspectAccount,
                },
            )
            .unwrap();
        let inspected = completed(&mut connection, saved.cursor, 4).await;
        assert_eq!(inspected.captured_scope, Some(scope));
        assert!(matches!(
            inspected.result,
            Ok(RuntimeReply::Protocol(ProtocolReply::Account(_)))
        ));
        connection.poll(inspected.cursor).unwrap();
        assert!(connection.join());

        let mut restored =
            AccountConnection::start(RuntimeConfig::isolated_fixture(fixture.0.clone())).unwrap();
        ready(&mut restored).await;
        let snapshot = restored.snapshot();
        let dormant = &snapshot.accounts.as_slice()[0];
        assert_eq!(dormant.state, SavedAccountState::Dormant);
        assert_eq!(dormant.summary.account, account);
        assert_eq!(dormant.summary.navigation_key, key);
        assert_eq!(
            dormant.summary.account_label.as_ref().unwrap().as_str(),
            "Synthetic"
        );
        assert!(dormant.summary.scope.is_none());
        assert!(restored.account_cancellation(scope).is_err());
        assert!(restored.new_operation_id(scope).is_err());
        restored
            .try_submit(ConnectionRequestId(1), RuntimeCommand::Restore { account })
            .unwrap();
        let active = completed(&mut restored, ConnectionCursor::default(), 1).await;
        let Ok(RuntimeReply::Active { summary, .. }) = active.result else {
            panic!("restore");
        };
        let new_scope = summary.scope.unwrap();
        assert_ne!(scope, new_scope);
        let cancel = restored.account_cancellation(new_scope).unwrap();
        restored.logout(account).unwrap();
        assert!(cancel.is_cancelled());
        let replay = restored.poll(ConnectionCursor::default()).unwrap();
        assert!(!replay.completions.as_slice()[0].scope_is_current);
        assert!(
            replay.snapshot.accounts.as_slice()[0]
                .summary
                .scope
                .is_none()
        );
        restored.poll(active.cursor).unwrap();
        assert!(restored.join());
    }
}

#[test]
fn account_connection_schema_upgrade_preserves_eligibility_and_unsettled_items() {
    let fixture = Fixture::new();
    let (account, key, operation) = {
        let mut ledger = fixture.ledger();
        let account = ledger
            .reserve("https://example.invalid/cloud/".into())
            .unwrap()
            .id;
        ledger.bind_user(account, "synthetic-user").unwrap();
        let operation = ledger
            .stage(
                account,
                LoginAttemptId::from_local(random().unwrap()),
                StorageBackend::PrivateFile,
                "synthetic-user",
            )
            .unwrap();
        (account, ledger.account_key(account).unwrap(), operation)
    };
    // Recreate the immediately previous schema while preserving the actual private
    // profile/store binding, account and dispatched operation.
    {
        let db = rusqlite::Connection::open(fixture.0.join("restoration.sqlite")).unwrap();
        db.execute_batch("BEGIN IMMEDIATE; DROP TABLE account_labels; DROP TABLE navigation_rooms; UPDATE meta SET schema=5; COMMIT;").unwrap();
    }
    let mut ledger = fixture.ledger();
    let state = ledger.account(account).unwrap();
    assert_eq!(state.disposition, 1);
    assert!(state.eligible.is_none());
    let retained = ledger.operation(account, operation.id).unwrap();
    assert_eq!(retained.item, operation.item);
    assert_eq!(retained.attempt, operation.attempt);
    assert_eq!(retained.settlement, 0);
    assert!(!retained.abandoned && !retained.cleanup && !retained.activated);
    assert_eq!(ledger.account_key(account).unwrap(), key);
    assert_eq!(ledger.account_label(account).unwrap(), None);
    assert!(ledger.room_key(account, 7).unwrap().is_some());
}

#[tokio::test]
async fn account_connection_charges_completed_slots_until_acknowledgement() {
    let fixture = Fixture::new();
    let mut connection =
        AccountConnection::start(RuntimeConfig::isolated_fixture(fixture.0.clone())).unwrap();
    ready(&mut connection).await;
    for id in 1..=7 {
        connection
            .try_submit(ConnectionRequestId(id), RuntimeCommand::Accounts)
            .unwrap();
    }
    let last = completed(&mut connection, ConnectionCursor::default(), 7).await;
    assert_eq!(connection.snapshot().retained, 7);
    assert!(matches!(
        connection.try_submit(ConnectionRequestId(8), RuntimeCommand::Accounts),
        Err(IdentityError::Request(ErrorKind::Busy))
    ));
    assert!(matches!(
        connection.try_submit(ConnectionRequestId(7), RuntimeCommand::Accounts),
        Err(IdentityError::Request(ErrorKind::Conflict))
    ));
    let page = connection.poll(ConnectionCursor::default()).unwrap();
    assert_eq!(page.completions.as_slice().len(), 7);
    assert_eq!(page.cursor, last.cursor);
    connection.poll(page.cursor).unwrap();
    assert_eq!(connection.snapshot().retained, 0);
    connection
        .try_submit(ConnectionRequestId(8), RuntimeCommand::Accounts)
        .unwrap();
    connection.cancel_request(ConnectionRequestId(8)).unwrap();
    let cancelled = completed(&mut connection, page.cursor, 8).await;
    assert!(cancelled.cancel_requested);
    // A completion racing cancellation is still returned with its correlation.
    connection.shutdown();
    assert_eq!(connection.snapshot().state, ConnectionState::Closing);
    assert!(connection.join());
}

#[test]
fn account_connection_private_keys_and_labels_survive_restart_without_authority() {
    let fixture = Fixture::new();
    let (account, account_key, room_key) = {
        let mut ledger = fixture.ledger();
        let account = ledger
            .reserve("https://example.invalid/cloud/".into())
            .unwrap()
            .id;
        ledger
            .set_account_label(account, &Text::new("A person's display name").unwrap())
            .unwrap();
        let first = ledger.room_key(account, 41).unwrap().unwrap();
        assert_eq!(ledger.room_key(account, 41).unwrap().as_ref(), Some(&first));
        assert_ne!(ledger.room_key(account, 42).unwrap().as_ref(), Some(&first));
        assert_eq!(ledger.room_key(account, 0).unwrap(), None);
        (account, ledger.account_key(account).unwrap(), first)
    };
    let mut ledger = fixture.ledger();
    assert_eq!(ledger.account_key(account).unwrap(), account_key);
    assert_eq!(ledger.room_key(account, 41).unwrap(), Some(room_key));
    assert!(ledger.account(account).unwrap().eligible.is_none());
    ledger
        .set_account_label(account, &Text::new("unsafe\u{202e}name").unwrap())
        .unwrap();
    assert_eq!(ledger.account_label(account).unwrap(), None);
    for id in 100..354 {
        assert!(ledger.room_key(account, id).unwrap().is_some());
    }
    assert_eq!(ledger.room_key(account, 1000).unwrap(), None);
    assert!(ledger.room_key(account, 41).unwrap().is_some());
}

/// Exercise actual file-provider deletion and the retained desktop-facing facade,
/// including a dormant account left behind by an interrupted removal. No network
/// is needed for local removal, and another account's credential stays intact.
#[cfg(unix)]
#[tokio::test]
async fn account_removal_dormant_cleanup_is_local_exact_and_retryable() {
    use std::os::unix::fs::PermissionsExt;
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    listener.set_nonblocking(true).unwrap();
    let origin = format!("http://{}/", listener.local_addr().unwrap());
    for deny_cleanup in [false, true] {
        let fixture = Fixture::new();
        let (account, other, filename, other_filename) = {
            let mut ledger = fixture.ledger();
            let store = providers(&ledger);
            let mut entries = Vec::new();
            for user in ["synthetic-user-a", "synthetic-user-b"] {
                let account = ledger.reserve(origin.clone()).unwrap();
                ledger.bind_user(account.id, user).unwrap();
                let op = ledger
                    .stage(
                        account.id,
                        LoginAttemptId::from_local(random().unwrap()),
                        StorageBackend::PrivateFile,
                        user,
                    )
                    .unwrap();
                assert!(matches!(
                    store.write(op.item, "synthetic-private-credential"),
                    WriteResult::Present
                ));
                ledger.settle(op, true).unwrap();
                let revision = ledger.account(account.id).unwrap().revision;
                ledger.activate(op, revision).unwrap();
                entries.push((account.id, super::super::provider::filename(op.item)));
            }
            // State left by v0.0.5: retired and suppressed, cleanup not dispatched.
            ledger.retire_all(entries[0].0).unwrap();
            (
                entries[0].0,
                entries[1].0,
                entries[0].1.clone(),
                entries[1].1.clone(),
            )
        };
        let file = fixture.0.join(&filename);
        let other_file = fixture.0.join(&other_filename);
        let other_bytes = std::fs::read(&other_file).unwrap();
        if deny_cleanup {
            std::fs::set_permissions(&file, std::fs::Permissions::from_mode(0o644)).unwrap();
        }
        let mut connection =
            AccountConnection::start(RuntimeConfig::isolated_fixture(fixture.0.clone())).unwrap();
        ready(&mut connection).await;
        connection
            .try_submit(
                ConnectionRequestId(1),
                RuntimeCommand::RetireAll { account },
            )
            .unwrap();
        let retired = completed(&mut connection, ConnectionCursor::default(), 1).await;
        assert!(
            matches!(retired.result, Ok(RuntimeReply::State(ref s)) if s.cleanup_pending && s.scope.is_none())
        );
        assert!(file.exists());
        connection
            .try_submit(ConnectionRequestId(2), RuntimeCommand::Maintain { account })
            .unwrap();
        let cleanup = completed(&mut connection, retired.cursor, 2).await;
        let mut cursor = cleanup.cursor;
        if deny_cleanup {
            assert!(matches!(
                cleanup.result,
                Err(IdentityError::Storage {
                    backend: StorageBackend::PrivateFile,
                    failure: StorageFailure::AccessDenied
                })
            ));
            assert!(file.exists());
            assert!(
                connection
                    .snapshot()
                    .accounts
                    .as_slice()
                    .iter()
                    .any(|a| a.summary.account == account && a.summary.scope.is_none())
            );
            connection
                .try_submit(
                    ConnectionRequestId(3),
                    RuntimeCommand::ForgetAccount { account },
                )
                .unwrap();
            let refusal = completed(&mut connection, cursor, 3).await;
            assert!(
                refusal.result.is_err(),
                "failed cleanup must leave the durable marker"
            );
            std::fs::set_permissions(&file, std::fs::Permissions::from_mode(0o600)).unwrap();
            connection
                .try_submit(ConnectionRequestId(4), RuntimeCommand::Maintain { account })
                .unwrap();
            let retry = completed(&mut connection, refusal.cursor, 4).await;
            assert!(matches!(retry.result, Ok(RuntimeReply::State(ref s)) if !s.cleanup_pending));
            cursor = retry.cursor;
        } else {
            assert!(matches!(cleanup.result, Ok(RuntimeReply::State(ref s)) if !s.cleanup_pending));
        }
        assert!(!file.exists());
        connection
            .try_submit(
                ConnectionRequestId(5),
                RuntimeCommand::ForgetAccount { account },
            )
            .unwrap();
        let forgotten = completed(&mut connection, cursor, 5).await;
        assert!(matches!(forgotten.result, Ok(RuntimeReply::Forgotten(id)) if id == account));
        assert!(connection.join());
        let ledger = fixture.ledger();
        let rows = ledger.accounts().unwrap();
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].id, other);
        assert_eq!(std::fs::read(&other_file).unwrap(), other_bytes);
        assert!(
            matches!(listener.accept(), Err(error) if error.kind() == std::io::ErrorKind::WouldBlock),
            "local removal must not contact the server"
        );
    }
}
