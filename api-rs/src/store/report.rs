//! Read models for the admin API.
//!
//! Every query here reads from the aggregates first and falls back to raw rows, because a day
//! answers from one place or the other: within the retention window the raw rows still exist,
//! and after it only the aggregates do. Merging in SQL would mean `COALESCE` across two tables
//! with different shapes; merging in Rust is a few lines and obvious.

use rusqlite::{params, params_from_iter, types::Value as SqlValue};
use serde::{Deserialize, Serialize};

use crate::error::AppError;
use crate::store::request::RequestRow;
use crate::store::{searches::TrendingTerm, Store};

/// How far back a request may look.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Window {
    #[serde(rename = "24h")]
    Day,
    #[serde(rename = "7d")]
    Week,
    #[serde(rename = "30d")]
    Month,
}

impl Window {
    pub fn seconds(self) -> i64 {
        match self {
            Window::Day => 86_400,
            Window::Week => 7 * 86_400,
            Window::Month => 30 * 86_400,
        }
    }

    pub fn days(self) -> i64 {
        self.seconds() / 86_400
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Window::Day => "24h",
            Window::Week => "7d",
            Window::Month => "30d",
        }
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct Counts {
    pub requests: i64,
    pub errors: i64,
    pub visitors: i64,
    pub searches: i64,
}

#[derive(Debug, Clone, Serialize)]
pub struct PathRow {
    pub path: String,
    pub requests: i64,
}

#[derive(Debug, Clone, Serialize)]
pub struct CountryRow {
    pub country: String,
    pub requests: i64,
    pub errors: i64,
}

#[derive(Debug, Clone, Serialize)]
pub struct DayValue {
    pub day: String,
    pub value: i64,
}

#[derive(Debug, Clone, Serialize)]
pub struct ComparisonRow {
    pub key: String,
    pub label: String,
    pub mode: String,
    pub views: i64,
    pub last_seen: i64,
}

#[derive(Debug, Clone, Serialize)]
pub struct Overview {
    pub window: String,
    pub counts: Counts,
    /// 95th percentile request duration, in milliseconds.
    pub p95_ms: i64,
    pub top_paths: Vec<PathRow>,
    pub top_countries: Vec<CountryRow>,
    /// Rows still retained at full detail.
    pub log_rows: i64,
    /// The SQLite file, sidecars included.
    pub db_bytes: i64,
}

#[derive(Debug, Clone, Default, Deserialize)]
pub struct LogQuery {
    /// Unix seconds. Defaults to the last 24 hours.
    pub since: Option<i64>,
    pub until: Option<i64>,
    pub ip: Option<String>,
    pub session: Option<String>,
    pub country: Option<String>,
    /// Exact status, or a class like `4xx`.
    pub status: Option<String>,
    /// `api`, `page`, `card`, `admin`, `asset`, `health`.
    pub kind: Option<String>,
    /// Path prefix.
    pub path: Option<String>,
    /// Substring of the message or path.
    pub q: Option<String>,
    pub limit: Option<u32>,
    /// Cursor: return rows with `id` below this.
    pub before_id: Option<i64>,
}

#[derive(Debug, Clone, Serialize)]
pub struct LogPage {
    pub rows: Vec<RequestRow>,
    /// Pass back as `before_id` for the next page. `None` when the page was not full.
    pub next_before_id: Option<i64>,
}

#[derive(Debug, Clone, Serialize)]
pub struct Health {
    pub db_bytes: i64,
    pub log_rows: i64,
    pub search_rows: i64,
    pub bans: usize,
    /// Records the writer discarded because the queue was full. Non-zero means the writer is
    /// not keeping up, and it is better to see that number than to lose history silently.
    pub dropped: u64,
    pub rows_written: u64,
    pub commits: u64,
    /// `None` means no geo database is loaded.
    pub geo_age_secs: Option<i64>,
    pub geo_enabled: bool,
}

impl Store {
    /// Counts over a window, from the aggregates where they exist and from raw rows where the
    /// day has not been rolled up yet.
    pub async fn counts(&self, window: Window) -> Result<Counts, AppError> {
        let now = crate::clock::now_unix();
        let since = crate::clock::day_start_days_ago(now, window.days());
        let today = crate::clock::day_string(since);

        self.read(move |conn| {
            let (mut requests, mut errors, mut visitors, mut searches) = (0, 0, 0, 0);

            // Rolled-up days.
            let mut stmt = conn.prepare(
                "SELECT requests, errors, visitors, searches FROM daily_totals WHERE day >= ?1",
            )?;
            let rows = stmt.query_map(params![today], |row| {
                Ok((
                    row.get::<_, i64>(0)?,
                    row.get::<_, i64>(1)?,
                    row.get::<_, i64>(2)?,
                    row.get::<_, i64>(3)?,
                ))
            })?;
            let mut rolled_days = 0;
            for row in rows {
                let (r, e, v, s) = row?;
                requests += r;
                errors += e;
                visitors += v;
                searches += s;
                rolled_days += 1;
            }

            // Days still at full detail. Counted from raw so visitors are distinct per day
            // rather than summed from overlapping groups.
            let mut stmt = conn.prepare(
                "SELECT COUNT(*), SUM(status >= 400), COUNT(DISTINCT session)
                 FROM requests WHERE ts >= ?1",
            )?;
            let (raw_requests, raw_errors, raw_visitors): (i64, Option<i64>, i64) = stmt
                .query_row(params![since], |row| {
                    Ok((row.get(0)?, row.get(1)?, row.get(2)?))
                })?;

            let raw_searches: i64 = conn.query_row(
                "SELECT COUNT(*) FROM searches WHERE ts >= ?1",
                params![since],
                |row| row.get(0),
            )?;

            // Both sources are disjoint by construction: rollup deletes the rows it
            // summarises, so a day appears in exactly one of them.
            let _ = rolled_days;
            requests += raw_requests;
            errors += raw_errors.unwrap_or(0);
            visitors += raw_visitors;
            searches += raw_searches;

            Ok(Counts {
                requests,
                errors,
                visitors,
                searches,
            })
        })
        .await
    }

