use super::*;
use crate::identity::{
    AccountConnection, ConnectionCompletion, ConnectionCursor, ConnectionRequestId,
    ConnectionState, RuntimeCommand, RuntimeConfig, RuntimeReply, SavedAccountState,
};
use std::sync::atomic::Ordering;
use talk_core::{
    bounded::Text,
    identity::StorageChoice,
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
