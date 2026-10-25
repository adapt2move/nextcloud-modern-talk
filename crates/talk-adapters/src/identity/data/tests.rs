use super::*;
use crate::identity::{ledger, private_fs::PrivateDir};
use talk_core::{identity::StorageBackend, ids::LoginAttemptId};

#[path = "thumbnail_tests.rs"]
mod thumbnail_tests;

fn text<const N: usize>(value: &str) -> Text<N> {
    Text::new(value).unwrap()
}
struct Fixture {
    temporary: tempfile::TempDir,
    ledger: Arc<Mutex<Ledger>>,
    account: AccountId,
    room: NavigationRoomKey,
}
impl Fixture {
    fn new() -> Self {
        // The production owner rejects writable ancestors, including the macOS
        // per-user temporary directory. Keep fixtures under this checkout's
        // ordinary private-user parent chain without weakening that check.
        let temporary = tempfile::Builder::new()
            .prefix(".durable-cache-test-")
            .tempdir_in(env!("CARGO_MANIFEST_DIR"))
            .unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(temporary.path(), std::fs::Permissions::from_mode(0o700))
                .unwrap();
        }
        let path = temporary.path().canonicalize().unwrap();
        let dir = Arc::new(PrivateDir::open(&path, true).unwrap());
        let mut ledger = Ledger::open(dir).unwrap();
        let account = eligible(&mut ledger, "https://cache.example.invalid/");
        let key = ledger.room_key(account, 7).unwrap().unwrap();
        let fixture = Self {
            temporary,
            ledger: Arc::new(Mutex::new(ledger)),
            account,
            room: key,
        };
        fixture
            .store()
            .put_rooms(&page([room(&fixture.room, 100)]), 100)
            .unwrap();
        fixture
    }
    fn store(&self) -> Store {
        Store::new(self.ledger.clone(), self.account)
    }
    fn key(&self, id: u64) -> NavigationRoomKey {
        self.ledger
            .lock()
            .unwrap()
            .room_key(self.account, id)
            .unwrap()
            .unwrap()
    }
}
fn eligible(ledger: &mut Ledger, origin: &str) -> AccountId {
    let account = ledger.reserve(origin.into()).unwrap().id;
    ledger.bind_user(account, "synthetic-user").unwrap();
    let op = ledger
        .stage(
            account,
            LoginAttemptId::from_local(ledger::random().unwrap()),
            StorageBackend::PrivateFile,
            "synthetic-user",
        )
        .unwrap();
    ledger.settle(op, true).unwrap();
    let revision = ledger.account(account).unwrap().revision;
    ledger.activate(op, revision).unwrap();
    account
}
fn room(key: &NavigationRoomKey, saved_at: u64) -> CacheRoom {
    CacheRoom {
        key: key.clone(),
        title: text("Synthetic room"),
        kind: CachedRoomKind::Group,
        favorite: Some(true),
        archived: Some(false),
        unread: 2,
        participant_count: Some(3),
        participant_count_at: Some(saved_at),
        last_activity: Some(saved_at),
        saved_at,
    }
}
fn page<T, const N: usize>(values: impl IntoIterator<Item = T>) -> Page<T, N> {
    let mut page = Page::empty();
    for value in values {
        page.try_push(value).unwrap();
    }
    page
}
fn message(id: u64) -> CachedMessage {
    CachedMessage {
        id,
        author: text("Synthetic actor"),
        actor_key: Some(text("users:synthetic-user")),
        text: text("Synthetic message"),
        own: true,
        timestamp: 100,
        expires_at: None,
        edited: true,
        deleted: false,
        system: false,
        truncated: false,
        replyable: true,
        parent: None,
        thread: None,
        reactions: page([CachedReaction {
            emoji: text("❤️"),
            count: 1,
            own: true,
        }]),
        attachments: Page::empty(),
    }
}
fn history(key: &NavigationRoomKey, saved_at: u64) -> CacheHistory {
    CacheHistory {
        room: key.clone(),
        messages: page([message(7)]),
        pins: page([7]),
        pin_messages: Page::empty(),
        pins_loaded: true,
        older: Some(7),
        latest: Some(9),
        saved_at,
    }
}
fn record(key: &NavigationRoomKey, id: u128) -> OutboxRecord {
    OutboxRecord {
        operation: OperationId::from_local(id),
        room: key.clone(),
        kind: OutboxKind::Reaction,
        phase: OutboxPhase::Queued,
        payload: text("{\"message\":7,\"emoji\":\"❤️\",\"add\":true}"),
    }
}

