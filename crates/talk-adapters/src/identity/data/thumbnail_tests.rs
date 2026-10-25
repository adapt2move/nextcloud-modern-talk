//! Real SQLite/reopen fixtures with authored RGB PNGs; no external account or
//! OS credential provider. These establish store behavior, not native decoding.
use super::*;
use std::io::Write;

fn crc(bytes: &[u8]) -> u32 {
    let mut value = u32::MAX;
    for byte in bytes {
        value ^= u32::from(*byte);
        for _ in 0..8 {
            value = (value >> 1) ^ (0xedb88320u32 & (0u32.wrapping_sub(value & 1)));
        }
    }
    !value
}
fn chunk(target: &mut Vec<u8>, kind: &[u8; 4], bytes: &[u8]) {
    target.extend_from_slice(&(bytes.len() as u32).to_be_bytes());
    let start = target.len();
    target.extend_from_slice(kind);
    target.extend_from_slice(bytes);
    target.extend_from_slice(&crc(&target[start..]).to_be_bytes());
}
fn png(width: u32, height: u32) -> Arc<[u8]> {
    let mut result = b"\x89PNG\r\n\x1a\n".to_vec();
    let mut header = Vec::new();
    header.extend_from_slice(&width.to_be_bytes());
    header.extend_from_slice(&height.to_be_bytes());
    header.extend_from_slice(&[8, 2, 0, 0, 0]);
    chunk(&mut result, b"IHDR", &header);
    let mut encoder = flate2::write::ZlibEncoder::new(Vec::new(), flate2::Compression::none());
    for row in 0..height {
        encoder.write_all(&[0]).unwrap();
        for col in 0..width {
            encoder.write_all(&[row as u8, col as u8, 42]).unwrap();
        }
    }
    chunk(&mut result, b"IDAT", &encoder.finish().unwrap());
    chunk(&mut result, b"IEND", &[]);
    result.into()
}
fn thumbnail(fixture: &Fixture, id: u64, saved_at: u64) -> CacheThumbnail {
    CacheThumbnail {
        room: fixture.room.clone(),
        message: id,
        placeholder: text("image"),
        bytes: png(2, 2),
        saved_at,
        expires_at: None,
        attachment: media_message(id).attachments.as_slice()[0].clone(),
    }
}
fn media_message(id: u64) -> CachedMessage {
    CachedMessage {
        attachments: page([CachedAttachment {
            placeholder: text("image"),
            name: text("Synthetic.png"),
            mime_type: Some(text("image/png")),
            size: Some(100),
            id: Some(text("42")),
        }]),
        ..message(id)
    }
}

#[test]
fn preview_survives_reopen_has_private_permissions_and_expires_without_room_or_intent_loss() {
    let fixture = Fixture::new();
    let preview = thumbnail(&fixture, 7, 100);
    fixture
        .store()
        .execute(DataCommand::PutThumbnail(preview.clone()), 100)
        .unwrap();
    fixture
        .store()
        .enqueue(&record(&fixture.room, 1), 100)
        .unwrap();
    let path = fixture.temporary.path().to_path_buf();
    let account = fixture.account;
    drop(fixture.ledger);
    let directory = Arc::new(PrivateDir::open(&path, true).unwrap());
    let ledger = Arc::new(Mutex::new(Ledger::open(directory).unwrap()));
    let reopened = Store::new(ledger, account);
    assert_eq!(
        reopened.load(101).unwrap().thumbnails.as_slice(),
        &[preview]
    );
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        assert_eq!(
            std::fs::metadata(&path).unwrap().permissions().mode() & 0o777,
            0o700
        );
        assert_eq!(
            std::fs::metadata(path.join(filename(account)))
                .unwrap()
                .permissions()
                .mode()
                & 0o777,
            0o600
        );
    }
    let expired = reopened.load(100 + CACHE_THUMBNAIL_LIFETIME).unwrap();
    assert!(expired.thumbnails.as_slice().is_empty());
    assert_eq!(expired.rooms.as_slice().len(), 1);
    assert_eq!(expired.outbox.as_slice().len(), 1);
}

