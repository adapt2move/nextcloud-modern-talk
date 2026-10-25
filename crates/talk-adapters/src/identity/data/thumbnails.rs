//! Disposable downscaled previews share the ordinary-room database. Foreign-key
//! deletion and the account ledger fence prevent orphaned or revoked previews.
use super::*;

const TOTAL_BYTES: usize = 4 * 1024 * 1024;

impl Store {
    pub(super) fn put_thumbnail(&self, thumbnail: &CacheThumbnail, now: u64) -> Result<()> {
        if thumbnail.message == 0
            || thumbnail.placeholder.as_str().is_empty()
            || !fresh(thumbnail.saved_at, now)
            || thumbnail.expires_at.is_some_and(|at| at > 0 && at <= now)
            || !valid_image(&thumbnail.bytes)
            || thumbnail.attachment.placeholder != thumbnail.placeholder
            || !preview_mime(thumbnail.attachment.mime_type.as_ref())
        {
            return Err(DataError::InvalidInput);
        }
        let message = integer(thumbnail.message)?;
        let attachment = codec::encode(&thumbnail.attachment, META_BYTES)?;
        let saved = integer(thumbnail.saved_at)?;
        let (ledger, mut db) = self.connection()?;
        self.check_room(&ledger, &thumbnail.room)?;
        super::prune(&mut db, now)?;
        let tx = sql(db.transaction())?;
        let known: bool = sql(tx.query_row(
            "SELECT EXISTS(SELECT 1 FROM rooms WHERE room=?1 AND saved<=?2 AND saved>?3)",
            params![
                thumbnail.room.as_str(),
                integer(now)?,
                integer(now)?.saturating_sub(CACHE_LIFETIME as i64)
            ],
            |r| r.get(0),
        ))?;
        if !known {
            return Err(DataError::InvalidInput);
        }
        let expires = message_expiry(&tx, thumbnail, now)?
            .map(integer)
            .transpose()?;
        // Replacing the exact target is an access. Use a monotonic local ordinal
        // for LRU so equal/adjusted wall-clock values cannot reorder admission.
        let used: i64 = sql(tx.query_row(
            "SELECT coalesce(max(used),0) FROM thumbnails",
            [],
            |r| r.get(0),
        ))?;
        let used = used.checked_add(1).ok_or(DataError::Corrupt)?;
        sql(tx.execute(
            "DELETE FROM thumbnails WHERE room=?1 AND message=?2 AND placeholder=?3",
            params![
                thumbnail.room.as_str(),
                message,
                thumbnail.placeholder.as_str()
            ],
        ))?;
        loop {
            let (count, size): (i64, i64) = sql(tx.query_row(
                "SELECT count(*),coalesce(sum(length(bytes)),0) FROM thumbnails",
                [],
                |r| Ok((r.get(0)?, r.get(1)?)),
            ))?;
            if count < CACHE_THUMBNAIL_LIMIT as i64
                && size + thumbnail.bytes.len() as i64 <= TOTAL_BYTES as i64
            {
                break;
            }
            if !evict(&tx)? {
                return Err(DataError::Capacity);
            }
        }
        reserve_for_write(&tx, thumbnail.bytes.len())?;
        sql(tx.execute("INSERT INTO thumbnails(room,message,placeholder,saved,expires,used,bytes,attachment) VALUES(?1,?2,?3,?4,?5,?6,?7,?8)", params![thumbnail.room.as_str(), message, thumbnail.placeholder.as_str(), saved, expires, used, thumbnail.bytes.as_ref(),attachment]))?;
        sql(tx.commit())
    }
    pub(super) fn remove_thumbnail(
        &self,
        room: &NavigationRoomKey,
        message: u64,
        placeholder: &Text<128>,
    ) -> Result<()> {
        let message = integer(message)?;
        if message == 0 || placeholder.as_str().is_empty() {
            return Err(DataError::InvalidInput);
        }
        let (ledger, db) = self.connection()?;
        self.check_room(&ledger, room)?;
        sql(db.execute(
            "DELETE FROM thumbnails WHERE room=?1 AND message=?2 AND placeholder=?3",
            params![room.as_str(), message, placeholder.as_str()],
        ))?;
        Ok(())
    }
    pub(super) fn clear_thumbnails(&self) -> Result<()> {
        let (_ledger, db) = self.connection()?;
        sql(db.execute("DELETE FROM thumbnails", []))?;
        Ok(())
    }
}
fn preview_mime(mime: Option<&Text<256>>) -> bool {
    mime.is_some_and(|mime| {
        mime.as_str().starts_with("image/")
            || mime.as_str().starts_with("video/")
            || mime.as_str() == "application/pdf"
    })
}