#[test]
fn metadata_history_members_and_intents_survive_a_fully_reopened_profile() {
    let fixture = Fixture::new();
    let store = fixture.store();
    store
        .put_rooms(&page([room(&fixture.room, 100)]), 100)
        .unwrap();
    store
        .put_history(&history(&fixture.room, 100), 100)
        .unwrap();
    store.member_count(&fixture.room, 4, 110).unwrap();
    let first = store.enqueue(&record(&fixture.room, 1), 100).unwrap();
    let second = store.enqueue(&record(&fixture.room, 2), 99).unwrap();
    assert!(
        first.ordinal < second.ordinal,
        "FIFO uses durable insertion order, not a wall clock"
    );
    store
        .set_phase(OperationId::from_local(1), OutboxPhase::Dispatched)
        .unwrap();
    store
        .set_phase(OperationId::from_local(1), OutboxPhase::Uncertain)
        .unwrap();
    let account = fixture.account;
    let key = fixture.room.clone();
    let path = fixture.temporary.path().canonicalize().unwrap();
    drop(store);
    drop(fixture.ledger);
    let dir = Arc::new(PrivateDir::open(&path, true).unwrap());
    let ledger = Arc::new(Mutex::new(Ledger::open(dir).unwrap()));
    let reopened = Store::new(ledger, account);
    let snapshot = reopened.load(120).unwrap();
    assert_eq!(snapshot.rooms.as_slice()[0].participant_count, Some(4));
    assert_eq!(snapshot.rooms.as_slice()[0].participant_count_at, Some(110));
    assert_eq!(snapshot.histories.as_slice(), &[history(&key, 100)]);
    assert_eq!(snapshot.outbox.as_slice().len(), 2);
    assert_eq!(
        snapshot.outbox.as_slice()[0].record.phase,
        OutboxPhase::Uncertain
    );
    assert_eq!(
        snapshot.outbox.as_slice()[1].record.phase,
        OutboxPhase::Queued
    );
    assert_eq!(snapshot.outbox.as_slice()[0].ordinal, first.ordinal);
    assert!(
        reopened
            .set_phase(OperationId::from_local(1), OutboxPhase::Queued)
            .is_err(),
        "restart cannot turn an uncertain effect into an automatic replay"
    );
}

#[test]
fn queue_capacity_preserves_every_durable_record_and_duplicate_admission_is_exact() {
    let fixture = Fixture::new();
    let store = fixture.store();
    let first = store.enqueue(&record(&fixture.room, 1), 100).unwrap();
    assert_eq!(
        store.enqueue(&record(&fixture.room, 1), 101).unwrap(),
        first
    );
    let mut different = record(&fixture.room, 1);
    different.payload = text("different");
    assert_eq!(
        store.enqueue(&different, 101).unwrap_err(),
        DataError::InvalidInput
    );
    for id in 2..=OUTBOX_LIMIT as u128 {
        store.enqueue(&record(&fixture.room, id), 100).unwrap();
    }
    assert_eq!(
        store.enqueue(&record(&fixture.room, 65), 100).unwrap_err(),
        DataError::Capacity
    );
    let before = store.load(100).unwrap().outbox;
    assert_eq!(before.as_slice().len(), OUTBOX_LIMIT);
    store.acknowledge(OperationId::from_local(1)).unwrap();
    store.enqueue(&record(&fixture.room, 65), 100).unwrap();
    let after = store.load(100).unwrap().outbox;
    assert_eq!(after.as_slice().len(), OUTBOX_LIMIT);
    assert!(
        after
            .as_slice()
            .iter()
            .any(|e| e.record.operation == OperationId::from_local(65))
    );
    assert_eq!(
        after.as_slice()[0].record.operation,
        OperationId::from_local(2)
    );
}