#[test]
fn previews_are_account_scoped_and_room_revocation_cascades_without_erasing_required_intents() {
    let fixture = Fixture::new();
    let preview = thumbnail(&fixture, 7, 100);
    fixture.store().put_thumbnail(&preview, 100).unwrap();
    fixture
        .store()
        .enqueue(&record(&fixture.room, 1), 100)
        .unwrap();
    let other = {
        let mut ledger = fixture.ledger.lock().unwrap();
        eligible(&mut ledger, "https://other-cache.example.invalid/")
    };
    let other = Store::new(fixture.ledger.clone(), other);
    assert!(other.load(100).unwrap().thumbnails.as_slice().is_empty());
    assert_eq!(
        other.put_thumbnail(&preview, 100),
        Err(DataError::InvalidInput)
    );
    fixture.store().remove_cached_room(&fixture.room).unwrap();
    let snapshot = fixture.store().load(100).unwrap();
    assert!(snapshot.thumbnails.as_slice().is_empty());
    assert_eq!(snapshot.outbox.as_slice().len(), 1);
    assert_eq!(
        fixture.store().put_thumbnail(&preview, 100),
        Err(DataError::InvalidInput)
    );
    fixture
        .store()
        .put_rooms(&page([room(&fixture.room, 100)]), 100)
        .unwrap();
    fixture.store().put_thumbnail(&preview, 100).unwrap();
    fixture.store().put_rooms(&Page::empty(), 100).unwrap();
    assert!(
        fixture
            .store()
            .load(100)
            .unwrap()
            .thumbnails
            .as_slice()
            .is_empty()
    );
}

#[test]
fn thumbnail_lru_and_byte_capacity_never_evict_outbox_or_room_metadata() {
    let fixture = Fixture::new();
    fixture
        .store()
        .enqueue(&record(&fixture.room, 1), 100)
        .unwrap();
    for id in 1..=24 {
        fixture
            .store()
            .put_thumbnail(&thumbnail(&fixture, id, 100), 100)
            .unwrap();
    }
    // Touch the first admitted target while every wall-clock value is equal.
    fixture
        .store()
        .put_thumbnail(&thumbnail(&fixture, 1, 100), 100)
        .unwrap();
    fixture
        .store()
        .put_thumbnail(&thumbnail(&fixture, 25, 100), 100)
        .unwrap();
    let snapshot = fixture.store().load(100).unwrap();
    assert_eq!(snapshot.thumbnails.as_slice().len(), 24);
    assert!(
        snapshot
            .thumbnails
            .as_slice()
            .iter()
            .any(|t| t.message == 1)
    );
    assert!(
        !snapshot
            .thumbnails
            .as_slice()
            .iter()
            .any(|t| t.message == 2)
    );
    let big = png(320, 240);
    assert!(big.len() < CACHE_THUMBNAIL_BYTES);
    for id in 1..=24 {
        let mut preview = thumbnail(&fixture, id, 100);
        preview.bytes = big.clone();
        fixture.store().put_thumbnail(&preview, 100).unwrap();
    }
    let snapshot = fixture.store().load(100).unwrap();
    assert!(snapshot.thumbnails.as_slice().len() < 24);
    assert!(
        snapshot
            .thumbnails
            .as_slice()
            .iter()
            .map(|t| t.bytes.len())
            .sum::<usize>()
            <= 4 * 1024 * 1024
    );
    assert_eq!(snapshot.rooms.as_slice().len(), 1);
    assert_eq!(snapshot.outbox.as_slice().len(), 1);
    let store = fixture.store();
    let (_ledger, db) = store.connection().unwrap();
    assert_eq!(
        db.query_row("PRAGMA max_page_count", [], |r| r.get::<_, i64>(0))
            .unwrap(),
        4096
    );
    assert!(
        db.query_row("PRAGMA page_count", [], |r| r.get::<_, i64>(0))
            .unwrap()
            <= 4096
    );
}

