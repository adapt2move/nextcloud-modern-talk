//! Synthetic native-worker scheduling tests. No machine credential store or
//! external account is touched: an explicit private-file fixture is gated.
use super::*;
use crate::identity::{
    CacheRoom, CachedRoomKind, DataCommand, DataError, DataResult, OutboxIntent, OutboxRecord,
};
use std::sync::{Arc, Mutex};
use talk_core::{bounded::Page, ids::OperationId};

#[tokio::test]
async fn cache_reads_and_durable_admission_bypass_a_blocked_credential_restore_and_logout_fences_late_results()
 {
    let server = Server::start(35).await;
    let fixture = Fixture::new();
    let runtime =
        IdentityRuntime::start(RuntimeConfig::isolated_fixture(fixture.0.clone())).unwrap();
    let (account, attempt) = login(&runtime, &server, None).await;
    command(
        &runtime,
        RuntimeCommand::Save {
            account,
            attempt,
            choice: StorageChoice::PrivateFile,
        },
    )
    .await
    .unwrap();
    assert!(runtime.join());
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_secs();
    let key = {
        let mut ledger = fixture.ledger();
        let key = ledger.room_key(account, 7).unwrap().unwrap();
        let ledger = Arc::new(Mutex::new(ledger));
        let store = crate::identity::data::Store::new(ledger, account);
        let mut rooms = Page::empty();
        rooms
            .try_push(CacheRoom {
                key: key.clone(),
                title: Text::new("Synthetic room").unwrap(),
                kind: CachedRoomKind::Group,
                favorite: Some(false),
                archived: Some(false),
                unread: 0,
                participant_count: Some(3),
                participant_count_at: Some(now),
                last_activity: Some(now),
                saved_at: now,
            })
            .unwrap();
        store.put_rooms(&rooms, now).unwrap();
        key
    };
    let config = RuntimeConfig::isolated_fixture(fixture.0.clone());
    let faults = config.faults.clone();
    let gate = ReleaseGate(faults.arm_read());
    let mut connection = AccountConnection::start(config).unwrap();
    ready(&mut connection).await;
    connection
        .try_submit(ConnectionRequestId(1), RuntimeCommand::Restore { account })
        .unwrap();
    entered(&gate.0).await;
    assert!(!gate.0.completed.load(Ordering::Acquire));
    // Restore holds one of seven ordinary account credits. These six requests
    // remain behind its provider gate, so no timing assumption can free a slot.
    for request in 2..=7 {
        connection
            .try_submit(
                ConnectionRequestId(request),
                RuntimeCommand::StorageStatus { account },
            )
            .unwrap();
    }
    assert!(
        matches!(
            connection.try_submit(
                ConnectionRequestId(8),
                RuntimeCommand::StorageStatus { account }
            ),
            Err(IdentityError::Request(ErrorKind::Busy))
        ),
        "all seven ordinary account credits are occupied"
    );
    connection
        .try_submit(
            ConnectionRequestId(8),
            RuntimeCommand::AccountData {
                account,
                command: DataCommand::Load,
            },
        )
        .unwrap();
    let loaded = tokio::time::timeout(
        std::time::Duration::from_secs(2),
        completed(&mut connection, ConnectionCursor::default(), 8),
    )
    .await
    .expect("disk read must complete while the provider remains held");
    assert!(
        matches!(&loaded.result,Ok(RuntimeReply::AccountData{account:returned,result:Ok(DataResult::Snapshot(snapshot))}) if *returned==account&&snapshot.rooms.as_slice().len()==1)
    );
    assert!(
        !gate.0.completed.load(Ordering::Acquire),
        "the credential provider is still blocked, not merely slow"
    );
    assert_eq!(loaded.captured_scope, None);
    assert!(!loaded.scope_is_current);
    connection.cancel_request(ConnectionRequestId(8)).unwrap();
    let cancelled = connection.poll(ConnectionCursor::default()).unwrap();
    assert!(
        cancelled
            .completions
            .as_slice()
            .iter()
            .find(|d| d.request == ConnectionRequestId(8))
            .unwrap()
            .cancel_requested,
        "composition must discard the exact cancelled result even if disk I/O already settled"
    );
    let record = OutboxRecord::from_intent(
        OperationId::from_local(1),
        key,
        &OutboxIntent::Delete { message: 7 },
    )
    .unwrap();
    connection
        .try_submit(
            ConnectionRequestId(9),
            RuntimeCommand::AccountData {
                account,
                command: DataCommand::Enqueue(record),
            },
        )
        .unwrap();
    let queued = tokio::time::timeout(
        std::time::Duration::from_secs(2),
        completed(&mut connection, ConnectionCursor::default(), 9),
    )
    .await
    .expect("durable admission must not wait on credential materialization");
    assert!(matches!(
        queued.result,
        Ok(RuntimeReply::AccountData {
            result: Ok(DataResult::Queued(_)),
            ..
        })
    ));
    assert!(!gate.0.completed.load(Ordering::Acquire));
    // Both completed data observations are deliberately unacknowledged. Their
    // reserved credits remain charged until composition acknowledges the cursor.
    assert!(
        matches!(
            connection.try_submit(
                ConnectionRequestId(10),
                RuntimeCommand::AccountData {
                    account,
                    command: DataCommand::Load
                }
            ),
            Err(IdentityError::Request(ErrorKind::Busy))
        ),
        "a third reserved data admission is bounded even after its predecessors commit"
    );
    connection.poll(queued.cursor).unwrap();
    connection.logout(account).unwrap();
    if let Ok(()) = connection.try_submit(
        ConnectionRequestId(11),
        RuntimeCommand::AccountData {
            account,
            command: DataCommand::Load,
        },
    ) {
        let denied = completed(&mut connection, queued.cursor, 11).await;
        assert!(
            matches!(
                denied.result,
                Err(_)
                    | Ok(RuntimeReply::AccountData {
                        result: Err(DataError::Ineligible),
                        ..
                    })
            ),
            "logout must not return cache data in the retired eligibility lifetime"
        );
    }
    gate.0.release();
    let restored = completed(&mut connection, queued.cursor, 1).await;
    assert!(!restored.scope_is_current);
    assert!(
        restored.result.is_err(),
        "the original provider result cannot activate after logout"
    );
    assert!(connection.join());
    assert_eq!(fixture.ledger().account(account).unwrap().disposition, 0);
    assert!(
        !fixture
            .0
            .join(format!("data-{:032x}.sqlite", account.as_local()))
            .exists(),
        "plain logout removes the private data file after suppression"
    );
}
