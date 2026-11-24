//! Responses as TMDB sent them, kept across restarts.
//!
//! The point is not to serve most requests from disk — the memory cache does that. The point is
//! what happens when the process dies: a deployment used to empty the cache, and the next refresh
//! rebuilt every list from a hundred upstream calls per list. With this table the rebuild reads
//! what it already had.
//!
//! The body is stored verbatim rather than parsed. A cached response is only ever handed back to
//! the same deserialiser that would have read it off the wire, so the table does not know, and
//! does not need to know, what a `TitleFacts` looks like. That is also what makes it safe to
//! change those shapes: a body this build can no longer read is dropped and refetched, which the
//! caller does by falling through to the network on a parse failure.
//!
//! Every row carries the instant it was fetched, because TMDB's terms forbid caching their content
//! for longer than six months — a rule that is only enforceable if the age is written down.

use rusqlite::{params, Connection};

/// Read a cached body, but only if it was fetched no earlier than `oldest`.
pub(super) fn get(conn: &Connection, key: &str, oldest: i64) -> rusqlite::Result<Option<String>> {
    conn.query_row(
        "SELECT body FROM tmdb_cache WHERE key = ?1 AND fetched_ts >= ?2",
        params![key, oldest],
        |row| row.get(0),
    )
    .or_else(|error| match error {
        rusqlite::Error::QueryReturnedNoRows => Ok(None),
        other => Err(other),
    })
}

pub(super) fn put(conn: &Connection, key: &str, body: &str, now: i64) -> rusqlite::Result<()> {
    conn.execute(
        "INSERT INTO tmdb_cache (key, body, fetched_ts) VALUES (?1, ?2, ?3)
         ON CONFLICT(key) DO UPDATE SET body = excluded.body, fetched_ts = excluded.fetched_ts",
        params![key, body, now],
    )?;

    Ok(())
}

/// Delete everything fetched before `before`. Returns how many rows went.
pub(super) fn purge(conn: &Connection, before: i64) -> rusqlite::Result<usize> {
    conn.execute(
        "DELETE FROM tmdb_cache WHERE fetched_ts < ?1",
        params![before],
    )
}

/// Drop one entry, for a body this build can no longer read.
pub(super) fn forget(conn: &Connection, key: &str) -> rusqlite::Result<()> {
    conn.execute("DELETE FROM tmdb_cache WHERE key = ?1", params![key])?;

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::store::migrations;

    fn db() -> Connection {
        let mut conn = Connection::open_in_memory().unwrap();
        migrations::migrate(&mut conn).unwrap();
        conn
    }

    #[test]
    fn a_body_comes_back_verbatim() {
        let conn = db();
        put(&conn, "k", r#"{"a":1}"#, 1_000).unwrap();

        assert_eq!(
            get(&conn, "k", 1_000).unwrap().as_deref(),
            Some(r#"{"a":1}"#)
        );
    }

    /// The age is the whole reason the timestamp is stored: a row older than the caller's floor is
    /// not a hit, and it is not an error either.
    #[test]
    fn a_body_older_than_the_floor_is_a_miss() {
        let conn = db();
        put(&conn, "k", "{}", 1_000).unwrap();

        assert_eq!(get(&conn, "k", 1_001).unwrap(), None);
    }

    #[test]
    fn putting_the_same_key_again_replaces_it() {
        let conn = db();
        put(&conn, "k", "old", 1_000).unwrap();
        put(&conn, "k", "new", 2_000).unwrap();

        assert_eq!(get(&conn, "k", 0).unwrap().as_deref(), Some("new"));
        assert_eq!(
            conn.query_row("SELECT count(*) FROM tmdb_cache", [], |row| row
                .get::<_, i64>(0))
                .unwrap(),
            1
        );
    }

    /// Six months is a ceiling on what may be kept, so the sweep has to be able to say "everything
    /// older than this is gone" — not "everything unused".
    #[test]
    fn the_purge_takes_only_what_is_older_than_the_cutoff() {
        let conn = db();
        put(&conn, "old", "{}", 1_000).unwrap();
        put(&conn, "new", "{}", 5_000).unwrap();

        assert_eq!(purge(&conn, 2_000).unwrap(), 1);
        assert_eq!(get(&conn, "old", 0).unwrap(), None);
        assert!(get(&conn, "new", 0).unwrap().is_some());
    }

    #[test]
    fn forgetting_a_key_removes_only_that_key() {
        let conn = db();
        put(&conn, "a", "{}", 1_000).unwrap();
        put(&conn, "b", "{}", 1_000).unwrap();

        forget(&conn, "a").unwrap();

        assert_eq!(get(&conn, "a", 0).unwrap(), None);
        assert!(get(&conn, "b", 0).unwrap().is_some());
    }
}