#[test]
fn invalid_thumbnail_headers_sizes_timestamps_and_expiry_are_rejected_before_persistence() {
    let fixture = Fixture::new();
    let valid = thumbnail(&fixture, 7, 100);
    for bytes in [
        Arc::from([]),
        Arc::from(vec![0; CACHE_THUMBNAIL_BYTES + 1]),
        png(321, 240),
        png(320, 241),
        Arc::from(b"<svg>remote</svg>".as_slice()),
    ] {
        let mut preview = valid.clone();
        preview.bytes = bytes;
        assert_eq!(
            fixture.store().put_thumbnail(&preview, 100),
            Err(DataError::InvalidInput)
        );
    }
    let mut future = valid.clone();
    future.saved_at = 101;
    assert_eq!(
        fixture.store().put_thumbnail(&future, 100),
        Err(DataError::InvalidInput)
    );
    let mut old = valid.clone();
    old.saved_at = 100;
    assert_eq!(
        fixture
            .store()
            .put_thumbnail(&old, 100 + CACHE_THUMBNAIL_LIFETIME),
        Err(DataError::InvalidInput)
    );
    let mut expired = valid.clone();
    expired.expires_at = Some(100);
    assert_eq!(
        fixture.store().put_thumbnail(&expired, 100),
        Err(DataError::InvalidInput)
    );
    assert!(
        fixture
            .store()
            .load(100)
            .unwrap()
            .thumbnails
            .as_slice()
            .is_empty()
    );
}

#[test]
fn observed_message_deletion_or_earlier_expiry_purges_previews_even_after_history_cleaning() {
    let fixture = Fixture::new();
    let preview = thumbnail(&fixture, 7, 100);
    fixture.store().put_thumbnail(&preview, 100).unwrap();
    let mut value = history(&fixture.room, 100);
    value.messages = page([CachedMessage {
        expires_at: Some(110),
        ..media_message(7)
    }]);
    fixture.store().put_history(&value, 100).unwrap();
    assert_eq!(
        fixture
            .store()
            .load(109)
            .unwrap()
            .thumbnails
            .as_slice()
            .len(),
        1
    );
    assert!(
        fixture
            .store()
            .load(110)
            .unwrap()
            .thumbnails
            .as_slice()
            .is_empty()
    );
    fixture.store().put_thumbnail(&preview, 110).unwrap();
    value.messages = page([CachedMessage {
        deleted: true,
        ..media_message(7)
    }]);
    value.saved_at = 110;
    fixture.store().put_history(&value, 110).unwrap();
    assert!(
        fixture
            .store()
            .load(110)
            .unwrap()
            .thumbnails
            .as_slice()
            .is_empty()
    );
}

#[test]
fn expired_or_suppressed_account_cannot_recreate_thumbnail_files() {
    let fixture = Fixture::new();
    let preview = thumbnail(&fixture, 7, 100);
    fixture.store().put_thumbnail(&preview, 100).unwrap();
    {
        let mut ledger = fixture.ledger.lock().unwrap();
        ledger.suppress(fixture.account, true, None, false).unwrap();
    }
    assert!(
        !fixture
            .temporary
            .path()
            .join(filename(fixture.account))
            .exists()
    );
    assert_eq!(
        fixture.store().put_thumbnail(&preview, 100),
        Err(DataError::Ineligible)
    );
    assert_eq!(fixture.store().load(100), Err(DataError::Ineligible));
    assert!(
        !fixture
            .temporary
            .path()
            .join(filename(fixture.account))
            .exists()
    );
}

#[test]
fn schema_one_migration_keeps_required_records_and_defaults_legacy_attachment_metadata() {
    let fixture = Fixture::new();
    let store = fixture.store();
    store
        .put_history(&history(&fixture.room, 100), 100)
        .unwrap();
    store.enqueue(&record(&fixture.room, 1), 100).unwrap();
    {
        let (_ledger, db) = store.connection().unwrap();
        let bytes: Vec<u8> = db
            .query_row("SELECT bytes FROM histories", [], |r| r.get(0))
            .unwrap();
        let mut legacy: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
        legacy["messages"][0]
            .as_object_mut()
            .unwrap()
            .remove("attachments");
        db.execute(
            "UPDATE histories SET bytes=?1",
            [serde_json::to_vec(&legacy).unwrap()],
        )
        .unwrap();
        db.execute_batch("DROP TABLE thumbnails; PRAGMA user_version=1;")
            .unwrap();
    }
    let loaded = store.load(100).unwrap();
    assert_eq!(loaded.histories.as_slice().len(), 1);
    assert!(
        loaded.histories.as_slice()[0].messages.as_slice()[0]
            .attachments
            .as_slice()
            .is_empty()
    );
    assert_eq!(loaded.outbox.as_slice().len(), 1);
    assert!(loaded.thumbnails.as_slice().is_empty());
    let (_ledger, db) = store.connection().unwrap();
    assert_eq!(
        db.query_row("PRAGMA user_version", [], |r| r.get::<_, i64>(0))
            .unwrap(),
        3
    );
}

