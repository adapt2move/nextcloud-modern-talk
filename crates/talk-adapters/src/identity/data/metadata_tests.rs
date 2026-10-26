use super::*;

fn observation(at: u64) -> CacheRoomObservation {
    CacheRoomObservation {
        description: Some(text("Synthetic description")),
        read_only: Some(true),
        notifications: Some(CachedNotificationLevel::Mention),
        call_notifications: Some(false),
        important: Some(true),
        preserved: Some(false),
        message_expiration: Some(3600),
        lobby: Some(true),
        guest_access: Some(false),
        observed_at: at,
    }
}

#[test]
fn legacy_room_decodes_without_details_and_observation_schema_has_no_authority_fields() {
    let fixture = Fixture::new();
    let base = room(&fixture.room, 100);
    let legacy = codec::encode(&base, META_BYTES).unwrap();
    assert!(
        !std::str::from_utf8(&legacy)
            .unwrap()
            .contains("observation")
    );
    let restored: CacheRoom = codec::decode(&legacy, META_BYTES).unwrap();
    assert_eq!(restored.observation, None);
    let mut value = base;
    assert!(value.retain_observation(observation(100)));
    let encoded = codec::encode(&value, META_BYTES).unwrap();
    let mut json: serde_json::Value = serde_json::from_slice(&encoded).unwrap();
    for prohibited in [
        "controls",
        "permissions",
        "effective_permissions",
        "role",
        "members",
        "bots",
        "guest_link",
        "password",
        "token",
    ] {
        assert!(json["observation"].get(prohibited).is_none());
        json["observation"][prohibited] = serde_json::json!("synthetic forbidden field");
        assert!(
            codec::decode::<CacheRoom>(&serde_json::to_vec(&json).unwrap(), META_BYTES).is_err()
        );
        json["observation"]
            .as_object_mut()
            .unwrap()
            .remove(prohibited);
    }
}

#[test]
fn detail_observations_survive_profile_reopen_and_independent_expiry_scrubs_bytes() {
    let fixture = Fixture::new();
    let mut value = room(&fixture.room, 100);
    assert!(value.retain_observation(observation(100)));
    fixture
        .store()
        .put_rooms(&page([value.clone()]), 100)
        .unwrap();
    let account = fixture.account;
    let path = fixture.temporary.path().canonicalize().unwrap();
    drop(fixture.ledger);
    let dir = Arc::new(PrivateDir::open(&path, true).unwrap());
    let ledger = Arc::new(Mutex::new(Ledger::open(dir).unwrap()));
    let store = Store::new(ledger, account);
    assert_eq!(
        store.load(101).unwrap().rooms.as_slice()[0].observation,
        value.observation
    );
    let later = 100 + CACHE_LIFETIME - 1;
    value.unread = 77;
    value.last_activity = Some(later);
    value.saved_at = later;
    store.put_rooms(&page([value]), later).unwrap();
    let snapshot = store.load(later + 1).unwrap();
    assert_eq!(snapshot.rooms.as_slice().len(), 1);
    assert_eq!(snapshot.rooms.as_slice()[0].unread, 77);
    assert!(snapshot.rooms.as_slice()[0].observation.is_none());
    let (_ledger, db) = store.connection().unwrap();
    let bytes: Vec<u8> = db
        .query_row("SELECT bytes FROM rooms", [], |r| r.get(0))
        .unwrap();
    assert!(
        !std::str::from_utf8(&bytes)
            .unwrap()
            .contains("Synthetic description")
    );
}

#[test]
fn oversized_escaped_details_are_omitted_without_losing_complete_room_policy() {
    let fixture = Fixture::new();
    let mut value = room(&fixture.room, 100);
    let mut large = observation(100);
    large.description = Some(text(&"\u{0000}".repeat(4000)));
    assert!(!value.retain_observation(large.clone()));
    assert!(value.observation.is_none());
    // The storage boundary checks its own budget even if a producer skipped the helper.
    value.observation = Some(large);
    let other = room(&fixture.key(8), 100);
    fixture
        .store()
        .put_rooms(&page([value, other]), 100)
        .unwrap();
    let snapshot = fixture.store().load(100).unwrap();
    assert_eq!(snapshot.rooms.as_slice().len(), 2);
    assert!(
        snapshot
            .rooms
            .as_slice()
            .iter()
            .all(|r| r.observation.is_none())
    );
    let store = fixture.store();
    let (_ledger, db) = store.connection().unwrap();
    let maximum: i64 = db
        .query_row("SELECT max(length(bytes)) FROM rooms", [], |r| r.get(0))
        .unwrap();
    assert!(maximum <= META_BYTES as i64);
    let pages: i64 = db
        .query_row("PRAGMA max_page_count", [], |r| r.get(0))
        .unwrap();
    assert_eq!(pages * 4096, 16 * 1024 * 1024);
}

#[test]
fn scalar_observation_revocation_and_account_binding_survive_reopen() {
    let fixture = Fixture::new();
    let mut value = room(&fixture.room, 100);
    value.retain_observation(observation(100));
    fixture.store().put_rooms(&page([value]), 100).unwrap();
    let other = eligible(
        &mut fixture.ledger.lock().unwrap(),
        "https://other-cache.example.invalid/",
    );
    let foreign = Store::new(fixture.ledger.clone(), other);
    assert_eq!(
        foreign.put_rooms(&page([room(&fixture.room, 100)]), 100),
        Err(DataError::InvalidInput)
    );
    let mut value = fixture.store().load(101).unwrap().rooms.as_slice()[0].clone();
    value.observation = None;
    fixture.store().put_rooms(&page([value]), 101).unwrap();
    assert!(
        fixture.store().load(102).unwrap().rooms.as_slice()[0]
            .observation
            .is_none()
    );
    fixture.store().put_rooms(&Page::empty(), 102).unwrap();
    assert!(
        fixture
            .store()
            .load(103)
            .unwrap()
            .rooms
            .as_slice()
            .is_empty()
    );
}