fn message_expiry(db: &Connection, thumbnail: &CacheThumbnail, now: u64) -> Result<Option<u64>> {
    let bytes: Option<Vec<u8>> = sql(db
        .query_row(
            "SELECT bytes FROM histories WHERE room=?1 AND length(bytes)<=?2",
            params![thumbnail.room.as_str(), HISTORY_BYTES as i64],
            |r| r.get(0),
        )
        .optional())?;
    let Some(bytes) = bytes else {
        return Ok(thumbnail.expires_at);
    };
    let history: CacheHistory = codec::decode(&bytes, HISTORY_BYTES)?;
    let Some(message) = history
        .messages
        .as_slice()
        .iter()
        .chain(history.pin_messages.as_slice())
        .find(|m| m.id == thumbnail.message)
    else {
        return Ok(thumbnail.expires_at);
    };
    if message.deleted
        || message.expires_at.is_some_and(|at| at > 0 && at <= now)
        || !message
            .attachments
            .as_slice()
            .iter()
            .any(|a| a == &thumbnail.attachment)
    {
        return Err(DataError::InvalidInput);
    }
    Ok([thumbnail.expires_at, message.expires_at]
        .into_iter()
        .flatten()
        .filter(|at| *at > 0)
        .min())
}

fn fresh(saved: u64, now: u64) -> bool {
    saved <= now && now - saved < CACHE_THUMBNAIL_LIFETIME
}
pub(super) fn prune(db: &Connection, now: u64) -> Result<()> {
    let now = integer(now)?;
    sql(db.execute("DELETE FROM thumbnails WHERE saved>?1 OR saved<=?2 OR (expires IS NOT NULL AND expires>0 AND expires<=?1) OR room NOT IN (SELECT room FROM rooms)", params![now, now.saturating_sub(CACHE_THUMBNAIL_LIFETIME as i64)]))?;
    sql(db.execute("DELETE FROM thumbnails WHERE typeof(placeholder)<>'text' OR length(CAST(placeholder AS BLOB)) NOT BETWEEN 1 AND 128 OR typeof(attachment)<>'blob' OR length(attachment)>8192 OR typeof(message)<>'integer' OR message<=0",[]))?;
    Ok(())
}

pub(super) fn remove_unavailable(db: &Connection, history: &CacheHistory, now: u64) -> Result<()> {
    for message in history
        .messages
        .as_slice()
        .iter()
        .chain(history.pin_messages.as_slice())
    {
        if message.deleted || message.expires_at.is_some_and(|at| at > 0 && at <= now) {
            sql(db.execute(
                "DELETE FROM thumbnails WHERE room=?1 AND message=?2",
                params![history.room.as_str(), integer(message.id)?],
            ))?;
        } else {
            let placeholders: Vec<(String, Option<Vec<u8>>)> = {
                let mut query=sql(db.prepare("SELECT CASE WHEN length(CAST(placeholder AS BLOB)) BETWEEN 1 AND 128 THEN placeholder END,CASE WHEN length(attachment)<=8192 THEN attachment END FROM thumbnails WHERE room=?1 AND message=?2 LIMIT 25"))?;
                sql(
                    query.query_map(params![history.room.as_str(), integer(message.id)?], |r| {
                        Ok((r.get(0)?, r.get(1)?))
                    }),
                )?
                .collect::<rusqlite::Result<_>>()
                .map_err(|_| DataError::Corrupt)?
            };
            for (placeholder, encoded) in placeholders {
                let prior = encoded
                    .as_ref()
                    .and_then(|bytes| codec::decode::<CachedAttachment>(bytes, META_BYTES).ok());
                if !message
                    .attachments
                    .as_slice()
                    .iter()
                    .any(|a| a.placeholder.as_str() == placeholder && Some(a) == prior.as_ref())
                {
                    sql(db.execute(
                        "DELETE FROM thumbnails WHERE room=?1 AND message=?2 AND placeholder=?3",
                        params![history.room.as_str(), integer(message.id)?, placeholder],
                    ))?;
                }
            }
            if let Some(expires) = message.expires_at.filter(|at| *at > 0) {
                // A newly observed earlier expiry also constrains previews already
                // persisted before that message-policy update.
                sql(db.execute("UPDATE thumbnails SET expires=?3 WHERE room=?1 AND message=?2 AND (expires IS NULL OR expires=0 OR expires>?3)", params![history.room.as_str(), integer(message.id)?, integer(expires)?]))?;
            }
        }
    }
    Ok(())
}