#[test]
fn queue_byte_budget_does_not_evict_earlier_actions() {
    let fixture = Fixture::new();
    let store = fixture.store();
    for id in 1..=16 {
        let mut value = record(&fixture.room, id);
        value.payload = text(&"x".repeat(32768));
        store.enqueue(&value, 100).unwrap();
    }
    assert_eq!(
        store.enqueue(&record(&fixture.room, 17), 100).unwrap_err(),
        DataError::Capacity
    );
    assert_eq!(store.load(100).unwrap().outbox.as_slice().len(), 16);
}

#[test]
fn cache_eviction_and_expiry_leave_pending_effects_owned() {
    let fixture = Fixture::new();
    let store = fixture.store();
    let mut rooms = Page::empty();
    for id in 1..=10 {
        let key = fixture.key(id);
        rooms.try_push(room(&key, 100 + id)).unwrap();
    }
    store.put_rooms(&rooms, 110).unwrap();
    for value in rooms.as_slice() {
        store
            .put_history(&history(&value.key, value.saved_at), 110)
            .unwrap();
    }
    let mut action = record(&fixture.room, 1);
    action.phase = OutboxPhase::Queued;
    store.enqueue(&action, 100).unwrap();
    store
        .set_phase(action.operation, OutboxPhase::Dispatched)
        .unwrap();
    let snapshot = store.load(110).unwrap();
    assert_eq!(snapshot.histories.as_slice().len(), CACHE_HISTORY_LIMIT);
    let snapshot = store.load(110 + CACHE_LIFETIME).unwrap();
    assert_eq!(
        snapshot.rooms.as_slice().len(),
        1,
        "pending intent retains its bounded source metadata until settlement"
    );
    assert!(snapshot.histories.as_slice().is_empty());
    assert_eq!(snapshot.outbox.as_slice().len(), 1);
    assert_eq!(
        snapshot.outbox.as_slice()[0].record.phase,
        OutboxPhase::Dispatched
    );
}

#[test]
fn expiry_and_deletion_remove_text_reactions_quotes_and_pins() {
    let fixture = Fixture::new();
    let store = fixture.store();
    store
        .put_rooms(&page([room(&fixture.room, 100)]), 100)
        .unwrap();
    let mut expired = message(1);
    expired.expires_at = Some(100);
    let mut deleted = message(2);
    deleted.deleted = true;
    deleted.text = text("deleted canary");
    let mut reply = message(3);
    reply.parent = Some(CachedReply {
        id: 1,
        author: text("Fixture"),
        text: text("expired canary"),
        unavailable: false,
    });
    let mut value = history(&fixture.room, 100);
    value.messages = page([expired, deleted, reply]);
    value.pins = page([1, 2, 3]);
    store.put_history(&value, 100).unwrap();
    let snapshot = store.load(100).unwrap();
    let history = &snapshot.histories.as_slice()[0];
    assert_eq!(history.messages.as_slice().len(), 2);
    assert!(history.messages.as_slice()[0].text.as_str().is_empty());
    assert!(
        history.messages.as_slice()[0]
            .reactions
            .as_slice()
            .is_empty()
    );
    let parent = history.messages.as_slice()[1].parent.as_ref().unwrap();
    assert!(parent.unavailable && parent.text.as_str().is_empty());
    assert_eq!(history.pins.as_slice(), &[3]);
    let (_ledger, db) = store.connection().unwrap();
    let bytes: Vec<u8> = db
        .query_row("SELECT bytes FROM histories", [], |row| row.get(0))
        .unwrap();
    let encoded = String::from_utf8(bytes).unwrap();
    assert!(!encoded.contains("deleted canary") && !encoded.contains("expired canary"));
}

