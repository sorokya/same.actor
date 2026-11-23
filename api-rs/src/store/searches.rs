//! Search events: what was looked for, and whether it worked.
//!
//! Recorded separately from the request log because the interesting question is "what did people
//! search for and fail to find", which is a product question, not a traffic one. Requests to
//! `/api/search` also appear in the log; this table is what trending and the zero-result report
//! read.

use rusqlite::{params, Connection};
use serde::Serialize;

use crate::error::AppError;
use crate::store::Store;

#[derive(Debug, Clone)]
pub struct SearchEvent {
    pub ts: i64,
    pub mode: String,
    /// As typed, truncated before it gets here.
    pub term: String,
    /// Lowercased, trimmed, whitespace-collapsed. Trending groups on this.
    pub term_norm: String,
    pub results: u32,
    pub session: Option<String>,
    pub ip: Option<String>,
}

/// Longest term stored. Nobody searches for 200 characters, and the column is not a text dump.
const MAX_TERM: usize = 120;

/// Normalise a term for grouping: lowercase, trim, collapse internal whitespace.
pub fn normalise(term: &str) -> String {
    term.split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
        .to_lowercase()
}

pub(super) fn insert(conn: &Connection, batch: &[SearchEvent]) -> rusqlite::Result<()> {
    if batch.is_empty() {
        return Ok(());
    }

    let mut stmt = conn.prepare_cached(
        "INSERT INTO searches (ts, mode, term, term_norm, results, session, ip)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
    )?;

    for event in batch {
        stmt.execute(params![
            event.ts,
            event.mode,
            truncate(&event.term, MAX_TERM),
            truncate(&event.term_norm, MAX_TERM),
            event.results,
            event.session,
            event.ip,
        ])?;
    }

    Ok(())
}

fn truncate(value: &str, limit: usize) -> String {
    if value.len() <= limit {
        return value.to_owned();
    }

    let mut end = limit;
    while end > 0 && !value.is_char_boundary(end) {
        end -= 1;
    }

    value[..end].to_owned()
}

#[derive(Debug, Clone, Serialize)]
pub struct SearchRow {
    pub id: i64,
    pub ts: i64,
    pub mode: String,
    pub term: String,
    pub term_norm: String,
    pub results: i64,
}

#[derive(Debug, Clone, Serialize)]
pub struct TrendingTerm {
    pub term: String,
    pub searches: i64,
}

/// Trending reads both sources. Rolled-up days live in `daily_searches`; days still inside the
/// retention window live in `searches`. Reading only the aggregates looked correct and showed an
/// empty page for the last 24 hours — the part anyone actually looks at.
const TRENDING: &str = "SELECT term, SUM(total) AS total FROM (
         SELECT term_norm AS term, SUM(searches) AS total
         FROM daily_searches WHERE day >= ?1 GROUP BY term_norm
         UNION ALL
         SELECT term_norm AS term, COUNT(*) AS total
         FROM searches WHERE ts >= ?2 GROUP BY term_norm
     )
     GROUP BY term
     HAVING total >= ?3
     ORDER BY total DESC, term ASC
     LIMIT ?4";

impl Store {
    /// The most recent searches, newest first.
    pub async fn recent_searches(&self, limit: u32) -> Result<Vec<SearchRow>, AppError> {
        let limit = limit.clamp(1, 1_000) as i64;

        self.read(move |conn| {
            let mut stmt = conn.prepare(
                "SELECT id, ts, mode, term, term_norm, results FROM searches
                 ORDER BY id DESC LIMIT ?1",
            )?;
            let rows = stmt.query_map([limit], |row| {
                Ok(SearchRow {
                    id: row.get(0)?,
                    ts: row.get(1)?,
                    mode: row.get(2)?,
                    term: row.get(3)?,
                    term_norm: row.get(4)?,
                    results: row.get(5)?,
                })
            })?;
            rows.collect()
        })
        .await
    }

    /// Offer a search event. Fire and forget, like request records: a search must not wait on a
    /// database write to answer.
    pub fn record_search(&self, event: SearchEvent) -> bool {
        self.offer_search(event)
    }

    /// Trending terms over the last `days` days of rolled-up data.
    ///
    /// Read from the aggregates rather than the raw table so it keeps working after retention
    /// has removed the detail. `min_searches` keeps the page from publishing a single visitor's
    /// one-off query.
    pub async fn trending(
        &self,
        days: i64,
        min_searches: i64,
        limit: u32,
    ) -> Result<Vec<TrendingTerm>, AppError> {
        let now = crate::clock::now_unix();
        let window = days.max(1);
        let from_day = crate::clock::day_string(crate::clock::day_start_days_ago(now, window));
        let from_ts = crate::clock::day_start_days_ago(now, window);
        let limit = limit.clamp(1, 100) as i64;

        self.read(move |conn| {
            let mut stmt = conn.prepare(TRENDING)?;
            let rows = stmt.query_map(
                params![from_day, from_ts, min_searches.max(1), limit],
                |row| {
                    Ok(TrendingTerm {
                        term: row.get(0)?,
                        searches: row.get(1)?,
                    })
                },
            )?;
            rows.collect()
        })
        .await
    }

