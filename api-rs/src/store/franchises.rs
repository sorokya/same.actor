//! Franchises, the curated groups the synergy views cluster by.
//!
//! TMDB has no franchise field. Its collections are too narrow (Iron Man, The Avengers and
//! Guardians are three collections and one franchise) and its keywords are noisy, so the grouping
//! is a decision the maintainer makes and this table records. What TMDB gives us is stored on the
//! list items and offered as *suggestions* in the admin; nothing here is inferred.
//!
//! One title belongs to at most one franchise, and the primary key says so — `(kind, media_id)`
//! is the whole table's identity, so a title cannot be assigned twice and a reassignment is an
//! upsert rather than a cleanup.

use std::collections::HashMap;

use rusqlite::{params, Connection, OptionalExtension};

use crate::error::AppError;
use crate::store::Store;

/// A curated group.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Franchise {
    pub id: i64,
    /// URL-stable: renaming changes `name`, never `slug`, so a link keeps working.
    pub slug: String,
    pub name: String,
    /// Colour comes from the fixed palette by position, not from a column: an admin-chosen
    /// palette is how a twenty-column matrix ends up with two indistinguishable purples.
    pub sort_order: i64,
    pub titles: usize,
}

/// One title inside a franchise.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Assigned {
    pub kind: String,
    pub media_id: u32,
    pub name: String,
    pub poster: Option<String>,
}

/// An assignment to record: which franchise, and which title.
#[derive(Debug, Clone)]
pub struct Assign {
    pub franchise_id: i64,
    pub kind: String,
    pub media_id: u32,
    pub name: String,
    pub poster: Option<String>,
}

/// A cameo flag that overrides the Billing-order cutoff.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Cameo {
    pub kind: String,
    pub media_id: u32,
    pub person_id: u32,
    /// `true` forces "this is a cameo"; `false` forces "this is not", which is the half the
    /// cutoff gets wrong more often.
    pub cameo: bool,
}

/// Where a title sits, for the views that cluster by franchise.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Placement {
    pub franchise_id: i64,
    pub slug: String,
    pub name: String,
    pub sort_order: i64,
}

/// A slug for a franchise name, unique in the table.
///
/// Names are not always Latin script, and `slugify` reduces those to nothing; the fallback keeps
/// a working URL rather than an empty segment.
fn unique_slug(conn: &Connection, name: &str, skip: Option<i64>) -> rusqlite::Result<String> {
    let base = crate::paths::slugify(name);
    let base = if base.is_empty() {
        "franchise".to_owned()
    } else {
        base
    };

    let mut candidate = base.clone();
    let mut suffix = 2;

    loop {
        let taken: Option<i64> = conn
            .query_row(
                "SELECT id FROM franchises WHERE slug = ?1",
                params![candidate],
                |row| row.get(0),
            )
            .ok();

        match taken {
            Some(id) if Some(id) != skip => {
                candidate = format!("{base}-{suffix}");
                suffix += 1;
            }
            _ => return Ok(candidate),
        }
    }
}

pub(super) fn all(conn: &Connection) -> rusqlite::Result<Vec<Franchise>> {
    let mut stmt = conn.prepare(
        "SELECT f.id, f.slug, f.name, f.sort_order, COUNT(t.media_id)
         FROM franchises f
         LEFT JOIN franchise_titles t ON t.franchise_id = f.id
         GROUP BY f.id
         ORDER BY f.sort_order, f.name",
    )?;

    let rows = stmt.query_map([], |row| {
        Ok(Franchise {
            id: row.get(0)?,
            slug: row.get(1)?,
            name: row.get(2)?,
            sort_order: row.get(3)?,
            titles: row.get::<_, i64>(4)? as usize,
        })
    })?;

    rows.collect()
}

pub(super) fn by_slug(conn: &Connection, slug: &str) -> rusqlite::Result<Option<Franchise>> {
    conn.query_row(
        "SELECT f.id, f.slug, f.name, f.sort_order, COUNT(t.media_id)
         FROM franchises f
         LEFT JOIN franchise_titles t ON t.franchise_id = f.id
         WHERE f.slug = ?1
         GROUP BY f.id",
        params![slug],
        |row| {
            Ok(Franchise {
                id: row.get(0)?,
                slug: row.get(1)?,
                name: row.get(2)?,
                sort_order: row.get(3)?,
                titles: row.get::<_, i64>(4)? as usize,
            })
        },
    )
    .optional()
}