#[test]
fn messages_that_expire_after_commit_are_redacted_from_durable_bytes_on_load() {
    let fixture = Fixture::new();
    let store = fixture.store();
    let mut expired = message(1);
    expired.text = text("future expiration canary");
    expired.expires_at = Some(110);
    let mut pinned = message(2);
    pinned.text = text("pin expiration canary");
    pinned.expires_at = Some(110);
    let mut value = history(&fixture.room, 100);
    value.messages = page([expired]);
    value.pin_messages = page([pinned]);
    value.pins = page([2]);
    store.put_history(&value, 100).unwrap();
    let snapshot = store.load(120).unwrap();
    assert!(
        snapshot.histories.as_slice()[0]
            .messages
            .as_slice()
            .is_empty()
    );
    assert!(
        snapshot.histories.as_slice()[0]
            .pin_messages
            .as_slice()
            .is_empty()
    );
    let (_ledger, db) = store.connection().unwrap();
    let bytes: Vec<u8> = db
        .query_row("SELECT bytes FROM histories", [], |r| r.get(0))
        .unwrap();
    assert!(
        !String::from_utf8(bytes)
            .unwrap()
            .contains("expiration canary")
    );
}

#[test]
fn intent_admission_requires_confirmed_source_metadata_and_explicit_settlement_releases_retention()
{
    let fixture = Fixture::new();
    let store = fixture.store();
    let unknown = fixture.key(99);
    assert_eq!(
        store.enqueue(&record(&unknown, 1), 100).unwrap_err(),
        DataError::InvalidInput
    );
    store.enqueue(&record(&fixture.room, 1), 100).unwrap();
    assert_eq!(
        store
            .load(100 + CACHE_LIFETIME)
            .unwrap()
            .rooms
            .as_slice()
            .len(),
        1
    );
    store.acknowledge(OperationId::from_local(1)).unwrap();
    assert!(
        store
            .load(100 + CACHE_LIFETIME)
            .unwrap()
            .rooms
            .as_slice()
            .is_empty()
    );
}

#[test]
fn complete_policy_snapshot_atomically_retires_omitted_cache_without_losing_pending_effects() {
    let fixture = Fixture::new();
    let store = fixture.store();
    store
        .put_history(&history(&fixture.room, 100), 100)
        .unwrap();
    let action = record(&fixture.room, 1);
    let pending = store.enqueue(&action, 100).unwrap();
    let mut invalid = room(&fixture.key(22), 100);
    invalid.kind = CachedRoomKind::Unknown;
    assert_eq!(
        store.put_rooms(&page([invalid]), 100),
        Err(DataError::InvalidInput)
    );
    assert_eq!(
        store.load(100).unwrap().histories.as_slice().len(),
        1,
        "failed replacement rolls back removals"
    );
    store.put_rooms(&Page::empty(), 100).unwrap();
    let snapshot = store.load(100).unwrap();
    assert!(snapshot.rooms.as_slice().is_empty() && snapshot.histories.as_slice().is_empty());
    assert_eq!(snapshot.outbox.as_slice(), &[pending]);
    assert_eq!(
        store.enqueue(&record(&fixture.room, 2), 100).unwrap_err(),
        DataError::InvalidInput
    );
}

#[test]
fn plain_logout_purges_cache_and_outbox_before_same_account_can_be_reauthorized() {
    let fixture = Fixture::new();
    let store = fixture.store();
    store
        .put_history(&history(&fixture.room, 100), 100)
        .unwrap();
    store.enqueue(&record(&fixture.room, 1), 100).unwrap();
    let path = fixture.temporary.path().join(filename(fixture.account));
    assert!(path.is_file());
    fixture
        .ledger
        .lock()
        .unwrap()
        .suppress(fixture.account, false, None, false)
        .unwrap();
    assert!(!path.exists());
    assert_eq!(store.load(100).unwrap_err(), DataError::Ineligible);
    {
        let mut ledger = fixture.ledger.lock().unwrap();
        let operation = ledger
            .stage(
                fixture.account,
                LoginAttemptId::from_local(ledger::random().unwrap()),
                StorageBackend::PrivateFile,
                "synthetic-user",
            )
            .unwrap();
        ledger.settle(operation, true).unwrap();
        let revision = ledger.account(fixture.account).unwrap().revision;
        ledger.activate(operation, revision).unwrap();
    }
    let snapshot = store.load(100).unwrap();
    assert!(
        snapshot.rooms.as_slice().is_empty()
            && snapshot.histories.as_slice().is_empty()
            && snapshot.outbox.as_slice().is_empty()
    );
}

