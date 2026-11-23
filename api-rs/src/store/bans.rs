//! Bans: an address or a range, stored and enforced.
//!
//! There is no session ban. A cookie is client-controlled, so a ban a visitor escapes by
//! pressing ⌘⇧N is worse than no ban at all — it looks like enforcement while doing nothing.
//!
//! Enforcement reads from memory, not the database: the set is small, loads at startup,
//! refreshes every 30 seconds and immediately after any change, so a banned client never costs
//! a query per request.

use rusqlite::{params, Connection};
use serde::Serialize;

use crate::error::AppError;
use crate::store::{BanWrite, Store, WriteOp};

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Ban {
    pub id: i64,
    pub created_ts: i64,
    pub expires_ts: Option<i64>,
    /// An address, or a CIDR range.
    pub target: String,
    /// `ip` or `cidr`.
    pub kind: String,
    pub reason: Option<String>,
    /// `admin`, or `auto:<rule>` so a machine's decisions are distinguishable from a human's.
    pub created_by: String,
    pub hits: i64,
    pub last_hit: Option<i64>,
}

/// What a ban is created from.
#[derive(Debug, Clone)]
pub struct NewBan {
    pub target: String,
    pub kind: String,
    pub reason: Option<String>,
    pub created_by: String,
    /// `None` is permanent.
    pub expires_ts: Option<i64>,
}

const COLUMNS: &str =
    "id, created_ts, expires_ts, target, kind, reason, created_by, hits, last_hit";

fn row(row: &rusqlite::Row<'_>) -> rusqlite::Result<Ban> {
    Ok(Ban {
        id: row.get(0)?,
        created_ts: row.get(1)?,
        expires_ts: row.get(2)?,
        target: row.get(3)?,
        kind: row.get(4)?,
        reason: row.get(5)?,
        created_by: row.get(6)?,
        hits: row.get(7)?,
        last_hit: row.get(8)?,
    })
}

/// Insert, or update the expiry and reason of an existing ban for the same target.
///
/// Re-banning has to be idempotent: clicking "ban" twice, or an auto-ban rule firing again for
/// an address already banned, must not fail on the unique index.
pub(super) fn insert(conn: &Connection, ban: &NewBan) -> rusqlite::Result<i64> {
    conn.query_row(
        "INSERT INTO bans (created_ts, expires_ts, target, kind, reason, created_by)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6)
         ON CONFLICT(target) DO UPDATE SET
             expires_ts = excluded.expires_ts,
             reason     = excluded.reason,
             created_by = excluded.created_by
         RETURNING id",
        params![
            crate::clock::now_unix(),
            ban.expires_ts,
            ban.target,
            ban.kind,
            ban.reason,
            ban.created_by,
        ],
        |row| row.get(0),
    )
}

pub(super) fn delete(conn: &Connection, id: i64) -> rusqlite::Result<bool> {
    let affected = conn.execute("DELETE FROM bans WHERE id = ?1", params![id])?;
    Ok(affected > 0)
}

pub(super) fn load(conn: &Connection) -> rusqlite::Result<Vec<Ban>> {
    let mut stmt = conn.prepare(&format!("SELECT {COLUMNS} FROM bans ORDER BY id"))?;
    let rows = stmt.query_map([], row)?;
    rows.collect()
}

/// Count a hit against a ban. Batched by the writer, because a client being refused 400 times
/// is one event worth knowing about and 400 writes worth avoiding.
pub(super) fn record_hit(conn: &Connection, id: i64) -> rusqlite::Result<()> {
    conn.execute(
        "UPDATE bans SET hits = hits + 1, last_hit = ?2 WHERE id = ?1",
        params![id, crate::clock::now_unix()],
    )?;
    Ok(())
}

/// Remove bans whose expiry has passed. Run by the nightly job.
pub(super) fn purge_expired(conn: &Connection, now: i64) -> rusqlite::Result<usize> {
    conn.execute(
        "DELETE FROM bans WHERE expires_ts IS NOT NULL AND expires_ts <= ?1",
        params![now],
    )
}

impl Store {
    pub async fn insert_ban(&self, ban: NewBan) -> Result<i64, AppError> {
        self.submit(move |ack| WriteOp::Ban(BanWrite::Insert(ban, ack)))
            .await
    }

    pub async fn delete_ban(&self, id: i64) -> Result<bool, AppError> {
        self.submit(move |ack| WriteOp::Ban(BanWrite::Delete(id, ack)))
            .await
    }

    /// Every ban, including expired ones — the admin list shows them so an operator can see
    /// what expired and when. [`crate::bans::BanSet`] is what filters by expiry.
    pub async fn bans(&self) -> Result<Vec<Ban>, AppError> {
        self.read(load).await
    }

