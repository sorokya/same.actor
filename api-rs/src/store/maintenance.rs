//! Aggregation and retention: turn old raw rows into counts, then delete them.
//!
//! Two things happen at the retention boundary and they must happen together. The day's
//! aggregates are computed **from the raw rows that are about to be deleted**, and both steps
//! run in one transaction, so a crash cannot leave a day rolled up and its raw rows still there
//! (which would double count on the next run) or the reverse (which would lose the day).
//!
//! The aggregates carry no IP and no session id. That is the privacy cutover as much as the size
//! one: after it, the day exists only as counts.

use std::collections::{HashMap, HashSet};

use rusqlite::{params, Connection};

use crate::error::AppError;
use crate::store::Store;

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Report {
    /// Days rolled up, with how many raw rows each contributed.
    pub days: Vec<(String, u64)>,
    pub rows_deleted: u64,
    pub bans_purged: usize,
}

impl Report {
    pub fn is_empty(&self) -> bool {
        self.days.is_empty() && self.bans_purged == 0
    }
}

#[derive(Default)]
struct Group {
    requests: u64,
    errors: u64,
    sessions: HashSet<String>,
    durations: Vec<u32>,
}

impl Group {
    fn p95(&mut self) -> u32 {
        if self.durations.is_empty() {
            return 0;
        }

        self.durations.sort_unstable();
        let index = (self.durations.len() * 95 / 100).min(self.durations.len() - 1);
        self.durations[index]
    }
}

/// Roll up every complete day that still has raw rows older than the retention window, then
/// delete those rows. Returns what it did, so the caller can log it.
pub(super) fn run(
    conn: &mut Connection,
    retention_days: u32,
    now: i64,
) -> rusqlite::Result<Report> {
    let mut report = Report::default();

    let cutoff = crate::clock::day_start_days_ago(now, retention_days as i64);

    // Only days that actually have rows: an 8-year-old empty database should not cost 3000
    // round trips to prove it.
    //
    // Both tables, not just `requests`. Discovering days from one table meant a day that only
    // had searches was never rolled up and never pruned — it would have grown forever, and the
    // gap would only show up in trending.
    let mut stmt = conn.prepare(
        "SELECT day FROM (
             SELECT DISTINCT ts / 86400 AS day FROM requests WHERE ts < ?1
             UNION
             SELECT DISTINCT ts / 86400 AS day FROM searches WHERE ts < ?1
         ) ORDER BY day",
    )?;
    let day_indexes: Vec<i64> = stmt
        .query_map(params![cutoff], |row| row.get(0))?
        .collect::<rusqlite::Result<_>>()?;
    drop(stmt);

    for day_index in day_indexes {
        let start = day_index * 86_400;
        let end = start + 86_400;
        let day = crate::clock::day_string(start);

        let tx = conn.transaction()?;

        let rows = roll_requests(&tx, &day, start, end)?;
        roll_searches(&tx, &day, start, end)?;
        roll_totals(&tx, &day, start, end)?;
        let deleted = tx.execute(
            "DELETE FROM requests WHERE ts >= ?1 AND ts < ?2",
            params![start, end],
        )?;
        tx.execute(
            "DELETE FROM searches WHERE ts >= ?1 AND ts < ?2",
            params![start, end],
        )?;

        tx.commit()?;

        report.days.push((day, rows));
        report.rows_deleted += deleted as u64;
    }

    report.bans_purged = crate::store::bans::purge_expired(conn, now)?;

    Ok(report)
}

fn roll_requests(conn: &Connection, day: &str, start: i64, end: i64) -> rusqlite::Result<u64> {
    let mut stmt = conn.prepare(
        "SELECT kind, country, status, duration_ms, session FROM requests
         WHERE ts >= ?1 AND ts < ?2",
    )?;

    let mut groups: HashMap<(String, String, i64), Group> = HashMap::new();
    let mut total = 0u64;

    let rows = stmt.query_map(params![start, end], |row| {
        Ok((
            row.get::<_, String>(0)?,
            row.get::<_, Option<String>>(1)?,
            row.get::<_, i64>(2)?,
            row.get::<_, i64>(3)?,
            row.get::<_, Option<String>>(4)?,
        ))
    })?;

    for row in rows {
        let (kind, country, status, duration, session) = row?;
        total += 1;

        let key = (kind, country.unwrap_or_default(), status / 100);
        let group = groups.entry(key).or_default();
        group.requests += 1;
        if status >= 400 {
            group.errors += 1;
        }
        if let Some(session) = session {
            group.sessions.insert(session);
        }
        group
            .durations
            .push(duration.clamp(0, i64::from(u32::MAX)) as u32);
    }
    drop(stmt);

    let mut upsert = conn.prepare(
        "INSERT INTO daily_requests
             (day, kind, country, status_cls, requests, errors, sessions, p95_ms)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)
         ON CONFLICT(day, kind, country, status_cls) DO UPDATE SET
             requests = excluded.requests,
             errors   = excluded.errors,
             sessions = excluded.sessions,
             p95_ms   = excluded.p95_ms",
    )?;

    for ((kind, country, status_cls), mut group) in groups {
        let p95 = group.p95();
        upsert.execute(params![
            day,
            kind,
            country,
            status_cls,
            group.requests as i64,
            group.errors as i64,
            group.sessions.len() as i64,
            p95,
        ])?;
    }

    Ok(total)
}