pub(super) fn read(
    db: &Connection,
    now: u64,
) -> Result<Page<CacheThumbnail, CACHE_THUMBNAIL_LIMIT>> {
    match read_checked(db, now) {
        Err(DataError::Corrupt) => {
            // Previews are disposable. A malformed blob must not hide retained
            // history or prevent loading/settling the durable outbox.
            sql(db.execute("DELETE FROM thumbnails", []))?;
            Ok(Page::empty())
        }
        result => result,
    }
}

fn read_checked(db: &Connection, now: u64) -> Result<Page<CacheThumbnail, CACHE_THUMBNAIL_LIMIT>> {
    let (count, size): (i64, i64) = sql(db.query_row(
        "SELECT count(*),coalesce(sum(length(bytes)),0) FROM thumbnails",
        [],
        |r| Ok((r.get(0)?, r.get(1)?)),
    ))?;
    if count > CACHE_THUMBNAIL_LIMIT as i64 || size > TOTAL_BYTES as i64 {
        return Err(DataError::Corrupt);
    }
    // CASE bounds each allocation before rusqlite materializes a corrupt blob
    // or text value. The account total and row count were checked above.
    let mut query = sql(db.prepare("SELECT CASE WHEN length(CAST(room AS BLOB))=35 THEN room END,message,CASE WHEN length(CAST(placeholder AS BLOB)) BETWEEN 1 AND 128 THEN placeholder END,saved,expires,CASE WHEN length(bytes) BETWEEN 1 AND ?1 THEN bytes END,CASE WHEN length(attachment)<=8192 THEN attachment END FROM thumbnails ORDER BY used DESC,rowid DESC LIMIT 25"))?;
    let mut rows = sql(query.query([CACHE_THUMBNAIL_BYTES as i64]))?;
    let mut result = Page::empty();
    while let Some(row) = sql(rows.next())? {
        let room: String = row.get(0).map_err(|_| DataError::Corrupt)?;
        let message: i64 = row.get(1).map_err(|_| DataError::Corrupt)?;
        let placeholder: String = row.get(2).map_err(|_| DataError::Corrupt)?;
        let saved: i64 = row.get(3).map_err(|_| DataError::Corrupt)?;
        let expires: Option<i64> = row.get(4).map_err(|_| DataError::Corrupt)?;
        let bytes: Vec<u8> = row.get(5).map_err(|_| DataError::Corrupt)?;
        let encoded: Vec<u8> = row.get(6).map_err(|_| DataError::Corrupt)?;
        let attachment: CachedAttachment =
            codec::decode(&encoded, META_BYTES).map_err(|_| DataError::Corrupt)?;
        if message <= 0
            || saved < 0
            || expires.is_some_and(|at| at < 0)
            || !fresh(saved as u64, now)
            || !valid_image(&bytes)
            || expires.is_some_and(|at| at > 0 && at as u64 <= now)
            || attachment.placeholder.as_str() != placeholder
            || !preview_mime(attachment.mime_type.as_ref())
        {
            return Err(DataError::Corrupt);
        }
        result
            .try_push(CacheThumbnail {
                room: NavigationRoomKey::new(&room).map_err(|_| DataError::Corrupt)?,
                message: message as u64,
                placeholder: Text::new(&placeholder).map_err(|_| DataError::Corrupt)?,
                bytes: bytes.into(),
                saved_at: saved as u64,
                expires_at: expires.map(|at| at as u64),
                attachment,
            })
            .map_err(|_| DataError::Corrupt)?;
    }
    Ok(result)
}

fn evict(db: &Connection) -> Result<bool> {
    sql(db.execute("DELETE FROM thumbnails WHERE rowid=(SELECT rowid FROM thumbnails ORDER BY used,rowid LIMIT 1)", [])).map(|n| n == 1)
}