    pub async fn p95_ms(&self, window: Window) -> Result<i64, AppError> {
        let since = crate::clock::now_unix() - window.seconds();

        self.read(move |conn| {
            conn.query_row(
                "SELECT duration_ms FROM requests WHERE ts >= ?1
                 ORDER BY duration_ms
                 LIMIT 1 OFFSET (
                     SELECT MAX(0, (COUNT(*) * 95 / 100) - 1) FROM requests WHERE ts >= ?1
                 )",
                params![since],
                |row| row.get(0),
            )
            .or_else(|error| match error {
                rusqlite::Error::QueryReturnedNoRows => Ok(0),
                other => Err(other),
            })
        })
        .await
    }

    pub async fn top_paths(&self, window: Window, limit: u32) -> Result<Vec<PathRow>, AppError> {
        let since = crate::clock::now_unix() - window.seconds();
        let limit = limit.clamp(1, 50) as i64;

        self.read(move |conn| {
            let mut stmt = conn.prepare(
                "SELECT path, COUNT(*) AS requests FROM requests
                 WHERE ts >= ?1 GROUP BY path ORDER BY requests DESC, path LIMIT ?2",
            )?;
            let rows = stmt.query_map(params![since, limit], |row| {
                Ok(PathRow {
                    path: row.get(0)?,
                    requests: row.get(1)?,
                })
            })?;
            rows.collect()
        })
        .await
    }

    /// Countries over a window, from the aggregates.
    pub async fn geo(&self, days: i64) -> Result<Vec<CountryRow>, AppError> {
        let today = crate::clock::day_string(crate::clock::day_start_days_ago(
            crate::clock::now_unix(),
            days.max(1),
        ));

        self.read(move |conn| {
            let mut stmt = conn.prepare(
                "SELECT country, SUM(requests), SUM(errors) FROM daily_requests
                 WHERE day >= ?1 AND country != ''
                 GROUP BY country ORDER BY SUM(requests) DESC, country LIMIT 60",
            )?;
            let rows = stmt.query_map(params![today], |row| {
                Ok(CountryRow {
                    country: row.get(0)?,
                    requests: row.get(1)?,
                    errors: row.get(2)?,
                })
            })?;
            rows.collect()
        })
        .await
    }