/// The day's totals, with visitors counted once over the whole day rather than summed from
/// per-group distinct counts.
fn roll_totals(conn: &Connection, day: &str, start: i64, end: i64) -> rusqlite::Result<()> {
    let (requests, errors, visitors): (i64, i64, i64) = conn.query_row(
        "SELECT COUNT(*), SUM(status >= 400), COUNT(DISTINCT session)
         FROM requests WHERE ts >= ?1 AND ts < ?2",
        params![start, end],
        |row| Ok((row.get(0)?, row.get(1).unwrap_or(0), row.get(2)?)),
    )?;

    let searches: i64 = conn.query_row(
        "SELECT COUNT(*) FROM searches WHERE ts >= ?1 AND ts < ?2",
        params![start, end],
        |row| row.get(0),
    )?;

    conn.execute(
        "INSERT INTO daily_totals (day, requests, errors, visitors, searches)
         VALUES (?1, ?2, ?3, ?4, ?5)
         ON CONFLICT(day) DO UPDATE SET
             requests = excluded.requests,
             errors   = excluded.errors,
             visitors = excluded.visitors,
             searches = excluded.searches",
        params![day, requests, errors, visitors, searches],
    )?;

    Ok(())
}

fn roll_searches(conn: &Connection, day: &str, start: i64, end: i64) -> rusqlite::Result<()> {
    // Grouped in SQL: there are few distinct terms per day and no per-row work worth doing in
    // Rust, unlike the percentile above.
    conn.execute(
        "INSERT INTO daily_searches (day, mode, term_norm, searches)
         SELECT ?1, mode, term_norm, COUNT(*)
         FROM searches
         WHERE ts >= ?2 AND ts < ?3
         GROUP BY mode, term_norm
         ON CONFLICT(day, mode, term_norm) DO UPDATE SET searches = excluded.searches",
        params![day, start, end],
    )?;

    Ok(())
}

/// Keep the file from growing forever after the big deletes.
fn compact(conn: &Connection) {
    if let Err(error) = conn.execute_batch("PRAGMA incremental_vacuum; PRAGMA optimize;") {
        tracing::warn!(%error, "could not compact the database after retention");
    }
}

impl Store {
    /// Run retention now: roll up eligible days, delete their raw rows, purge expired bans.
    pub async fn maintain(&self, retention_days: u32) -> Result<Report, AppError> {
        let now = crate::clock::now_unix();

        let report = self
            .submit(move |ack| crate::store::WriteOp::Maintenance(retention_days, now, ack))
            .await?;

        Ok(report)
    }
}