pub(super) fn titles(conn: &Connection, franchise_id: i64) -> rusqlite::Result<Vec<Assigned>> {
    let mut stmt = conn.prepare(
        "SELECT kind, media_id, name, poster FROM franchise_titles
         WHERE franchise_id = ?1 ORDER BY name",
    )?;

    let rows = stmt.query_map(params![franchise_id], |row| {
        Ok(Assigned {
            kind: row.get(0)?,
            media_id: row.get(1)?,
            name: row.get(2)?,
            poster: row.get(3)?,
        })
    })?;

    rows.collect()
}

pub(super) fn insert(conn: &Connection, name: &str) -> rusqlite::Result<i64> {
    let slug = unique_slug(conn, name, None)?;
    let next: i64 = conn.query_row(
        "SELECT COALESCE(MAX(sort_order), 0) + 1 FROM franchises",
        [],
        |row| row.get(0),
    )?;

    conn.execute(
        "INSERT INTO franchises (slug, name, sort_order) VALUES (?1, ?2, ?3)",
        params![slug, name, next],
    )?;

    Ok(conn.last_insert_rowid())
}

pub(super) fn rename(conn: &Connection, id: i64, name: &str) -> rusqlite::Result<bool> {
    let changed = conn.execute(
        "UPDATE franchises SET name = ?2 WHERE id = ?1",
        params![id, name],
    )?;

    Ok(changed > 0)
}

/// Deleting a franchise gives its titles back to the queue: the titles are unassigned, not
/// deleted, because a franchise is a label on a title and the title is not ours to delete.
pub(super) fn delete(conn: &Connection, id: i64) -> rusqlite::Result<bool> {
    conn.execute(
        "DELETE FROM franchise_titles WHERE franchise_id = ?1",
        params![id],
    )?;
    let changed = conn.execute("DELETE FROM franchises WHERE id = ?1", params![id])?;

    Ok(changed > 0)
}

pub(super) fn assign(conn: &Connection, assign: &Assign) -> rusqlite::Result<()> {
    conn.execute(
        "INSERT INTO franchise_titles (franchise_id, kind, media_id, name, poster, added_ts)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6)
         ON CONFLICT(kind, media_id) DO UPDATE SET
             franchise_id = excluded.franchise_id,
             name         = excluded.name,
             poster       = excluded.poster",
        params![
            assign.franchise_id,
            assign.kind,
            assign.media_id,
            assign.name,
            assign.poster,
            crate::clock::now_unix()
        ],
    )?;

    Ok(())
}

pub(super) fn unassign(conn: &Connection, kind: &str, media_id: u32) -> rusqlite::Result<bool> {
    let changed = conn.execute(
        "DELETE FROM franchise_titles WHERE kind = ?1 AND media_id = ?2",
        params![kind, media_id],
    )?;

    Ok(changed > 0)
}

pub(super) fn set_cameo(conn: &Connection, cameo: &Cameo) -> rusqlite::Result<()> {
    conn.execute(
        "INSERT INTO cameo_overrides (kind, media_id, person_id, cameo)
         VALUES (?1, ?2, ?3, ?4)
         ON CONFLICT(kind, media_id, person_id) DO UPDATE SET cameo = excluded.cameo",
        params![
            cameo.kind,
            cameo.media_id,
            cameo.person_id,
            if cameo.cameo { 1 } else { 0 }
        ],
    )?;

    Ok(())
}

pub(super) fn clear_cameo(
    conn: &Connection,
    kind: &str,
    media_id: u32,
    person_id: u32,
) -> rusqlite::Result<bool> {
    let changed = conn.execute(
        "DELETE FROM cameo_overrides WHERE kind = ?1 AND media_id = ?2 AND person_id = ?3",
        params![kind, media_id, person_id],
    )?;

    Ok(changed > 0)
}

/// Every override for one title.
pub(super) fn cameos(conn: &Connection, kind: &str, media_id: u32) -> rusqlite::Result<Vec<Cameo>> {
    let mut stmt = conn.prepare(
        "SELECT person_id, cameo FROM cameo_overrides WHERE kind = ?1 AND media_id = ?2",
    )?;

    let rows = stmt.query_map(params![kind, media_id], |row| {
        Ok(Cameo {
            kind: kind.to_owned(),
            media_id,
            person_id: row.get(0)?,
            cameo: row.get::<_, i64>(1)? == 1,
        })
    })?;

    rows.collect()
}