    /// Terms that returned nothing. Free product insight: what people wanted and the catalogue
    /// could not give them.
    ///
    /// Raw rows only, so this covers the retention window rather than all time — the aggregates
    /// keep counts by term, not the result count that makes a search "zero result".
    pub async fn zero_result_searches(
        &self,
        days: i64,
        limit: u32,
    ) -> Result<Vec<TrendingTerm>, AppError> {
        let from = crate::clock::day_start_days_ago(crate::clock::now_unix(), days.max(1));

        self.read(move |conn| {
            let mut stmt = conn.prepare(
                "SELECT term_norm, COUNT(*) AS total
                 FROM searches
                 WHERE ts >= ?1 AND results = 0
                 GROUP BY term_norm
                 ORDER BY total DESC, term_norm ASC
                 LIMIT ?2",
            )?;
            let rows = stmt.query_map(params![from, limit.clamp(1, 100) as i64], |row| {
                Ok(TrendingTerm {
                    term: row.get(0)?,
                    searches: row.get(1)?,
                })
            })?;
            rows.collect()
        })
        .await
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn normalisation_collapses_case_and_whitespace() {
        assert_eq!(normalise("  The   MATRIX "), "the matrix");
        assert_eq!(normalise("the matrix"), normalise("The Matrix"));
        assert_eq!(normalise(""), "");
    }

    #[test]
    fn a_multi_byte_term_is_truncated_on_a_boundary() {
        let term = "é".repeat(200);
        let cut = truncate(&term, 121);
        assert!(cut.len() <= 121);
        assert!(term.starts_with(&cut));
    }
}

#[cfg(test)]
mod trending_tests {
    use super::*;
    use crate::store::{Store, StoreConfig};

    fn open(name: &str) -> Store {
        let dir =
            std::env::temp_dir().join(format!("same-actor-trend-{}-{name}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("dir");
        Store::open(StoreConfig {
            path: dir.join("test.db"),
            ..StoreConfig::default()
        })
        .expect("open")
    }

    fn event(ts: i64, term: &str) -> SearchEvent {
        SearchEvent {
            ts,
            mode: "title".into(),
            term: term.into(),
            term_norm: normalise(term),
            results: 2,
            session: None,
            ip: None,
        }
    }

    /// The bug this exists to prevent: trending that reads only rolled-up days shows nothing for
    /// the last 24 hours, which is the only window anyone looks at.
    #[tokio::test]
    async fn today_counts_before_anything_has_been_rolled_up() {
        let store = open("today");

        for _ in 0..3 {
            store.record_search(event(crate::clock::now_unix(), "The Matrix"));
        }
        store.record_search(event(crate::clock::now_unix(), "Arrival"));
        store.flush().await.expect("flush");

        let trending = store.trending(1, 1, 10).await.expect("trending");
        assert_eq!(trending.len(), 2);
        assert_eq!(trending[0].term, "the matrix", "case variants are one term");
        assert_eq!(trending[0].searches, 3, "highest count first");

        store.close().await.expect("close");
    }

    /// Rolled-up days and raw days are added together, not one shadowing the other.
    #[tokio::test]
    async fn a_rolled_up_day_and_a_raw_day_are_both_counted() {
        let store = open("mixed");
        let now = crate::clock::now_unix();
        let old = crate::clock::day_start_days_ago(now, 10) + 3_600;

        for _ in 0..2 {
            store.record_search(event(old, "the matrix"));
        }
        store.record_search(event(now, "the matrix"));
        store.flush().await.expect("flush");

        store.maintain(1).await.expect("maintain");

        let trending = store.trending(1, 1, 10).await.expect("trending");
        // The old day is outside a 1-day window, so only today's remains.
        assert_eq!(trending.len(), 1);
        assert_eq!(trending[0].searches, 1);

        // Over 30 days the rolled-up day is included too.
        let trending = store.trending(30, 1, 10).await.expect("trending");
        assert_eq!(trending[0].searches, 3, "2 rolled up + 1 raw");

        store.close().await.expect("close");
    }

    #[tokio::test]
    async fn the_threshold_keeps_a_one_off_search_off_the_page() {
        let store = open("threshold");
        let now = crate::clock::now_unix();

        store.record_search(event(now, "something private"));
        store.record_search(event(now, "the matrix"));
        store.record_search(event(now, "the matrix"));
        store.record_search(event(now, "the matrix"));
        store.flush().await.expect("flush");

        let public = store.trending(1, 3, 10).await.expect("trending");
        assert_eq!(public.len(), 1, "a single search is not trending");
        assert_eq!(public[0].term, "the matrix");

        let admin = store.trending(1, 1, 10).await.expect("trending");
        assert_eq!(admin.len(), 2, "the operator sees everything");

        store.close().await.expect("close");
    }
}