    /// One value per day for a chart.
    pub async fn timeseries(
        &self,
        metric: Metric,
        window: Window,
    ) -> Result<Vec<DayValue>, AppError> {
        let now = crate::clock::now_unix();
        let days = window.days();
        let since_day = crate::clock::day_start_days_ago(now, days);

        self.read(move |conn| {
            let column = match metric {
                Metric::Requests => "requests",
                Metric::Errors => "errors",
                Metric::Visitors => "visitors",
                Metric::Searches => "searches",
            };

            let mut by_day: std::collections::BTreeMap<String, i64> =
                std::collections::BTreeMap::new();

            // Every day in the window, so a chart has no holes where nothing happened.
            for offset in (0..=days).rev() {
                by_day.insert(
                    crate::clock::day_string(crate::clock::day_start_days_ago(now, offset)),
                    0,
                );
            }

            let mut stmt = conn.prepare(&format!(
                "SELECT day, {column} FROM daily_totals WHERE day >= ?1"
            ))?;
            let rows = stmt.query_map(params![crate::clock::day_string(since_day)], |row| {
                Ok((row.get::<_, String>(0)?, row.get::<_, i64>(1)?))
            })?;
            for row in rows {
                let (day, value) = row?;
                by_day.insert(day, value);
            }

            // The current day (and anything not yet rolled up) comes from raw rows.
            let expr = match metric {
                Metric::Errors => "SUM(status >= 400)",
                Metric::Visitors => "COUNT(DISTINCT session)",
                _ => "COUNT(*)",
            };
            let table = match metric {
                Metric::Searches => "searches",
                _ => "requests",
            };
            let mut stmt = conn.prepare(&format!(
                "SELECT date(ts, 'unixepoch') AS day, {expr} FROM {table}
                 WHERE ts >= ?1 GROUP BY day"
            ))?;
            let rows = stmt.query_map(params![since_day], |row| {
                Ok((row.get::<_, String>(0)?, row.get::<_, Option<i64>>(1)?))
            })?;
            for row in rows {
                let (day, value) = row?;
                // Raw wins: if the day is still at full detail, the aggregate for it does not
                // exist, and if both somehow do, the raw rows are the more current truth.
                if let Some(value) = value {
                    by_day.insert(day, value);
                }
            }

            Ok(by_day
                .into_iter()
                .map(|(day, value)| DayValue { day, value })
                .collect())
        })
        .await
    }

    pub async fn comparisons(&self, limit: u32) -> Result<Vec<ComparisonRow>, AppError> {
        let limit = limit.clamp(1, 100) as i64;

        self.read(move |conn| {
            let mut stmt = conn.prepare(
                "SELECT key, label, mode, views, last_seen FROM comparisons
                 ORDER BY views DESC, last_seen DESC LIMIT ?1",
            )?;
            let rows = stmt.query_map([limit], |row| {
                Ok(ComparisonRow {
                    key: row.get(0)?,
                    label: row.get(1)?,
                    mode: row.get(2)?,
                    views: row.get(3)?,
                    last_seen: row.get(4)?,
                })
            })?;
            rows.collect()
        })
        .await
    }

