//! Schema migrations, stepped by `PRAGMA user_version`.
//!
//! Append-only and never rewritten: a released migration has already run on somebody's
//! database, so the only safe edit is a new entry. Each step runs in its own transaction with
//! the version bump inside it, so a failure leaves the database at the last good version
//! rather than half-migrated.

use rusqlite::Connection;

use crate::error::AppError;

/// Index 0 becomes `user_version = 1`, and so on.
const MIGRATIONS: &[&str] = &[V1, V2, V3];

/// How many answers a comparison produced.
///
/// The `comparisons` table could already say a set was looked at; it could not say whether
/// looking at it was worth anything. The home page wants the sets that *worked* — two films with
/// nothing in common is a page that tells you so, not an example worth offering.
const V3: &str = r#"
ALTER TABLE comparisons ADD COLUMN matches INTEGER NOT NULL DEFAULT 0;
CREATE INDEX comparisons_recent ON comparisons(last_seen DESC);
"#;

/// True per-day totals.
///
/// `daily_requests` is grouped by kind, country and status class, so its `sessions` column is a
/// distinct count *within each group* — summing those overcounts anyone who made two kinds of
/// request. Visitor counts have to be counted once, over the whole day, and that is what this
/// row is.
const V2: &str = r#"
CREATE TABLE daily_totals (
  day      TEXT PRIMARY KEY,
  requests INTEGER NOT NULL,
  errors   INTEGER NOT NULL,
  visitors INTEGER NOT NULL,
  searches INTEGER NOT NULL
) WITHOUT ROWID;
"#;

/// Everything the plan calls for, created up front.
///
/// Later phases only add code: the tables exist and stay empty until something writes to them.
/// That is deliberate — an empty table costs nothing, and it keeps one reviewed schema in one
/// place instead of eight migrations that each need their own reasoning.
const V1: &str = r#"
-- Every request, one row. `code` and `message` are only set for failures.
CREATE TABLE requests (
  id          INTEGER PRIMARY KEY,
  ts          INTEGER NOT NULL,
  duration_ms INTEGER NOT NULL,
  ip          TEXT,
  session     TEXT,
  ua          TEXT,
  method      TEXT NOT NULL,
  path        TEXT NOT NULL,
  status      INTEGER NOT NULL,
  code        TEXT,
  message     TEXT,
  kind        TEXT NOT NULL,
  country     TEXT
);
CREATE INDEX requests_ts         ON requests(ts);
CREATE INDEX requests_ip_ts      ON requests(ip, ts);
CREATE INDEX requests_session_ts ON requests(session, ts);
CREATE INDEX requests_status_ts  ON requests(status, ts);

-- What people looked for, and whether it worked. `results = 0` is free product insight.
CREATE TABLE searches (
  id        INTEGER PRIMARY KEY,
  ts        INTEGER NOT NULL,
  mode      TEXT NOT NULL,
  term      TEXT NOT NULL,
  term_norm TEXT NOT NULL,
  results   INTEGER NOT NULL,
  session   TEXT,
  ip        TEXT
);
CREATE INDEX searches_ts      ON searches(ts);
CREATE INDEX searches_norm_ts ON searches(term_norm, ts);

-- Canonical comparison sets, with their view counts. The key includes the ids so a retitled
-- film does not inherit somebody else's count.
CREATE TABLE comparisons (
  key        TEXT PRIMARY KEY,
  mode       TEXT NOT NULL,
  picks_json TEXT NOT NULL,
  label      TEXT NOT NULL,
  views      INTEGER NOT NULL DEFAULT 0,
  cards      INTEGER NOT NULL DEFAULT 0,
  first_seen INTEGER NOT NULL,
  last_seen  INTEGER NOT NULL
);
CREATE INDEX comparisons_views ON comparisons(views DESC);

-- Bans are an address or a range. There is no session ban: a cookie is client-controlled, so
-- a ban a visitor can clear by pressing Cmd-Shift-N is worse than no ban at all.
CREATE TABLE bans (
  id         INTEGER PRIMARY KEY,
  created_ts INTEGER NOT NULL,
  expires_ts INTEGER,
  target     TEXT NOT NULL,
  kind       TEXT NOT NULL,
  reason     TEXT,
  created_by TEXT NOT NULL,
  hits       INTEGER NOT NULL DEFAULT 0,
  last_hit   INTEGER
);
CREATE UNIQUE INDEX bans_target ON bans(target);

-- Rollups. These survive retention; the raw rows they were built from do not, and neither
-- aggregate carries an IP or a session id.
CREATE TABLE daily_requests (
  day        TEXT NOT NULL,
  kind       TEXT NOT NULL,
  country    TEXT NOT NULL DEFAULT '',
  status_cls INTEGER NOT NULL,
  requests   INTEGER NOT NULL,
  errors     INTEGER NOT NULL,
  sessions   INTEGER NOT NULL,
  p95_ms     INTEGER NOT NULL,
  PRIMARY KEY (day, kind, country, status_cls)
) WITHOUT ROWID;

CREATE TABLE daily_searches (
  day       TEXT NOT NULL,
  mode      TEXT NOT NULL,
  term_norm TEXT NOT NULL,
  searches  INTEGER NOT NULL,
  PRIMARY KEY (day, mode, term_norm)
) WITHOUT ROWID;