/// Required room/history/outbox writes reclaim disposable previews first. Keep
/// the 16 MiB physical database cap; never evict durable intents to make space.
pub(super) fn reserve_for_write(db: &Connection, bytes: usize) -> Result<()> {
    loop {
        let pages: i64 = sql(db.query_row("PRAGMA page_count", [], |r| r.get(0)))?;
        let free: i64 = sql(db.query_row("PRAGMA freelist_count", [], |r| r.get(0)))?;
        if pages < free || pages > 4096 || free < 0 {
            return Err(DataError::Corrupt);
        }
        let available = (4096 - pages + free) as usize * 4096;
        // Reserve page/index overhead too. A required in-place update may need
        // less; if no disposable rows remain, SQLite decides atomically.
        if available >= bytes.saturating_add(64 * 1024) || !evict(db)? {
            return Ok(());
        }
    }
}

fn dimensions(width: u32, height: u32) -> bool {
    (1..=320).contains(&width) && (1..=240).contains(&height)
}
fn valid_image(bytes: &[u8]) -> bool {
    if bytes.is_empty() || bytes.len() > CACHE_THUMBNAIL_BYTES {
        return false;
    }
    if bytes.starts_with(b"\x89PNG\r\n\x1a\n") {
        return png(bytes);
    }
    jpeg(bytes)
}
fn png(bytes: &[u8]) -> bool {
    if bytes.len() < 45 || bytes.get(8..16) != Some(&b"\0\0\0\rIHDR"[..]) {
        return false;
    }
    let Some(header) = bytes.get(16..29) else {
        return false;
    };
    let width = u32::from_be_bytes([header[0], header[1], header[2], header[3]]);
    let height = u32::from_be_bytes([header[4], header[5], header[6], header[7]]);
    if !dimensions(width, height)
        || header[8] != 8
        || !matches!(header[9], 0 | 2 | 4 | 6)
        || header[10..13] != [0, 0, 0]
    {
        return false;
    }
    let mut offset = 33;
    let mut image = false;
    while let Some(chunk) = bytes.get(offset..offset + 8) {
        let size = u32::from_be_bytes([chunk[0], chunk[1], chunk[2], chunk[3]]) as usize;
        let Some(end) = offset.checked_add(12).and_then(|n| n.checked_add(size)) else {
            return false;
        };
        if end > bytes.len() {
            return false;
        }
        match &chunk[4..8] {
            b"IDAT" if size > 0 => image = true,
            b"IEND" => return image && size == 0 && end == bytes.len(),
            // The producer reencodes RGB(A) pixels. Text/EXIF/animation chunks
            // would be original metadata, outside this preview-only contract.
            _ => return false,
        }
        offset = end;
    }
    false
}
fn jpeg(bytes: &[u8]) -> bool {
    if !bytes.starts_with(&[0xff, 0xd8]) || !bytes.ends_with(&[0xff, 0xd9]) {
        return false;
    }
    let mut offset = 2;
    let mut sized = false;
    while let Some(&prefix) = bytes.get(offset) {
        if prefix != 0xff {
            return false;
        }
        offset += 1;
        while bytes.get(offset) == Some(&0xff) {
            offset += 1;
        }
        let Some(&marker) = bytes.get(offset) else {
            return false;
        };
        offset += 1;
        let Some(segment) = bytes.get(offset..offset + 2) else {
            return false;
        };
        let size = u16::from_be_bytes([segment[0], segment[1]]) as usize;
        if size < 2 {
            return false;
        }
        let Some(end) = offset.checked_add(size) else {
            return false;
        };
        let Some(segment) = bytes.get(offset + 2..end) else {
            return false;
        };
        match marker {
            0xc0 | 0xc2 => {
                if sized || segment.len() < 6 || segment[0] != 8 {
                    return false;
                }
                let height = u16::from_be_bytes([segment[1], segment[2]]) as u32;
                let width = u16::from_be_bytes([segment[3], segment[4]]) as u32;
                if !dimensions(width, height)
                    || !matches!(segment[5], 1 | 3)
                    || segment.len() != 6 + usize::from(segment[5]) * 3
                {
                    return false;
                }
                sized = true;
            }
            0xda => return sized && segment.len() >= 4 && end < bytes.len().saturating_sub(2),
            0xdb | 0xc4 | 0xdd => (),
            0xe0 if segment.len() == 14 && segment.starts_with(b"JFIF\0") => (),
            _ => return false,
        }
        offset = end;
    }
    false
}