/// Where every assigned title sits, keyed by kind and id.
pub(super) fn placements(conn: &Connection) -> rusqlite::Result<HashMap<(String, u32), Placement>> {
    let mut stmt = conn.prepare(
        "SELECT t.kind, t.media_id, f.id, f.slug, f.name, f.sort_order
         FROM franchise_titles t
         JOIN franchises f ON f.id = t.franchise_id",
    )?;

    let rows = stmt.query_map([], |row| {
        Ok((
            (row.get::<_, String>(0)?, row.get::<_, u32>(1)?),
            Placement {
                franchise_id: row.get(2)?,
                slug: row.get(3)?,
                name: row.get(4)?,
                sort_order: row.get(5)?,
            },
        ))
    })?;

    rows.collect()
}

impl Store {
    pub async fn franchises(&self) -> Result<Vec<Franchise>, AppError> {
        self.read(all).await
    }

    pub async fn franchise(&self, slug: String) -> Result<Option<Franchise>, AppError> {
        self.read(move |conn| by_slug(conn, &slug)).await
    }

    pub async fn franchise_titles(&self, id: i64) -> Result<Vec<Assigned>, AppError> {
        self.read(move |conn| titles(conn, id)).await
    }

    /// Every assignment, for the views that cluster by franchise.
    pub async fn placements(&self) -> Result<HashMap<(String, u32), Placement>, AppError> {
        self.read(placements).await
    }

    pub async fn cameos(&self, kind: String, media_id: u32) -> Result<Vec<Cameo>, AppError> {
        self.read(move |conn| cameos(conn, &kind, media_id)).await
    }

    /// Create a franchise and return its id.
    pub async fn create_franchise(&self, name: String) -> Result<i64, AppError> {
        self.submit(move |ack| {
            crate::store::WriteOp::Franchise(crate::store::FranchiseOp::Create(name, ack))
        })
        .await
    }

    pub async fn rename_franchise(&self, id: i64, name: String) -> Result<bool, AppError> {
        self.submit(move |ack| {
            crate::store::WriteOp::Franchise(crate::store::FranchiseOp::Rename(id, name, ack))
        })
        .await
    }

    pub async fn delete_franchise(&self, id: i64) -> Result<bool, AppError> {
        self.submit(move |ack| {
            crate::store::WriteOp::Franchise(crate::store::FranchiseOp::Delete(id, ack))
        })
        .await
    }

    pub async fn assign_franchise(&self, assign: Assign) -> Result<(), AppError> {
        self.submit(move |ack| {
            crate::store::WriteOp::Franchise(crate::store::FranchiseOp::Assign(assign, ack))
        })
        .await
    }

    pub async fn unassign_franchise(&self, kind: String, media_id: u32) -> Result<bool, AppError> {
        self.submit(move |ack| {
            crate::store::WriteOp::Franchise(crate::store::FranchiseOp::Unassign(
                kind, media_id, ack,
            ))
        })
        .await
    }

    pub async fn set_cameo(&self, cameo: Cameo) -> Result<(), AppError> {
        self.submit(move |ack| {
            crate::store::WriteOp::Franchise(crate::store::FranchiseOp::Cameo(cameo, ack))
        })
        .await
    }

