//! Request records: the write shape the middleware produces and the read shapes the admin
//! API consumes.

use rusqlite::{params, Connection};
use serde::Serialize;

use crate::error::AppError;
use crate::store::Store;

/// What kind of request this was. Recorded so the admin can default to "real traffic" without
/// throwing the rest away.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    /// `/api/*` — the JSON API.
    Api,
    /// A comparison page, rendered for humans and crawlers.
    Page,
    /// A rendered share card.
    Card,
    /// `/api/admin/*`.
    Admin,
    /// Static assets.
    Asset,
    /// `/healthz`.
    Health,
}

impl Kind {
    pub fn as_str(self) -> &'static str {
        match self {
            Kind::Api => "api",
            Kind::Page => "page",
            Kind::Card => "card",
            Kind::Admin => "admin",
            Kind::Asset => "asset",
            Kind::Health => "health",
        }
    }

    /// Classify by path. Order matters: `/api/admin` is an admin request, not an API request,
    /// and `/api` must be tested before the asset rules so `/api/…` is never mistaken for a
    /// file with an extension.
    pub fn of_path(path: &str) -> Self {
        if path.starts_with("/api/admin") {
            Kind::Admin
        } else if path == "/healthz" || path == "/readyz" {
            Kind::Health
        } else if path.starts_with("/api/") {
            Kind::Api
        } else if path.starts_with("/og/") {
            Kind::Card
        } else if path.starts_with("/assets/") || looks_like_a_file(path) {
            Kind::Asset
        } else {
            Kind::Page
        }
    }
}

/// Extensions that mean "an asset", not "a page". Kept short and explicit rather than
/// "anything with a dot", so a future `/discover/top.movies` style route is not swallowed.
fn looks_like_a_file(path: &str) -> bool {
    const ASSET_EXTENSIONS: [&str; 9] = [
        ".js",
        ".css",
        ".map",
        ".png",
        ".jpg",
        ".jpeg",
        ".svg",
        ".ico",
        ".webmanifest",
    ];

    let last = path.rsplit('/').next().unwrap_or_default();
    ASSET_EXTENSIONS
        .iter()
        .any(|extension| last.ends_with(extension))
}

/// One request, as the logging middleware sees it and the writer stores it.
#[derive(Debug, Clone)]
pub struct RequestLog {
    /// Unix seconds, UTC.
    pub ts: i64,
    pub duration_ms: u32,
    pub ip: Option<String>,
    pub session: Option<String>,
    pub ua: Option<String>,
    pub method: String,
    pub path: String,
    pub status: u16,
    /// Stable code from [`crate::error::AppError`], only for failures.
    pub code: Option<&'static str>,
    /// Human message, only for failures.
    pub message: Option<String>,
    pub kind: Kind,
    /// ISO-3166-1 alpha-2, resolved at write time.
    pub country: Option<String>,
}

/// A stored request, as read back.
#[derive(Debug, Clone, Serialize)]
pub struct RequestRow {
    pub id: i64,
    pub ts: i64,
    pub duration_ms: i64,
    pub ip: Option<String>,
    pub session: Option<String>,
    pub ua: Option<String>,
    pub method: String,
    pub path: String,
    pub status: i64,
    pub code: Option<String>,
    pub message: Option<String>,
    pub kind: String,
    pub country: Option<String>,
}

const INSERT: &str = "INSERT INTO requests \
    (ts, duration_ms, ip, session, ua, method, path, status, code, message, kind, country) \
    VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12)";

/// Insert a whole batch inside the caller's transaction.
pub(super) fn insert(conn: &mut Connection, batch: &[RequestLog]) -> rusqlite::Result<()> {
    let tx = conn.transaction()?;

    {
        let mut stmt = tx.prepare_cached(INSERT)?;
        for log in batch {
            stmt.execute(params![
                log.ts,
                log.duration_ms,
                log.ip,
                log.session,
                log.ua,
                log.method,
                log.path,
                log.status,
                log.code,
                log.message,
                log.kind.as_str(),
                log.country,
            ])?;
        }
    }

    tx.commit()
}

const SELECT: &str = "SELECT id, ts, duration_ms, ip, session, ua, method, path, status, \
                      code, message, kind, country FROM requests ORDER BY id DESC LIMIT ?1";

fn row(row: &rusqlite::Row<'_>) -> rusqlite::Result<RequestRow> {
    Ok(RequestRow {
        id: row.get(0)?,
        ts: row.get(1)?,
        duration_ms: row.get(2)?,
        ip: row.get(3)?,
        session: row.get(4)?,
        ua: row.get(5)?,
        method: row.get(6)?,
        path: row.get(7)?,
        status: row.get(8)?,
        code: row.get(9)?,
        message: row.get(10)?,
        kind: row.get(11)?,
        country: row.get(12)?,
    })
}

impl Store {
    /// The most recent records, newest first. This is the shape the log view pages through;
    /// the cursor variants and filters arrive with the admin API in phase 5.
    pub async fn recent_requests(&self, limit: u32) -> Result<Vec<RequestRow>, AppError> {
        let limit = limit.clamp(1, 1_000) as i64;

        self.read(move |conn| {
            let mut stmt = conn.prepare(SELECT)?;
            let rows = stmt.query_map([limit], row)?;
            rows.collect()
        })
        .await
    }

    /// Total records. Cheap enough to expose in the health panel.
    pub async fn request_count(&self) -> Result<i64, AppError> {
        self.read(|conn| conn.query_row("SELECT COUNT(*) FROM requests", [], |row| row.get(0)))
            .await
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn paths_are_classified_by_what_they_cost_and_what_they_serve() {
        assert_eq!(Kind::of_path("/api/search"), Kind::Api);
        assert_eq!(Kind::of_path("/api/findout"), Kind::Api);
        assert_eq!(Kind::of_path("/api/admin/logs"), Kind::Admin);
        assert_eq!(Kind::of_path("/healthz"), Kind::Health);
        assert_eq!(Kind::of_path("/og/title/movie-603.png"), Kind::Card);
        assert_eq!(Kind::of_path("/title/movie-603,movie-604"), Kind::Page);
        assert_eq!(Kind::of_path("/"), Kind::Page);
        assert_eq!(Kind::of_path("/discover/top-movies"), Kind::Page);
        assert_eq!(Kind::of_path("/assets/index-CfHjurDF.css"), Kind::Asset);
        assert_eq!(Kind::of_path("/favicon.ico"), Kind::Asset);
        assert_eq!(Kind::of_path("/apple-touch-icon.png"), Kind::Asset);
    }

    /// `/api/admin` must not be counted as API traffic, or the admin's own dashboard views
    /// would appear in the numbers it is reporting.
    #[test]
    fn admin_is_not_api() {
        assert_ne!(Kind::of_path("/api/admin/overview"), Kind::Api);
    }

    /// A path with a dot in a *segment* is still a page; only a file extension at the end is
    /// an asset.
    #[test]
    fn a_dot_in_a_segment_is_not_a_file_extension() {
        assert_eq!(Kind::of_path("/title/movie-603-the.matrix"), Kind::Page);
    }
}