CREATE TABLE daily_comparisons (
  day   TEXT NOT NULL,
  key   TEXT NOT NULL,
  views INTEGER NOT NULL,
  PRIMARY KEY (day, key)
) WITHOUT ROWID;

-- TMDB discovery lists, refreshed by a background job so no page waits on upstream.
CREATE TABLE lists (
  id         TEXT PRIMARY KEY,
  fetched_ts INTEGER NOT NULL,
  payload    TEXT NOT NULL
);

-- Cast synergy per list snapshot, recomputed only when the list itself changes.
CREATE TABLE list_synergy (
  list_id     TEXT PRIMARY KEY,
  computed_ts INTEGER NOT NULL,
  payload     TEXT NOT NULL
);
"#;

pub fn migrate(conn: &mut Connection) -> Result<(), AppError> {
    let current: i64 = conn
        .query_row("PRAGMA user_version", [], |row| row.get(0))
        .map_err(|error| AppError::Store(format!("could not read the schema version: {error}")))?;

    let current = current.max(0) as usize;

    if current > MIGRATIONS.len() {
        // Refusing is the only safe answer: the file was written by a newer build, and
        // running old code against a newer schema will corrupt it in ways nobody will enjoy
        // diagnosing.
        return Err(AppError::Store(format!(
            "the database is at schema version {current} but this build only knows {}; \
             upgrade the binary or point DB_PATH somewhere else",
            MIGRATIONS.len()
        )));
    }

    for (index, sql) in MIGRATIONS.iter().enumerate().skip(current) {
        let version = (index + 1) as i64;
        let tx = conn.transaction().map_err(|error| {
            AppError::Store(format!("could not begin migration {version}: {error}"))
        })?;

        tx.execute_batch(sql)
            .map_err(|error| AppError::Store(format!("migration {version} failed: {error}")))?;
        // Inside the transaction, so the version cannot claim a schema that is not there.
        tx.pragma_update(None, "user_version", version)
            .map_err(|error| {
                AppError::Store(format!("could not record version {version}: {error}"))
            })?;

        tx.commit().map_err(|error| {
            AppError::Store(format!("could not commit migration {version}: {error}"))
        })?;

        tracing::info!(version, "applied database migration");
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn memory() -> Connection {
        Connection::open_in_memory().expect("in-memory database")
    }

    fn version(conn: &Connection) -> i64 {
        conn.query_row("PRAGMA user_version", [], |row| row.get(0))
            .expect("user_version")
    }

    #[test]
    fn migrating_an_empty_database_creates_the_schema() {
        let mut conn = memory();
        assert_eq!(version(&conn), 0);

        migrate(&mut conn).expect("migrate");
        assert_eq!(version(&conn), MIGRATIONS.len() as i64);

        for table in [
            "requests",
            "searches",
            "comparisons",
            "bans",
            "daily_requests",
            "daily_searches",
            "daily_comparisons",
            "lists",
            "list_synergy",
        ] {
            let count: i64 = conn
                .query_row(
                    "SELECT COUNT(*) FROM sqlite_master WHERE type = 'table' AND name = ?1",
                    [table],
                    |row| row.get(0),
                )
                .expect("sqlite_master");
            assert_eq!(count, 1, "table {table} is missing");
        }
    }

    #[test]
    fn migrating_twice_is_a_no_op() {
        let mut conn = memory();
        migrate(&mut conn).expect("first");
        migrate(&mut conn).expect("second");
        assert_eq!(version(&conn), MIGRATIONS.len() as i64);
    }

    #[test]
    fn a_newer_database_is_refused_rather_than_downgraded() {
        let mut conn = memory();
        migrate(&mut conn).expect("migrate");

        // Pretend a future build already ran.
        conn.pragma_update(None, "user_version", MIGRATIONS.len() as i64 + 1)
            .expect("bump");

        let error = migrate(&mut conn).expect_err("a newer schema must be refused");
        assert!(
            error.to_string().contains("only knows"),
            "unhelpful message: {error}"
        );
    }

    #[test]
    fn the_expiry_column_is_nullable_so_permanent_means_permanent() {
        let mut conn = memory();
        migrate(&mut conn).expect("migrate");

        conn.execute(
            "INSERT INTO bans (created_ts, expires_ts, target, kind, created_by) \
             VALUES (1, NULL, '203.0.113.7', 'ip', 'admin')",
            [],
        )
        .expect("a ban with no expiry is a permanent ban");

        let expires: Option<i64> = conn
            .query_row("SELECT expires_ts FROM bans", [], |row| row.get(0))
            .expect("read back");
        assert_eq!(expires, None);
    }

    #[test]
    fn one_ban_target_can_only_exist_once() {
        let mut conn = memory();
        migrate(&mut conn).expect("migrate");

        let insert = "INSERT INTO bans (created_ts, target, kind, created_by) \
                      VALUES (1, '203.0.113.7', 'ip', 'admin')";
        conn.execute(insert, []).expect("first");

        assert!(
            conn.execute(insert, []).is_err(),
            "a duplicate ban target must be rejected by the schema, not by a race in the handler"
        );
    }
}