#[test]
fn account_keys_and_suppression_fence_all_cache_and_queue_access() {
    let fixture = Fixture::new();
    let store = fixture.store();
    let other = eligible(
        &mut fixture.ledger.lock().unwrap(),
        "https://other.example.invalid/",
    );
    let other_key = fixture
        .ledger
        .lock()
        .unwrap()
        .room_key(other, 7)
        .unwrap()
        .unwrap();
    assert_eq!(
        store.put_rooms(&page([room(&other_key, 100)]), 100),
        Err(DataError::InvalidInput)
    );
    assert_eq!(
        store.enqueue(&record(&other_key, 1), 100).unwrap_err(),
        DataError::InvalidInput
    );
    store
        .put_rooms(&page([room(&fixture.room, 100)]), 100)
        .unwrap();
    store.enqueue(&record(&fixture.room, 1), 100).unwrap();
    let path = fixture.temporary.path().join(filename(fixture.account));
    assert!(path.is_file());
    fixture
        .ledger
        .lock()
        .unwrap()
        .suppress(fixture.account, true, None, false)
        .unwrap();
    assert!(!path.exists());
    assert_eq!(store.load(100).unwrap_err(), DataError::Ineligible);
    assert_eq!(
        store.enqueue(&record(&fixture.room, 2), 100).unwrap_err(),
        DataError::Ineligible
    );
}

#[test]
fn malformed_or_future_records_fail_without_silently_dropping_durable_intents() {
    let fixture = Fixture::new();
    let store = fixture.store();
    store.enqueue(&record(&fixture.room, 1), 100).unwrap();
    {
        let (_ledger, db) = store.connection().unwrap();
        db.execute("UPDATE outbox SET payload=zeroblob(32769)", [])
            .unwrap();
    }
    assert_eq!(store.load(100).unwrap_err(), DataError::Corrupt);
    {
        let (_ledger, db) = store.connection().unwrap();
        db.execute("UPDATE outbox SET payload='{}'", []).unwrap();
        db.execute_batch("PRAGMA user_version=4").unwrap();
    }
    assert_eq!(store.load(100).unwrap_err(), DataError::Corrupt);
}

#[test]
fn serializer_and_decoder_enforce_text_and_collection_limits() {
    let fixture = Fixture::new();
    let value = history(&fixture.room, 100);
    assert_eq!(codec::encode(&value, 32).unwrap_err(), DataError::Capacity);
    let mut raw: serde_json::Value =
        serde_json::from_slice(&codec::encode(&value, HISTORY_BYTES).unwrap()).unwrap();
    raw["messages"][0]["text"] = serde_json::json!("x".repeat(4097));
    let encoded = serde_json::to_vec(&raw).unwrap();
    assert_eq!(
        codec::decode::<CacheHistory>(&encoded, HISTORY_BYTES).unwrap_err(),
        DataError::Corrupt
    );
    raw["messages"][0]["text"] = serde_json::json!("bounded");
    let message = raw["messages"][0].clone();
    raw["messages"] = serde_json::json!(vec![message; 101]);
    let encoded = serde_json::to_vec(&raw).unwrap();
    assert_eq!(
        codec::decode::<CacheHistory>(&encoded, HISTORY_BYTES).unwrap_err(),
        DataError::Corrupt
    );
}

#[test]
fn failed_message_bodies_survive_reopen_and_cannot_be_turned_back_into_automatic_dispatch() {
    let fixture = Fixture::new();
    let store = fixture.store();
    for (index, intent) in [
        OutboxIntent::Send {
            body: text("rejected send retained"),
            options: Default::default(),
        },
        OutboxIntent::Edit {
            message: 7,
            body: text("rejected edit retained"),
        },
        OutboxIntent::Schedule {
            body: text("rejected schedule retained"),
            timestamp: 500,
            options: Default::default(),
        },
    ]
    .into_iter()
    .enumerate()
    {
        let operation = OperationId::from_local(100 + index as u128);
        let record = OutboxRecord::from_intent(operation, fixture.room.clone(), &intent).unwrap();
        store.enqueue(&record, 100).unwrap();
        store.set_phase(operation, OutboxPhase::Dispatched).unwrap();
        store.set_phase(operation, OutboxPhase::Failed).unwrap();
        assert_eq!(
            store.set_phase(operation, OutboxPhase::Queued).unwrap_err(),
            DataError::InvalidInput
        );
        assert_eq!(
            store
                .set_phase(operation, OutboxPhase::Dispatched)
                .unwrap_err(),
            DataError::InvalidInput
        );
        let reopened = fixture.store().load(101).unwrap();
        let retained = reopened
            .outbox
            .as_slice()
            .iter()
            .find(|entry| entry.record.operation == operation)
            .unwrap();
        assert_eq!(retained.record.phase, OutboxPhase::Failed);
        assert_eq!(retained.record.intent().unwrap(), intent);
    }
}

