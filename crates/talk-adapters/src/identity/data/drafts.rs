//! Unsubmitted text is independent of disposable history and accepted outbox work.
use super::*;

pub const CACHE_DRAFT_LIMIT: usize = 32;
const DRAFT_BYTES: usize = 8192;
const TOTAL_BYTES: usize = 128 * 1024;

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CacheDraft {
    #[serde(with = "codec::room_key")]
    pub room: NavigationRoomKey,
    pub thread: Option<u64>,
    pub reply: Option<u64>,
    #[serde(with = "codec::text")]
    pub text: Text<4096>,
    pub saved_at: u64,
}
fn thread(draft: &CacheDraft) -> Result<i64> {
    if draft.thread == Some(0) || draft.reply == Some(0) {
        return Err(DataError::InvalidInput);
    }
    integer(draft.thread.unwrap_or(0))
}
impl Store {
    pub(super) fn put_draft(&self, draft: &CacheDraft, now: u64) -> Result<()> {
        let key = thread(draft)?;
        if !fresh(draft.saved_at, now) {
            return Err(DataError::InvalidInput);
        }
        let (ledger, mut db) = self.connection()?;
        self.check_room(&ledger, &draft.room)?;
        super::prune(&mut db, now)?;
        let tx = sql(db.transaction())?;
        // A replacement is atomic. Full capacity never evicts another unsent draft.
        sql(tx.execute(
            "DELETE FROM drafts WHERE room=?1 AND thread=?2",
            params![draft.room.as_str(), key],
        ))?;
        if !draft.text.as_str().is_empty() {
            let known: bool = sql(tx.query_row(
                "SELECT EXISTS(SELECT 1 FROM rooms WHERE room=?1 AND saved<=?2 AND saved>?3)",
                params![
                    draft.room.as_str(),
                    integer(now)?,
                    integer(now.saturating_sub(CACHE_LIFETIME))?
                ],
                |r| r.get(0),
            ))?;
            if !known {
                return Err(DataError::Ineligible);
            }
            let bytes = codec::encode(draft, DRAFT_BYTES)?;
            let (count, size): (i64, i64) = sql(tx.query_row(
                "SELECT count(*),coalesce(sum(length(bytes)),0) FROM drafts",
                [],
                |r| Ok((r.get(0)?, r.get(1)?)),
            ))?;
            if count >= CACHE_DRAFT_LIMIT as i64 || size + bytes.len() as i64 > TOTAL_BYTES as i64 {
                return Err(DataError::Capacity);
            }
            thumbnails::reserve_for_write(&tx, bytes.len())?;
            sql(tx.execute(
                "INSERT INTO drafts(room,thread,saved,bytes) VALUES(?1,?2,?3,?4)",
                params![draft.room.as_str(), key, integer(draft.saved_at)?, bytes],
            ))?;
        }
        sql(tx.commit())
    }
}
pub(super) fn prune(db: &Connection, now: u64) -> Result<()> {
    sql(db.execute(
        "DELETE FROM drafts WHERE saved>?1 OR saved<=?2",
        params![integer(now)?, integer(now.saturating_sub(CACHE_LIFETIME))?],
    ))?;
    Ok(())
}
pub(super) fn read(db: &Connection, now: u64) -> Result<Page<CacheDraft, CACHE_DRAFT_LIMIT>> {
    let mut query = sql(db.prepare("SELECT room,thread,saved,CASE WHEN length(bytes)<=8192 THEN bytes ELSE NULL END FROM drafts ORDER BY saved,room,thread LIMIT 33"))?;
    let mut rows = sql(query.query([]))?;
    let mut result = Page::empty();
    let mut size = 0;
    while let Some(row) = sql(rows.next())? {
        let key: String = row.get(0).map_err(|_| DataError::Corrupt)?;
        let root: i64 = row.get(1).map_err(|_| DataError::Corrupt)?;
        let saved: i64 = row.get(2).map_err(|_| DataError::Corrupt)?;
        let bytes: Vec<u8> = row.get(3).map_err(|_| DataError::Corrupt)?;
        size += bytes.len();
        let draft: CacheDraft = codec::decode(&bytes, DRAFT_BYTES)?;
        if size > TOTAL_BYTES
            || draft.room.as_str() != key
            || thread(&draft)? != root
            || integer(draft.saved_at)? != saved
            || !fresh(draft.saved_at, now)
        {
            return Err(DataError::Corrupt);
        }
        result.try_push(draft).map_err(|_| DataError::Corrupt)?;
    }
    Ok(result)
}