    /// A page of the request log. Filters compose; the cursor is `id < before_id` so paging is
    /// stable while new rows arrive above it.
    pub async fn logs(&self, query: LogQuery) -> Result<LogPage, AppError> {
        let now = crate::clock::now_unix();
        let since = query.since.unwrap_or(now - 86_400);
        let until = query.until.unwrap_or(now);
        let limit = query.limit.unwrap_or(100).clamp(1, 500) as i64;

        self.read(move |conn| {
            let mut sql = String::from(
                "SELECT id, ts, duration_ms, ip, session, ua, method, path, status, code,
                        message, kind, country
                 FROM requests WHERE ts >= ?1 AND ts <= ?2",
            );
            let mut values: Vec<SqlValue> = vec![since.into(), until.into()];

            if let Some(ip) = query.ip {
                sql.push_str(" AND ip = ?");
                values.push(ip.into());
            }
            if let Some(session) = query.session {
                sql.push_str(" AND session = ?");
                values.push(session.into());
            }
            if let Some(country) = query.country {
                sql.push_str(" AND country = ?");
                values.push(country.into());
            }
            if let Some(status) = query.status {
                // `4xx` means the class; `404` means the code.
                if let Some(class) = status.strip_suffix("xx") {
                    if let Ok(class) = class.parse::<i64>() {
                        sql.push_str(" AND status / 100 = ?");
                        values.push(class.into());
                    }
                } else if let Ok(code) = status.parse::<i64>() {
                    sql.push_str(" AND status = ?");
                    values.push(code.into());
                }
            }
            if let Some(kind) = query.kind {
                sql.push_str(" AND kind = ?");
                values.push(kind.into());
            }
            if let Some(path) = query.path {
                sql.push_str(" AND path LIKE ?");
                values.push(format!("{path}%").into());
            }
            if let Some(q) = query.q {
                sql.push_str(" AND (path LIKE ? OR message LIKE ? OR ua LIKE ?)");
                let needle = format!("%{q}%");
                values.push(needle.clone().into());
                values.push(needle.clone().into());
                values.push(needle.into());
            }
            if let Some(before) = query.before_id {
                sql.push_str(" AND id < ?");
                values.push(before.into());
            }

            sql.push_str(" ORDER BY id DESC LIMIT ?");
            values.push((limit + 1).into());

            let mut stmt = conn.prepare(&sql)?;
            let mut rows: Vec<RequestRow> = stmt
                .query_map(params_from_iter(values), crate::store::request::from_row)?
                .collect::<rusqlite::Result<_>>()?;

            // One extra row tells us whether there is a next page, without a second query.
            let next_before_id = (rows.len() as i64 > limit).then(|| {
                rows.truncate(limit as usize);
                rows.last().map(|row| row.id)
            });

            Ok(LogPage {
                next_before_id: next_before_id.flatten(),
                rows,
            })
        })
        .await
    }

    pub async fn log_by_id(&self, id: i64) -> Result<Option<RequestRow>, AppError> {
        self.read(move |conn| {
            conn.query_row(
                "SELECT id, ts, duration_ms, ip, session, ua, method, path, status, code,
                        message, kind, country
                 FROM requests WHERE id = ?1",
                params![id],
                crate::store::request::from_row,
            )
            .map(Some)
            .or_else(|error| match error {
                rusqlite::Error::QueryReturnedNoRows => Ok(None),
                other => Err(other),
            })
        })
        .await
    }

    pub async fn trending_terms(
        &self,
        window: Window,
        limit: u32,
    ) -> Result<Vec<TrendingTerm>, AppError> {
        // The public threshold is 3; the admin sees everything, because the operator's own
        // curiosity is not the same thing as publishing a stranger's search.
        self.trending(window.days(), 1, limit).await
    }

    pub async fn health(
        &self,
        bans: usize,
        geo_age_secs: Option<i64>,
        geo_enabled: bool,
    ) -> Result<Health, AppError> {
        let stats = self.stats();
        let path = self.path().to_path_buf();

        let (log_rows, search_rows) = self
            .read(|conn| {
                let log_rows: i64 =
                    conn.query_row("SELECT COUNT(*) FROM requests", [], |row| row.get(0))?;
                let search_rows: i64 =
                    conn.query_row("SELECT COUNT(*) FROM searches", [], |row| row.get(0))?;
                Ok((log_rows, search_rows))
            })
            .await?;

        let db_bytes = db_bytes(&path);

        Ok(Health {
            db_bytes,
            log_rows,
            search_rows,
            bans,
            dropped: stats.dropped,
            rows_written: stats.rows,
            commits: stats.commits,
            geo_age_secs,
            geo_enabled,
        })
    }
}

impl Store {
    pub async fn log_row_count(&self) -> Result<i64, AppError> {
        self.request_count().await
    }