#[test]
fn an_exact_retry_replaces_its_failed_source_atomically_at_full_capacity_and_after_ack_loss() {
    let fixture = Fixture::new();
    let store = fixture.store();
    let previous = OperationId::from_local(1);
    for id in 1..=OUTBOX_LIMIT as u128 {
        let record = OutboxRecord::from_intent(
            OperationId::from_local(id),
            fixture.room.clone(),
            &OutboxIntent::Send {
                body: text("source body"),
                options: Default::default(),
            },
        )
        .unwrap();
        store.enqueue(&record, 100).unwrap();
    }
    store.set_phase(previous, OutboxPhase::Failed).unwrap();
    let corrected = OutboxRecord::from_intent(
        OperationId::from_local(1000),
        fixture.room.clone(),
        &OutboxIntent::Send {
            body: text("source body"),
            options: Default::default(),
        },
    )
    .unwrap();
    let accepted = store.replace(previous, &corrected, 101).unwrap();
    // An acknowledgement can disappear after COMMIT. Repeating the same
    // replacement must observe the new record, never enqueue it twice.
    assert_eq!(
        fixture.store().replace(previous, &corrected, 102).unwrap(),
        accepted
    );
    let loaded = fixture.store().load(102).unwrap().outbox;
    assert_eq!(loaded.as_slice().len(), OUTBOX_LIMIT);
    assert!(
        !loaded
            .as_slice()
            .iter()
            .any(|entry| entry.record.operation == previous)
    );
    assert_eq!(
        loaded
            .as_slice()
            .iter()
            .filter(|entry| entry.record.operation == corrected.operation)
            .count(),
        1
    );
    assert_eq!(
        loaded
            .as_slice()
            .iter()
            .find(|entry| entry.record.operation == corrected.operation)
            .unwrap()
            .record
            .intent()
            .unwrap(),
        corrected.intent().unwrap()
    );
    let changed = OutboxRecord::from_intent(
        corrected.operation,
        fixture.room.clone(),
        &OutboxIntent::Send {
            body: text("conflicting second body"),
            options: Default::default(),
        },
    )
    .unwrap();
    assert_eq!(
        store.replace(previous, &changed, 103).unwrap_err(),
        DataError::InvalidInput
    );
}

