use super::*;

fn status(at: u64) -> CacheUserStatus {
    CacheUserStatus {
        presence: CachedPresence::Away,
        icon: Some(text("☕")),
        message: Some(text("Synthetic status")),
        observed_at: at,
    }
}

#[test]
fn direct_status_survives_reopen_but_expires_and_scrubs_its_bytes() {
    let fixture = Fixture::new();
    let mut value = room(&fixture.room, 100);
    value.kind = CachedRoomKind::Direct;
    assert!(value.retain_user_status(status(100)));
    fixture
        .store()
        .put_rooms(&page([value.clone()]), 100)
        .unwrap();
    let account = fixture.account;
    let path = fixture.temporary.path().canonicalize().unwrap();
    drop(fixture.ledger);
    let dir = Arc::new(PrivateDir::open(&path, true).unwrap());
    let store = Store::new(Arc::new(Mutex::new(Ledger::open(dir).unwrap())), account);
    assert_eq!(
        store.load(101).unwrap().rooms.as_slice()[0].user_status,
        value.user_status
    );
    let later = 100 + CACHE_PRESENCE_LIFETIME;
    assert!(
        store.load(later).unwrap().rooms.as_slice()[0]
            .user_status
            .is_none()
    );
    let (_ledger, db) = store.connection().unwrap();
    let bytes: Vec<u8> = db
        .query_row("SELECT bytes FROM rooms", [], |row| row.get(0))
        .unwrap();
    assert!(
        !std::str::from_utf8(&bytes)
            .unwrap()
            .contains("Synthetic status")
    );
}

#[test]
fn other_rooms_and_future_statuses_do_not_persist_status() {
    let fixture = Fixture::new();
    let mut value = room(&fixture.room, 100);
    assert!(value.retain_user_status(status(100)));
    fixture
        .store()
        .put_rooms(&page([value.clone()]), 100)
        .unwrap();
    assert!(
        fixture.store().load(101).unwrap().rooms.as_slice()[0]
            .user_status
            .is_none()
    );
    value.kind = CachedRoomKind::Direct;
    assert!(value.retain_user_status(status(102)));
    fixture.store().put_rooms(&page([value]), 101).unwrap();
    assert!(
        fixture.store().load(101).unwrap().rooms.as_slice()[0]
            .user_status
            .is_none()
    );
}
