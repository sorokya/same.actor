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

const TRENDING: &str = "SELECT term_norm, SUM(searches) AS total
     FROM daily_searches
     WHERE day >= ?1
     GROUP BY term_norm
     HAVING total >= ?2
     ORDER BY total DESC, term_norm ASC
     LIMIT ?3";

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
        let from_day = crate::clock::day_string(crate::clock::day_start_days_ago(
            crate::clock::now_unix(),
            days.max(1),
        ));
        let limit = limit.clamp(1, 100) as i64;

        self.read(move |conn| {
            let mut stmt = conn.prepare(TRENDING)?;
            let rows = stmt.query_map(params![from_day, min_searches, limit], |row| {
                Ok(TrendingTerm {
                    term: row.get(0)?,
                    searches: row.get(1)?,
                })
            })?;
            rows.collect()
        })
        .await
    }

    /// Terms that returned nothing. Free product insight: what people wanted and the catalogue
    /// could not give them.
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