    pub async fn clear_cameo(
        &self,
        kind: String,
        media_id: u32,
        person_id: u32,
    ) -> Result<bool, AppError> {
        self.submit(move |ack| {
            crate::store::WriteOp::Franchise(crate::store::FranchiseOp::ClearCameo(
                kind, media_id, person_id, ack,
            ))
        })
        .await
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::store::StoreConfig;

    fn open(name: &str) -> Store {
        let dir = std::env::temp_dir().join(format!(
            "same-actor-franchises-{}-{name}",
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("dir");
        Store::open(StoreConfig {
            path: dir.join("test.db"),
            ..StoreConfig::default()
        })
        .expect("open")
    }

    fn assign(franchise_id: i64, id: u32, name: &str) -> Assign {
        Assign {
            franchise_id,
            kind: "movie".to_owned(),
            media_id: id,
            name: name.to_owned(),
            poster: None,
        }
    }

    #[tokio::test]
    async fn a_title_belongs_to_one_franchise_at_a_time() {
        let store = open("one-per-title");
        let marvel = store
            .create_franchise("Marvel".into())
            .await
            .expect("create");
        let xmen = store
            .create_franchise("X-Men".into())
            .await
            .expect("create");

        store
            .assign_franchise(assign(marvel, 603, "The Matrix"))
            .await
            .expect("assign");
        store
            .assign_franchise(assign(xmen, 603, "The Matrix"))
            .await
            .expect("reassign");

        let titles = store.franchise_titles(xmen).await.expect("titles");
        assert_eq!(
            titles.len(),
            1,
            "the title moved rather than being duplicated"
        );
        assert_eq!(
            store.franchise_titles(marvel).await.expect("titles").len(),
            0
        );

        store.close().await.expect("close");
    }

    #[tokio::test]
    async fn slugs_are_unique_and_stable_across_a_rename() {
        let store = open("slugs");
        let first = store
            .create_franchise("Marvel".into())
            .await
            .expect("create");
        // Created for its side effect: a second namesake is what forces the `-2` suffix.
        let _second = store
            .create_franchise("Marvel".into())
            .await
            .expect("create");

        let all = store.franchises().await.expect("list");
        assert_eq!(all[0].slug, "marvel");
        assert_eq!(
            all[1].slug, "marvel-2",
            "a second namesake gets its own URL"
        );

        store
            .rename_franchise(first, "Marvel Cinematic".into())
            .await
            .expect("rename");
        let all = store.franchises().await.expect("list");
        assert_eq!(all[0].name, "Marvel Cinematic");
        assert_eq!(all[0].slug, "marvel", "the URL survives a rename");

        store.close().await.expect("close");
    }

    #[tokio::test]
    async fn deleting_a_franchise_releases_its_titles() {
        let store = open("delete");
        let id = store
            .create_franchise("Marvel".into())
            .await
            .expect("create");
        store
            .assign_franchise(assign(id, 603, "The Matrix"))
            .await
            .expect("assign");

        assert!(store.delete_franchise(id).await.expect("delete"));
        assert!(store.placements().await.expect("placements").is_empty());

        store.close().await.expect("close");
    }

    #[tokio::test]
    async fn cameo_overrides_round_trip_both_ways() {
        let store = open("cameo");
        let flag = Cameo {
            kind: "movie".to_owned(),
            media_id: 603,
            person_id: 6384,
            cameo: true,
        };

        store.set_cameo(flag.clone()).await.expect("set");
        let stored = store.cameos("movie".into(), 603).await.expect("read");
        assert_eq!(stored, vec![flag.clone()]);

        // The other direction is the valuable one: an actor the cutoff wrongly demotes.
        store
            .set_cameo(Cameo {
                cameo: false,
                ..flag
            })
            .await
            .expect("unset");
        let stored = store.cameos("movie".into(), 603).await.expect("read");
        assert!(!stored[0].cameo, "an override can un-flag a cameo");

        assert!(store
            .clear_cameo("movie".into(), 603, 6384)
            .await
            .expect("clear"));
        assert!(store
            .cameos("movie".into(), 603)
            .await
            .expect("read")
            .is_empty());

        store.close().await.expect("close");
    }

    #[tokio::test]
    async fn placements_are_keyed_by_kind_and_id() {
        let store = open("placements");
        let id = store
            .create_franchise("Marvel".into())
            .await
            .expect("create");
        store
            .assign_franchise(assign(id, 603, "The Matrix"))
            .await
            .expect("assign");

        let placements = store.placements().await.expect("placements");
        let placement = placements
            .get(&("movie".to_owned(), 603))
            .expect("the title is placed");

        assert_eq!(placement.slug, "marvel");
        assert_eq!(placement.name, "Marvel");

        store.close().await.expect("close");
    }

    /// A name that slugifies to nothing still needs a URL.
    #[tokio::test]
    async fn a_name_with_no_latin_letters_gets_a_workable_slug() {
        let store = open("slug-fallback");
        store.create_franchise("東京".into()).await.expect("create");

        let all = store.franchises().await.expect("list");
        assert!(!all[0].slug.is_empty(), "slug: {}", all[0].slug);

        store.close().await.expect("close");
    }
}