#[test]
fn corrupted_disposable_preview_is_pruned_without_blocking_required_outbox_load() {
    let fixture = Fixture::new();
    let store = fixture.store();
    store
        .put_thumbnail(&thumbnail(&fixture, 7, 100), 100)
        .unwrap();
    let queued = store.enqueue(&record(&fixture.room, 1), 100).unwrap();
    {
        let (_ledger, db) = store.connection().unwrap();
        db.execute_batch("PRAGMA ignore_check_constraints=ON;")
            .unwrap();
        db.execute(
            "UPDATE thumbnails SET bytes=zeroblob(?1)",
            [CACHE_THUMBNAIL_BYTES as i64 + 1],
        )
        .unwrap();
    }
    let loaded = store.load(100).unwrap();
    assert!(loaded.thumbnails.as_slice().is_empty());
    assert_eq!(loaded.outbox.as_slice(), &[queued]);
    assert_eq!(loaded.rooms.as_slice().len(), 1);
    let (_ledger, db) = store.connection().unwrap();
    assert_eq!(
        db.query_row("SELECT count(*) FROM thumbnails", [], |r| r
            .get::<_, i64>(0))
            .unwrap(),
        0
    );
}

#[test]
fn physical_database_pressure_evicts_previews_before_required_durable_writes() {
    let fixture = Fixture::new();
    let store = fixture.store();
    let first = store.enqueue(&record(&fixture.room, 1), 100).unwrap();
    let bytes = png(320, 240);
    for id in 1..=18 {
        let mut preview = thumbnail(&fixture, id, 100);
        preview.bytes = bytes.clone();
        store.put_thumbnail(&preview, 100).unwrap();
    }
    let before = store.load(100).unwrap().thumbnails.as_slice().len();
    {
        let (_ledger, db) = store.connection().unwrap();
        // An owned synthetic table fills the real bounded SQLite file. Each
        // insert either commits or returns FULL; no size limit is changed.
        db.execute_batch("CREATE TABLE synthetic_pressure(bytes BLOB NOT NULL);")
            .unwrap();
        for _ in 0..4096 {
            if db
                .execute("INSERT INTO synthetic_pressure VALUES(zeroblob(4096))", [])
                .is_err()
            {
                break;
            }
        }
        assert!(
            db.query_row("PRAGMA page_count", [], |r| r.get::<_, i64>(0))
                .unwrap()
                <= 4096
        );
        assert!(
            db.query_row("PRAGMA freelist_count", [], |r| r.get::<_, i64>(0))
                .unwrap()
                < 16
        );
    }
    let second = store.enqueue(&record(&fixture.room, 2), 100).unwrap();
    let loaded = store.load(100).unwrap();
    assert!(loaded.thumbnails.as_slice().len() < before);
    assert_eq!(loaded.rooms.as_slice().len(), 1);
    assert_eq!(loaded.outbox.as_slice(), &[first, second]);
    let (_ledger, db) = store.connection().unwrap();
    assert!(
        db.query_row("PRAGMA page_count", [], |r| r.get::<_, i64>(0))
            .unwrap()
            <= 4096
    );
    assert_eq!(
        db.query_row("PRAGMA max_page_count", [], |r| r.get::<_, i64>(0))
            .unwrap(),
        4096
    );
}

