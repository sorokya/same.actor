//! Stored discovery snapshots.
//!
//! A snapshot is written whole by the refresh job and read whole by the page. There is no partial
//! update: a list is either the previous refresh or the new one, never half of each.

use rusqlite::{params, Connection};

use crate::error::AppError;
use crate::store::Store;

#[derive(Debug, Clone)]
pub struct Stored {
    pub id: String,
    pub fetched_ts: i64,
    pub payload: String,
    pub synergy: String,
}

pub(super) fn put(
    conn: &Connection,
    id: &str,
    payload: &str,
    synergy: &str,
    now: i64,
) -> rusqlite::Result<()> {
    conn.execute(
        "INSERT INTO lists (id, fetched_ts, payload) VALUES (?1, ?2, ?3)
         ON CONFLICT(id) DO UPDATE SET fetched_ts = excluded.fetched_ts, payload = excluded.payload",
        params![id, now, payload],
    )?;

    conn.execute(
        "INSERT INTO list_synergy (list_id, computed_ts, payload) VALUES (?1, ?2, ?3)
         ON CONFLICT(list_id) DO UPDATE SET
             computed_ts = excluded.computed_ts,
             payload     = excluded.payload",
        params![id, now, synergy],
    )?;

    Ok(())
}

pub(super) fn get(conn: &Connection, id: &str) -> rusqlite::Result<Option<Stored>> {
    conn.query_row(
        "SELECT l.id, l.fetched_ts, l.payload, COALESCE(s.payload, '[]')
         FROM lists l LEFT JOIN list_synergy s ON s.list_id = l.id
         WHERE l.id = ?1",
        params![id],
        |row| {
            Ok(Stored {
                id: row.get(0)?,
                fetched_ts: row.get(1)?,
                payload: row.get(2)?,
                synergy: row.get(3)?,
            })
        },
    )
    .map(Some)
    .or_else(|error| match error {
        rusqlite::Error::QueryReturnedNoRows => Ok(None),
        other => Err(other),
    })
}

pub(super) fn oldest_age_secs(conn: &Connection, now: i64) -> rusqlite::Result<Option<i64>> {
    conn.query_row("SELECT MIN(fetched_ts) FROM lists", [], |row| {
        row.get::<_, Option<i64>>(0)
    })
    .map(|oldest| oldest.map(|ts| now - ts))
}

/// Drop snapshots whose id is no longer one of the lists the site serves.
///
/// Without this, a renamed or removed list leaves a row behind for ever: it is never read, never
/// refreshed, and never deleted, so the database grows a little every time the section model
/// changes.
pub(super) fn prune(conn: &Connection, keep: &[String]) -> rusqlite::Result<usize> {
    let mut removed = 0;

    let known: Vec<String> = conn
        .prepare("SELECT id FROM lists")?
        .query_map([], |row| row.get::<_, String>(0))?
        .collect::<rusqlite::Result<_>>()?;

    for id in known {
        if keep.iter().any(|keep| keep == &id) {
            continue;
        }

        conn.execute("DELETE FROM list_synergy WHERE list_id = ?1", params![id])?;
        removed += conn.execute("DELETE FROM lists WHERE id = ?1", params![id])?;
    }

    Ok(removed)
}

impl Store {
    /// Remove snapshots for lists that no longer exist. Returns how many rows went.
    pub async fn prune_lists(&self, keep: Vec<String>) -> Result<usize, AppError> {
        self.submit(move |ack| crate::store::WriteOp::PruneLists(keep, ack))
            .await
    }

    pub async fn put_list(
        &self,
        id: String,
        payload: String,
        synergy: String,
    ) -> Result<(), AppError> {
        let now = crate::clock::now_unix();
        self.submit(move |ack| crate::store::WriteOp::PutList(id, payload, synergy, now, ack))
            .await
    }

    pub async fn list_snapshot(&self, id: String) -> Result<Option<Stored>, AppError> {
        self.read(move |conn| get(conn, &id)).await
    }

    /// Age of the oldest stored list, for the health panel: a list that stopped refreshing is
    /// invisible until someone notices the pages are stale.
    pub async fn lists_age_secs(&self) -> Result<Option<i64>, AppError> {
        let now = crate::clock::now_unix();
        self.read(move |conn| oldest_age_secs(conn, now)).await
    }
}