/// Wraps [`run`] with the compaction step, which must happen outside the per-day transactions.
pub(super) fn maintain(conn: &mut Connection, retention_days: u32, now: i64) -> Report {
    let report = run(conn, retention_days, now).unwrap_or_else(|error| {
        tracing::error!(%error, "could not roll up and prune old rows");
        Report::default()
    });

    compact(conn);
    report
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::store::request::{Kind, RequestLog};

    struct TempDb {
        dir: std::path::PathBuf,
    }

    impl TempDb {
        fn new(name: &str) -> Self {
            let dir = std::env::temp_dir().join(format!(
                "same-actor-maintenance-{}-{name}",
                std::process::id()
            ));
            let _ = std::fs::remove_dir_all(&dir);
            std::fs::create_dir_all(&dir).expect("temp dir");
            Self { dir }
        }

        fn open(&self) -> Store {
            Store::open(crate::store::StoreConfig {
                path: self.dir.join("test.db"),
                ..crate::store::StoreConfig::default()
            })
            .expect("open")
        }
    }

    impl Drop for TempDb {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.dir);
        }
    }

    fn log(ts: i64, status: u16, session: &str, duration_ms: u32) -> RequestLog {
        RequestLog {
            ts,
            duration_ms,
            ip: Some("203.0.113.7".into()),
            session: Some(session.into()),
            ua: Some("test".into()),
            method: "GET".into(),
            path: "/api/search".into(),
            status,
            code: None,
            message: None,
            kind: Kind::Api,
            country: Some("GB".into()),
        }
    }

    #[tokio::test]
    async fn old_days_are_rolled_up_and_their_raw_rows_removed() {
        let db = TempDb::new("rollup");
        let store = db.open();

        let now = crate::clock::now_unix();
        let old = crate::clock::day_start_days_ago(now, 10) + 3_600;

        // Three requests on the old day: two visits, one of them failing.
        store.record(log(old, 200, "a", 10));
        store.record(log(old + 1, 200, "a", 20));
        store.record(log(old + 2, 502, "b", 30));
        store.flush().await.expect("flush");

        let report = store.maintain(1).await.expect("maintain");

        assert_eq!(report.days.len(), 1);
        assert_eq!(report.days[0].1, 3, "three rows rolled up");
        assert_eq!(report.rows_deleted, 3);
        assert_eq!(
            store.request_count().await.expect("count"),
            0,
            "raw rows are gone"
        );

        let day = crate::clock::day_string(old);
        // Successes group by status class 2; the 502 is class 5 and its own row.
        let (requests, errors, sessions) = daily(&store, &day, 2).await;
        assert_eq!(requests, 2, "the two 200s are one group");
        assert_eq!(errors, 0, "a success is not an error");
        assert_eq!(sessions, 1, "distinct sessions counted, not rows");

        let (requests, errors, _) = daily(&store, &day, 5).await;
        assert_eq!(requests, 1);
        assert_eq!(errors, 1, "the 502 counts as an error in its own class");

        store.close().await.expect("close");
    }

    async fn daily(store: &Store, day: &str, status_cls: i64) -> (i64, i64, i64) {
        let day = day.to_owned();
        store
            .read(move |conn| {
                conn.query_row(
                    "SELECT requests, errors, sessions FROM daily_requests
                     WHERE day = ?1 AND status_cls = ?2",
                    params![day, status_cls],
                    |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
                )
            })
            .await
            .expect("daily row")
    }

    /// Retention must not touch a day that is still inside its window.
    #[tokio::test]
    async fn recent_days_are_left_alone() {
        let db = TempDb::new("recent");
        let store = db.open();

        let now = crate::clock::now_unix();
        store.record(log(now - 60, 200, "a", 5));
        store.flush().await.expect("flush");

        let report = store.maintain(90).await.expect("maintain");
        assert!(report.days.is_empty(), "nothing should be rolled up yet");
        assert_eq!(store.request_count().await.expect("count"), 1);

        store.close().await.expect("close");
    }

    /// Running twice must not double count: the second run finds no raw rows for the day and so
    /// does nothing at all.
    #[tokio::test]
    async fn a_second_run_changes_nothing() {
        let db = TempDb::new("idempotent");
        let store = db.open();

        let now = crate::clock::now_unix();
        let old = crate::clock::day_start_days_ago(now, 10) + 3_600;
        for i in 0..10 {
            store.record(log(old + i, 200, "a", 10));
        }
        store.flush().await.expect("flush");

        let first = store.maintain(1).await.expect("first");
        assert_eq!(first.rows_deleted, 10);

        let second = store.maintain(1).await.expect("second");
        assert!(second.days.is_empty(), "the day was already rolled up");
        assert_eq!(second.rows_deleted, 0);

        let (requests, _, _) = daily(&store, &crate::clock::day_string(old), 2).await;
        assert_eq!(
            requests, 10,
            "the aggregate must not have been recomputed from nothing"
        );

        store.close().await.expect("close");
    }

    /// The privacy promise: after rollup, nothing in the aggregate identifies a visitor.
    #[tokio::test]
    async fn aggregates_keep_no_address_or_session() {
        let db = TempDb::new("privacy");
        let store = db.open();

        let now = crate::clock::now_unix();
        let old = crate::clock::day_start_days_ago(now, 10) + 3_600;
        store.record(log(old, 200, "secret-session", 10));
        store.flush().await.expect("flush");
        store.maintain(1).await.expect("maintain");

        let leaks: i64 = store
            .read(|conn| {
                conn.query_row(
                    "SELECT (SELECT COUNT(*) FROM daily_requests) + (SELECT COUNT(*) FROM requests)",
                    [],
                    |row| row.get(0),
                )
            })
            .await
            .expect("count");

        assert_eq!(leaks, 1, "one aggregate row, and no raw rows");
        let columns: Vec<String> = store
            .read(|conn| {
                let mut stmt =
                    conn.prepare("SELECT name FROM pragma_table_info('daily_requests')")?;
                let rows = stmt.query_map([], |row| row.get::<_, String>(0))?;
                rows.collect()
            })
            .await
            .expect("columns");

        assert!(!columns
            .iter()
            .any(|c| c == "ip" || c == "session" || c == "ua"));

        store.close().await.expect("close");
    }

    #[tokio::test]
    async fn searches_are_rolled_up_by_normalised_term() {
        let db = TempDb::new("searches");
        let store = db.open();

        let now = crate::clock::now_unix();
        let old = crate::clock::day_start_days_ago(now, 10) + 3_600;

        for term in ["the matrix", "The Matrix", "the matrix"] {
            assert!(
                store.record_search(crate::store::searches::SearchEvent {
                    ts: old,
                    mode: "title".into(),
                    term: term.into(),
                    term_norm: term.to_lowercase(),
                    results: 3,
                    session: Some("a".into()),
                    ip: Some("203.0.113.7".into()),
                }),
                "the queue refused a search event"
            );
        }
        store.flush().await.expect("flush");
        store.maintain(1).await.expect("maintain");

        let searches: i64 = store
            .read(|conn| {
                conn.query_row(
                    "SELECT searches FROM daily_searches WHERE term_norm = 'the matrix'",
                    [],
                    |row| row.get(0),
                )
            })
            .await
            .expect("daily search");
        assert_eq!(searches, 3, "case variants collapse into one term");

        store.close().await.expect("close");
    }
}