#[test]
fn attachment_revocation_purges_its_thumbnail_and_deleted_cache_rows_drop_attachment_metadata() {
    let fixture = Fixture::new();
    let store = fixture.store();
    let mut value = history(&fixture.room, 100);
    value.messages = page([media_message(7)]);
    store.put_history(&value, 100).unwrap();
    let preview = thumbnail(&fixture, 7, 100);
    store.put_thumbnail(&preview, 100).unwrap();
    value.messages = page([message(7)]);
    store.put_history(&value, 100).unwrap();
    assert!(store.load(100).unwrap().thumbnails.as_slice().is_empty());
    assert_eq!(
        store.put_thumbnail(&preview, 100),
        Err(DataError::InvalidInput)
    );
    value.messages = page([CachedMessage {
        deleted: true,
        ..media_message(7)
    }]);
    store.put_history(&value, 100).unwrap();
    assert!(
        store.load(100).unwrap().histories.as_slice()[0]
            .messages
            .as_slice()[0]
            .attachments
            .as_slice()
            .is_empty()
    );
}

#[test]
fn exact_attachment_descriptor_replacement_purges_pixels_even_without_prior_history() {
    let fixture = Fixture::new();
    let store = fixture.store();
    let preview = thumbnail(&fixture, 7, 100);
    store.put_thumbnail(&preview, 100).unwrap();
    let mut replacement = media_message(7);
    let mut attachment = replacement.attachments.as_slice()[0].clone();
    attachment.id = Some(text("43"));
    replacement.attachments = page([attachment]);
    let mut value = history(&fixture.room, 100);
    value.messages = page([replacement]);
    store.put_history(&value, 100).unwrap();
    assert!(store.load(100).unwrap().thumbnails.as_slice().is_empty());
    assert_eq!(
        store.put_thumbnail(&preview, 100),
        Err(DataError::InvalidInput)
    );
}

#[test]
fn image_video_and_pdf_raster_previews_have_idempotent_scoped_removal() {
    let fixture = Fixture::new();
    let store = fixture.store();
    let first = store.enqueue(&record(&fixture.room, 1), 100).unwrap();
    for (id, mime) in [(7, "image/png"), (8, "video/mp4"), (9, "application/pdf")] {
        let mut preview = thumbnail(&fixture, id, 100);
        preview.attachment.mime_type = Some(text(mime));
        store
            .execute(DataCommand::PutThumbnail(preview), 100)
            .unwrap();
    }
    assert_eq!(store.load(100).unwrap().thumbnails.as_slice().len(), 3);
    let removal = DataCommand::RemoveThumbnail {
        room: fixture.room.clone(),
        message: 8,
        placeholder: text("image"),
    };
    store.execute(removal.clone(), 100).unwrap();
    store.execute(removal, 100).unwrap();
    let snapshot = store.load(100).unwrap();
    assert_eq!(snapshot.thumbnails.as_slice().len(), 2);
    assert!(
        !snapshot
            .thumbnails
            .as_slice()
            .iter()
            .any(|t| t.message == 8)
    );
    store.execute(DataCommand::ClearThumbnails, 100).unwrap();
    let snapshot = store.load(100).unwrap();
    assert!(snapshot.thumbnails.as_slice().is_empty());
    assert_eq!(snapshot.outbox.as_slice(), &[first]);
    assert_eq!(snapshot.rooms.as_slice().len(), 1);
}

#[test]
fn schema_two_unbound_previews_are_discarded_without_blocking_required_records() {
    let fixture = Fixture::new();
    let store = fixture.store();
    store.enqueue(&record(&fixture.room, 1), 100).unwrap();
    store
        .put_history(&history(&fixture.room, 100), 100)
        .unwrap();
    {
        let (_ledger, db) = store.connection().unwrap();
        db.execute_batch("DROP TABLE thumbnails; CREATE TABLE thumbnails(room TEXT NOT NULL REFERENCES rooms(room) ON DELETE CASCADE,message INTEGER NOT NULL,placeholder TEXT NOT NULL,saved INTEGER NOT NULL,expires INTEGER,used INTEGER NOT NULL,bytes BLOB NOT NULL,PRIMARY KEY(room,message,placeholder)); PRAGMA user_version=2;").unwrap();
        db.execute(
            "INSERT INTO thumbnails VALUES(?1,7,'image',100,NULL,1,?2)",
            params![fixture.room.as_str(), png(2, 2).as_ref()],
        )
        .unwrap();
    }
    let loaded = store.load(100).unwrap();
    assert!(loaded.thumbnails.as_slice().is_empty());
    assert_eq!(loaded.histories.as_slice().len(), 1);
    assert_eq!(loaded.outbox.as_slice().len(), 1);
}
