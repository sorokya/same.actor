//! Comparison view counts.
//!
//! A "comparison" is the canonical set a URL describes — `title:movie-603,movie-604` — not the
//! request that opened it. Counting per request would let one refresh look like ten people, and
//! counting per IP would collapse a household into one.
//!
//! Deduplication lives in memory (see [`crate::views`]): a database round trip on every page view
//! to check "have I seen this session and this key before" costs more than the lost counts on
//! restart are worth.

use rusqlite::{params, Connection};
use serde::Serialize;

use crate::error::AppError;
use crate::store::Store;

#[derive(Debug, Clone, Serialize)]
pub struct Comparison {
    pub key: String,
    pub mode: String,
    pub label: String,
    pub views: i64,
    pub last_seen: i64,
}

/// One view to record.
#[derive(Debug, Clone)]
pub struct NewView {
    /// `title:movie-603,movie-604` — stable, and includes the ids so a retitled film does not
    /// inherit somebody else's count.
    pub key: String,
    pub mode: String,
    /// What to show a human: the selections' names.
    pub label: String,
    /// `[{id,kind,name}]`, so the admin can show what the set was.
    pub picks_json: String,
}

pub(super) fn record_view(conn: &Connection, view: &NewView) -> rusqlite::Result<()> {
    let now = crate::clock::now_unix();

    conn.execute(
        "INSERT INTO comparisons (key, mode, picks_json, label, views, first_seen, last_seen)
         VALUES (?1, ?2, ?3, ?4, 1, ?5, ?5)
         ON CONFLICT(key) DO UPDATE SET
             views     = views + 1,
             last_seen = excluded.last_seen,
             label     = excluded.label,
             picks_json = excluded.picks_json",
        params![view.key, view.mode, view.picks_json, view.label, now],
    )?;

    // The daily row is what makes a trend possible; the cumulative count above cannot say
    // whether a comparison is rising or merely old.
    conn.execute(
        "INSERT INTO daily_comparisons (day, key, views) VALUES (?1, ?2, 1)
         ON CONFLICT(day, key) DO UPDATE SET views = views + 1",
        params![crate::clock::day_string(now), view.key],
    )?;

    Ok(())
}

pub(super) fn count(conn: &Connection, key: &str) -> rusqlite::Result<i64> {
    conn.query_row(
        "SELECT views FROM comparisons WHERE key = ?1",
        params![key],
        |row| row.get(0),
    )
    .or_else(|error| match error {
        rusqlite::Error::QueryReturnedNoRows => Ok(0),
        other => Err(other),
    })
}

/// The canonical key for a comparison. Shared with the URL layer so a page and its API call
/// cannot disagree about which set they are counting.
pub fn key(mode: &str, picks: &[String]) -> String {
    format!("{mode}:{}", picks.join(","))
}

impl Store {
    /// Offer a view. Fire and forget: a page render must not wait on a counter.
    pub fn note_view(&self, view: NewView) -> bool {
        self.offer_view(view)
    }

    pub async fn view_count(&self, key: String) -> Result<i64, AppError> {
        self.read(move |conn| count(conn, &key)).await
    }

    /// Views per day for one comparison, for the trend line.
    pub async fn view_history(
        &self,
        key: String,
        days: i64,
    ) -> Result<Vec<crate::store::report::DayValue>, AppError> {
        let since = crate::clock::day_start_days_ago(crate::clock::now_unix(), days.max(1));
        let from_day = crate::clock::day_string(since);

        self.read(move |conn| {
            let mut stmt = conn.prepare(
                "SELECT day, views FROM daily_comparisons WHERE key = ?1 AND day >= ?2
                 ORDER BY day",
            )?;
            let rows = stmt.query_map(params![key, from_day], |row| {
                Ok(crate::store::report::DayValue {
                    day: row.get(0)?,
                    value: row.get(1)?,
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
    use crate::store::{Store, StoreConfig};

    fn open(name: &str) -> Store {
        let dir =
            std::env::temp_dir().join(format!("same-actor-views-{}-{name}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("dir");
        Store::open(StoreConfig {
            path: dir.join("test.db"),
            ..StoreConfig::default()
        })
        .expect("open")
    }

    fn view(key: &str) -> NewView {
        NewView {
            key: key.into(),
            mode: "title".into(),
            label: "The Matrix + The Matrix Reloaded".into(),
            picks_json: "[]".into(),
        }
    }

    #[test]
    fn a_key_is_stable_for_the_same_set_and_differs_for_another() {
        let a = key("title", &["movie-603".into(), "movie-604".into()]);
        let b = key("title", &["movie-603".into(), "movie-604".into()]);
        let c = key("title", &["movie-603".into(), "movie-605".into()]);

        assert_eq!(a, b);
        assert_ne!(a, c);
        assert_eq!(a, "title:movie-603,movie-604");
    }

    #[tokio::test]
    async fn views_accumulate_for_the_same_comparison() {
        let store = open("accumulate");

        store.note_view(view("title:movie-603,movie-604"));
        store.note_view(view("title:movie-603,movie-604"));
        store.note_view(view("title:movie-603,movie-605"));
        store.flush().await.expect("flush");

        assert_eq!(
            store
                .view_count("title:movie-603,movie-604".into())
                .await
                .expect("count"),
            2
        );
        assert_eq!(
            store
                .view_count("title:movie-603,movie-605".into())
                .await
                .expect("count"),
            1
        );

        store.close().await.expect("close");
    }

    #[tokio::test]
    async fn an_unseen_comparison_counts_zero_rather_than_failing() {
        let store = open("unseen");
        assert_eq!(
            store
                .view_count("title:movie-1,movie-2".into())
                .await
                .expect("count"),
            0
        );
        store.close().await.expect("close");
    }

    #[tokio::test]
    async fn views_are_also_recorded_per_day() {
        let store = open("daily");
        store.note_view(view("title:movie-603,movie-604"));
        store.note_view(view("title:movie-603,movie-604"));
        store.flush().await.expect("flush");

        let history = store
            .view_history("title:movie-603,movie-604".into(), 1)
            .await
            .expect("history");

        assert_eq!(history.len(), 1, "one day of history");
        assert_eq!(history[0].value, 2);

        store.close().await.expect("close");
    }
}