    /// Fire and forget: a refused request must not wait on a counter update.
    pub fn ban_hit(&self, id: i64) {
        self.offer_ban_hit(id);
    }

    pub async fn purge_expired_bans(&self) -> Result<usize, AppError> {
        self.submit(move |ack| WriteOp::Ban(BanWrite::Purge(ack)))
            .await
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::store::{Store, StoreConfig};

    struct TempDb {
        dir: std::path::PathBuf,
    }

    impl TempDb {
        fn new(name: &str) -> Self {
            let dir =
                std::env::temp_dir().join(format!("same-actor-bans-{}-{name}", std::process::id()));
            let _ = std::fs::remove_dir_all(&dir);
            std::fs::create_dir_all(&dir).expect("temp dir");
            Self { dir }
        }

        fn open(&self) -> Store {
            Store::open(StoreConfig {
                path: self.dir.join("test.db"),
                ..StoreConfig::default()
            })
            .expect("open")
        }
    }

    impl Drop for TempDb {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.dir);
        }
    }

    fn ban(target: &str, kind: &str, expires_ts: Option<i64>) -> NewBan {
        NewBan {
            target: target.into(),
            kind: kind.into(),
            reason: Some("testing".into()),
            created_by: "admin".into(),
            expires_ts,
        }
    }

    #[tokio::test]
    async fn a_ban_round_trips() {
        let db = TempDb::new("roundtrip");
        let store = db.open();

        let id = store
            .insert_ban(ban("203.0.113.7", "ip", None))
            .await
            .expect("insert");
        let bans = store.bans().await.expect("read");

        assert_eq!(bans.len(), 1);
        assert_eq!(bans[0].id, id);
        assert_eq!(bans[0].target, "203.0.113.7");
        assert_eq!(bans[0].expires_ts, None, "no expiry means permanent");
        assert_eq!(bans[0].hits, 0);

        store.close().await.expect("close");
    }

    /// Clicking "ban" twice must update, not fail on the unique index.
    #[tokio::test]
    async fn banning_the_same_target_twice_updates_the_existing_ban() {
        let db = TempDb::new("duplicate");
        let store = db.open();

        let first = store
            .insert_ban(ban("203.0.113.7", "ip", None))
            .await
            .expect("first");
        let second = store
            .insert_ban(NewBan {
                reason: Some("second reason".into()),
                expires_ts: Some(4_000_000_000),
                ..ban("203.0.113.7", "ip", None)
            })
            .await
            .expect("second");

        assert_eq!(first, second, "the ban kept its identity");
        let bans = store.bans().await.expect("read");
        assert_eq!(bans.len(), 1);
        assert_eq!(bans[0].reason.as_deref(), Some("second reason"));
        assert_eq!(bans[0].expires_ts, Some(4_000_000_000));

        store.close().await.expect("close");
    }

    #[tokio::test]
    async fn a_ban_can_be_deleted_and_deleting_an_unknown_id_says_so() {
        let db = TempDb::new("delete");
        let store = db.open();

        let id = store
            .insert_ban(ban("203.0.113.7", "ip", None))
            .await
            .expect("insert");
        assert!(store.delete_ban(id).await.expect("delete"));
        assert!(!store.delete_ban(id).await.expect("delete again"));
        assert!(store.bans().await.expect("read").is_empty());

        store.close().await.expect("close");
    }

    #[tokio::test]
    async fn hits_are_counted() {
        let db = TempDb::new("hits");
        let store = db.open();

        let id = store
            .insert_ban(ban("203.0.113.7", "ip", None))
            .await
            .expect("insert");
        store.ban_hit(id);
        store.ban_hit(id);
        // Hits are offered, not awaited, so the barrier is what makes them visible.
        store.flush().await.expect("flush");

        let bans = store.bans().await.expect("read");
        assert_eq!(bans[0].hits, 2);
        assert!(bans[0].last_hit.is_some());

        store.close().await.expect("close");
    }

    #[tokio::test]
    async fn expired_bans_are_purged_and_permanent_ones_are_not() {
        let db = TempDb::new("purge");
        let store = db.open();

        let now = crate::clock::now_unix();
        store
            .insert_ban(ban("203.0.113.7", "ip", Some(now - 10)))
            .await
            .expect("expired");
        store
            .insert_ban(ban("203.0.113.8", "ip", Some(now + 10_000)))
            .await
            .expect("live");
        store
            .insert_ban(ban("203.0.113.9", "ip", None))
            .await
            .expect("permanent");

        assert_eq!(store.purge_expired_bans().await.expect("purge"), 1);

        let remaining: Vec<String> = store
            .bans()
            .await
            .expect("read")
            .into_iter()
            .map(|ban| ban.target)
            .collect();
        assert_eq!(remaining, ["203.0.113.8", "203.0.113.9"]);

        store.close().await.expect("close");
    }
}