    /// Size on disk, sidecars included.
    pub async fn db_bytes(&self) -> Result<i64, AppError> {
        Ok(db_bytes(self.path()))
    }
}

/// The database plus its write-ahead log and shared-memory sidecar, which is what actually
/// occupies the disk.
fn db_bytes(path: &std::path::Path) -> i64 {
    let mut total = 0;

    for suffix in ["", "-wal", "-shm"] {
        let candidate = std::path::PathBuf::from(format!("{}{suffix}", path.display()));
        total += std::fs::metadata(candidate)
            .map(|meta| meta.len())
            .unwrap_or(0) as i64;
    }

    total
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Metric {
    Requests,
    Errors,
    Visitors,
    Searches,
}

/// One row of the suspicious-client report, with the evidence that produced the score.
#[derive(Debug, Clone, Serialize)]
pub struct Suspicion {
    pub ip: String,
    pub requests: i64,
    pub errors: i64,
    pub limited: i64,
    pub paths: i64,
    pub sessions: i64,
    pub score: i64,
    /// Every reason that contributed, in plain words.
    pub reasons: Vec<String>,
    pub first_seen: i64,
    pub last_seen: i64,
}

impl Store {
    /// Aggregate per address over a window, then score in Rust.
    ///
    /// The scoring lives here rather than in SQL because every row must carry the reasons that
    /// produced it: a number with no explanation is not something an operator can act on.
    pub async fn suspicious(&self, window: Window, limit: u32) -> Result<Vec<Suspicion>, AppError> {
        let since = crate::clock::now_unix() - window.seconds();

        let raw = self
            .read(move |conn| {
                let mut stmt = conn.prepare(
                    "SELECT ip, COUNT(*) AS requests,
                            SUM(status >= 400) AS errors,
                            SUM(status = 429) AS limited,
                            COUNT(DISTINCT path) AS paths,
                            COUNT(DISTINCT session) AS sessions,
                            COUNT(*) FILTER (WHERE ua IS NULL OR ua = '') AS no_ua,
                            MIN(ts) AS first_seen,
                            MAX(ts) AS last_seen
                     FROM requests
                     WHERE ts >= ?1 AND ip IS NOT NULL AND kind != 'asset'
                     GROUP BY ip
                     ORDER BY requests DESC
                     LIMIT 500",
                )?;

                let rows = stmt.query_map(params![since], |row| {
                    Ok(RawSuspicion {
                        ip: row.get(0)?,
                        requests: row.get(1)?,
                        errors: row.get(2).unwrap_or(0),
                        limited: row.get(3).unwrap_or(0),
                        paths: row.get(4)?,
                        sessions: row.get(5)?,
                        no_ua: row.get(6).unwrap_or(0),
                        first_seen: row.get(7)?,
                        last_seen: row.get(8)?,
                    })
                })?;

                rows.collect::<rusqlite::Result<Vec<_>>>()
            })
            .await?;

        Ok(score(raw, limit))
    }
}

struct RawSuspicion {
    ip: String,
    requests: i64,
    errors: i64,
    limited: i64,
    paths: i64,
    sessions: i64,
    no_ua: i64,
    first_seen: i64,
    last_seen: i64,
}

/// Turn per-address aggregates into a ranked list with reasons.
///
/// The thresholds are deliberately blunt and documented in the plan: this is a triage list for a
/// human, not an automatic ban. Nothing here bans anyone — the automatic rules are separate,
/// opt-in and configurable.
fn score(rows: Vec<RawSuspicion>, limit: u32) -> Vec<Suspicion> {
    // The 99th percentile of request volume across the window, as a baseline for "unusual".
    let mut volumes: Vec<i64> = rows.iter().map(|row| row.requests).collect();
    volumes.sort_unstable();
    let median = if volumes.is_empty() {
        0
    } else {
        volumes[volumes.len() / 2]
    };

    let mut scored: Vec<Suspicion> = rows
        .into_iter()
        .filter_map(|row| {
            let mut score = 0;
            let mut reasons = Vec::new();

            // A single address responsible for a large multiple of the median is the clearest
            // signal there is.
            if median > 0 && row.requests > median.saturating_mul(20) {
                score += 3;
                reasons.push(format!(
                    "{} requests, {}× the median address",
                    row.requests,
                    row.requests / median.max(1)
                ));
            }

            // Walking an id space: many distinct paths from few sessions.
            if row.paths > 50 && row.sessions <= row.paths / 10 {
                score += 3;
                reasons.push(format!(
                    "{} distinct paths from {} sessions — id walking",
                    row.paths, row.sessions
                ));
            }

            if row.requests >= 20 && row.errors * 100 / row.requests > 40 {
                score += 2;
                reasons.push(format!(
                    "{}% of {} requests failed",
                    row.errors * 100 / row.requests,
                    row.requests
                ));
            }

            if row.no_ua * 100 / row.requests.max(1) > 50 {
                score += 2;
                reasons.push("most requests carried no user agent".into());
            }

            if row.limited >= 10 {
                score += 2;
                reasons.push(format!("{} rate-limit refusals", row.limited));
            }

            if score < 2 {
                return None;
            }

            Some(Suspicion {
                ip: row.ip,
                requests: row.requests,
                errors: row.errors,
                limited: row.limited,
                paths: row.paths,
                sessions: row.sessions,
                score,
                reasons,
                first_seen: row.first_seen,
                last_seen: row.last_seen,
            })
        })
        .collect();

    scored.sort_by(|a, b| {
        b.score
            .cmp(&a.score)
            .then(b.requests.cmp(&a.requests))
            .then(a.ip.cmp(&b.ip))
    });
    scored.truncate(limit.clamp(1, 200) as usize);
    scored
}

#[cfg(test)]
mod tests {
    use super::*;

    fn raw(ip: &str, requests: i64, errors: i64, paths: i64, sessions: i64) -> RawSuspicion {
        RawSuspicion {
            ip: ip.into(),
            requests,
            errors,
            limited: 0,
            paths,
            sessions,
            no_ua: 0,
            first_seen: 0,
            last_seen: 1,
        }
    }

    #[test]
    fn an_id_walker_ranks_first_with_a_reason() {
        let scored = score(
            vec![
                // Twenty honest visitors making a handful of requests each.
                raw("198.51.100.1", 5, 0, 3, 1),
                raw("198.51.100.2", 4, 0, 2, 1),
                raw("198.51.100.3", 6, 0, 4, 1),
                // One client walking ids.
                raw("203.0.113.9", 400, 380, 400, 2),
            ],
            10,
        );

        assert_eq!(scored[0].ip, "203.0.113.9");
        assert!(scored[0].score >= 5);
        assert!(
            scored[0]
                .reasons
                .iter()
                .any(|reason| reason.contains("id walking")),
            "reasons: {:?}",
            scored[0].reasons
        );
        assert!(
            scored[0]
                .reasons
                .iter()
                .any(|reason| reason.contains("failed")),
            "a high error rate should be named"
        );
        assert!(
            !scored.iter().any(|row| row.ip == "198.51.100.1"),
            "an honest visitor must not appear at all"
        );
    }

    #[test]
    fn one_quiet_client_is_not_suspicious() {
        let scored = score(vec![raw("203.0.113.7", 3, 0, 2, 1)], 10);
        assert!(scored.is_empty(), "a few requests is not a finding");
    }

    #[test]
    fn the_limit_is_respected() {
        let rows = (0..50)
            .map(|i| raw(&format!("203.0.113.{i}"), 500, 490, 500, 1))
            .collect();
        assert_eq!(score(rows, 5).len(), 5);
    }

    #[test]
    fn a_window_maps_to_a_duration() {
        assert_eq!(Window::Day.seconds(), 86_400);
        assert_eq!(Window::Week.days(), 7);
        assert_eq!(Window::Month.days(), 30);
    }
}