#[test]
fn draft_replacement_rejects_unsettled_sources_and_wrong_room_kind_or_operation() {
    let fixture = Fixture::new();
    let store = fixture.store();
    let old = OutboxRecord::from_intent(
        OperationId::from_local(1),
        fixture.room.clone(),
        &OutboxIntent::Send {
            body: text("old body"),
            options: Default::default(),
        },
    )
    .unwrap();
    let corrected = OutboxRecord::from_intent(
        OperationId::from_local(2),
        fixture.room.clone(),
        &OutboxIntent::Send {
            body: text("old body"),
            options: Default::default(),
        },
    )
    .unwrap();
    store.enqueue(&old, 100).unwrap();
    assert_eq!(
        store.replace(old.operation, &corrected, 101).unwrap_err(),
        DataError::InvalidInput
    );
    store
        .set_phase(old.operation, OutboxPhase::Dispatched)
        .unwrap();
    assert_eq!(
        store.replace(old.operation, &corrected, 101).unwrap_err(),
        DataError::InvalidInput
    );
    store
        .set_phase(old.operation, OutboxPhase::Uncertain)
        .unwrap();
    assert_eq!(
        store.replace(old.operation, &corrected, 101).unwrap_err(),
        DataError::InvalidInput
    );
    assert_eq!(
        store
            .set_phase(old.operation, OutboxPhase::Failed)
            .unwrap_err(),
        DataError::InvalidInput
    );
    let old = OutboxRecord {
        operation: OperationId::from_local(10),
        phase: OutboxPhase::Queued,
        ..old
    };
    store.enqueue(&old, 101).unwrap();
    store
        .set_phase(old.operation, OutboxPhase::Dispatched)
        .unwrap();
    store.set_phase(old.operation, OutboxPhase::Failed).unwrap();
    let changed_payload = OutboxRecord::from_intent(
        OperationId::from_local(3),
        fixture.room.clone(),
        &OutboxIntent::Send {
            body: text("different edited text"),
            options: Default::default(),
        },
    )
    .unwrap();
    assert_eq!(
        store
            .replace(old.operation, &changed_payload, 102)
            .unwrap_err(),
        DataError::InvalidInput
    );
    let other_room = fixture.key(8);
    store
        .put_rooms(
            &page([room(&fixture.room, 101), room(&other_room, 101)]),
            101,
        )
        .unwrap();
    let wrong_room = OutboxRecord::from_intent(
        OperationId::from_local(3),
        other_room,
        &OutboxIntent::Send {
            body: text("old body"),
            options: Default::default(),
        },
    )
    .unwrap();
    assert_eq!(
        store.replace(old.operation, &wrong_room, 102).unwrap_err(),
        DataError::InvalidInput
    );
    let wrong_kind = OutboxRecord::from_intent(
        OperationId::from_local(3),
        fixture.room.clone(),
        &OutboxIntent::Delete { message: 7 },
    )
    .unwrap();
    assert_eq!(
        store.replace(old.operation, &wrong_kind, 102).unwrap_err(),
        DataError::InvalidInput
    );
    let same_operation = OutboxRecord::from_intent(
        old.operation,
        fixture.room.clone(),
        &OutboxIntent::Send {
            body: text("old body"),
            options: Default::default(),
        },
    )
    .unwrap();
    assert_eq!(
        store
            .replace(old.operation, &same_operation, 102)
            .unwrap_err(),
        DataError::InvalidInput
    );
    let retained = store.load(102).unwrap().outbox;
    assert_eq!(retained.as_slice().len(), 2);
    let retained = retained
        .as_slice()
        .iter()
        .find(|entry| entry.record.operation == old.operation)
        .unwrap();
    assert_eq!(retained.record.intent().unwrap(), old.intent().unwrap());
}

#[test]
fn replacement_insert_failure_rolls_back_the_retirement_of_the_failed_source() {
    let fixture = Fixture::new();
    let store = fixture.store();
    let intent = OutboxIntent::Send {
        body: text("never lose this failed draft"),
        options: Default::default(),
    };
    let previous = OperationId::from_local(1);
    let old = OutboxRecord::from_intent(previous, fixture.room.clone(), &intent).unwrap();
    store.enqueue(&old, 100).unwrap();
    store.set_phase(previous, OutboxPhase::Failed).unwrap();
    let replacement =
        OutboxRecord::from_intent(OperationId::from_local(2), fixture.room.clone(), &intent)
            .unwrap();
    {
        let (_ledger, db) = store.connection().unwrap();
        db.execute_batch("CREATE TRIGGER reject_synthetic_replacement BEFORE INSERT ON outbox BEGIN SELECT RAISE(ABORT,'synthetic insert failure'); END;").unwrap();
    }
    assert_eq!(
        store.replace(previous, &replacement, 101).unwrap_err(),
        DataError::Unavailable
    );
    let reopened = fixture.store().load(101).unwrap().outbox;
    assert_eq!(reopened.as_slice().len(), 1);
    assert_eq!(reopened.as_slice()[0].record.operation, previous);
    assert_eq!(reopened.as_slice()[0].record.phase, OutboxPhase::Failed);
    assert_eq!(reopened.as_slice()[0].record.intent().unwrap(), intent);
}
