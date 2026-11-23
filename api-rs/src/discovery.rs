//! Discovery lists, and the cast synergy table that goes with them.
//!
//! Five lists — top movies, top TV, new films, upcoming, on the air — refreshed every few hours
//! into SQLite so no page waits on TMDB. The refresh is the only thing that pays for the upstream
//! calls; a visitor reads what is already there.
//!
//! The interesting part is the synergy table. Twenty titles makes the pairwise intersection that
//! powers the main comparison useless: 190 pairs, most of them empty. What is meaningful at that
//! scale is an actor who turns up **twice or more** across the whole list — the character actor
//! who is in everything. That is what this computes, once per list snapshot.

use std::collections::HashMap;
use std::num::NonZeroU32;
use std::sync::Arc;

use serde::{Deserialize, Serialize};
use tokio::task::JoinSet;

use crate::domain::{Kind, Selection};
use crate::error::AppError;
use crate::state::AppState;
use crate::tmdb::Tmdb;

/// One discovery list.
pub struct ListSpec {
    /// Stored key: `movie:popular`.
    pub id: &'static str,
    /// URL segment: `/discover/top-movies`.
    pub slug: &'static str,
    pub heading: &'static str,
    pub blurb: &'static str,
    pub kind: Kind,
    /// TMDB path, relative to `/3`.
    pub path: &'static str,
    /// Whether the endpoint takes a `region` (the ones with release dates do).
    pub regional: bool,
}

pub const LISTS: &[ListSpec] = &[
    ListSpec {
        id: "movie:popular",
        slug: "top-movies",
        heading: "Top films",
        blurb: "What everyone is watching, and the actors who keep turning up in it.",
        kind: Kind::Movie,
        path: "movie/popular",
        regional: false,
    },
    ListSpec {
        id: "tv:popular",
        slug: "top-tv",
        heading: "Top shows",
        blurb: "The most-watched series right now, and the faces shared between them.",
        kind: Kind::Tv,
        path: "tv/popular",
        regional: false,
    },
    ListSpec {
        id: "movie:now_playing",
        slug: "new-movies",
        heading: "In cinemas",
        blurb: "What is showing near you this week.",
        kind: Kind::Movie,
        path: "movie/now_playing",
        regional: true,
    },
    ListSpec {
        id: "movie:upcoming",
        slug: "upcoming",
        heading: "Coming soon",
        blurb: "Not out yet. The same actors, in films nobody has seen.",
        kind: Kind::Movie,
        path: "movie/upcoming",
        regional: true,
    },
    ListSpec {
        id: "tv:on_the_air",
        slug: "on-the-air",
        heading: "On the air",
        blurb: "Series with an episode this week.",
        kind: Kind::Tv,
        path: "tv/on_the_air",
        regional: false,
    },
];

pub fn by_slug(slug: &str) -> Option<&'static ListSpec> {
    LISTS.iter().find(|list| list.slug == slug)
}

pub fn by_id(id: &str) -> Option<&'static ListSpec> {
    LISTS.iter().find(|list| list.id == id)
}

/// One title in a list, as stored and returned.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Item {
    pub id: u32,
    pub name: String,
    pub year: Option<u16>,
    pub poster: Option<String>,
    pub overview: Option<String>,
    pub rating: Option<f64>,
}

/// One row of the synergy table.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SynergyRow {
    pub person_id: u32,
    pub name: String,
    /// Portrait, when TMDB has one. A table of names is a lot less scannable than a table of
    /// faces, and the face is what makes "the same actor again" obvious.
    #[serde(default)]
    pub image: Option<String>,
    /// How many of the list's titles they appear in.
    pub count: usize,
    /// The titles, so the row explains itself.
    pub titles: Vec<String>,
}

/// Keep the synergy table to something a person can scan. The tail is people in exactly two
/// titles, which at twenty titles is a long list and not a discovery.
const MAX_SYNERGY_ROWS: usize = 250;

/// One credited person during a refresh.
#[derive(Debug, Clone)]
pub struct Credit {
    pub id: u32,
    pub name: String,
    /// Portrait, when TMDB had one.
    pub image: Option<String>,
}

/// Build the "appears in two or more" table.
///
/// Pure, so it can be tested without TMDB: the fetch is the boring part, this is the part with a
/// decision in it.
pub fn synergy(per_title: &[(String, Vec<Credit>)]) -> Vec<SynergyRow> {
    let mut people: HashMap<u32, (String, Option<String>, Vec<String>)> = HashMap::new();

    for (title, cast) in per_title {
        // One person can be credited twice in the same title (two roles, or a documentary
        // appearance); that is still one title.
        let mut seen_here: HashMap<u32, (&str, Option<String>)> = HashMap::new();
        for credit in cast {
            seen_here
                .entry(credit.id)
                .or_insert_with(|| (credit.name.as_str(), credit.image.clone()));
        }

        for (id, (name, image)) in seen_here {
            let entry = people
                .entry(id)
                .or_insert_with(|| (name.to_owned(), image.clone(), Vec::new()));
            // The first title that had a portrait wins; a later title without one must not
            // blank out a face we already have.
            if entry.1.is_none() {
                entry.1 = image;
            }
            entry.2.push(title.clone());
        }
    }

    let mut rows: Vec<SynergyRow> = people
        .into_iter()
        .filter(|(_, (_, _, titles))| titles.len() >= 2)
        .map(|(person_id, (name, image, titles))| SynergyRow {
            person_id,
            name,
            image,
            count: titles.len(),
            titles,
        })
        .collect();

    // Most-shared first, then alphabetical so the order is stable between refreshes.
    rows.sort_by(|a, b| b.count.cmp(&a.count).then(a.name.cmp(&b.name)));
    rows.truncate(MAX_SYNERGY_ROWS);
    rows
}

/// Fetch one list and its synergy, and store both.
///
/// Errors are returned rather than logged: the caller decides whether a failed list should keep
/// the previous snapshot (it should) or fail the job (it should not).
pub async fn refresh_one(
    tmdb: &Arc<Tmdb>,
    store: &crate::store::Store,
    list: &ListSpec,
    region: &str,
) -> Result<(usize, usize), AppError> {
    let region = list.regional.then_some(region);
    let raw = tmdb.list(list.path, region).await?;

    let items: Vec<Item> = raw
        .into_iter()
        .map(|entry| Item {
            id: entry.id,
            name: entry.display_name(),
            year: entry.year(),
            poster: entry
                .poster_path
                .as_deref()
                .map(|path| tmdb.image_url(path)),
            overview: entry.overview.filter(|text| !text.trim().is_empty()),
            rating: entry.vote_average.filter(|rating| *rating > 0.0),
        })
        .collect();

    if items.is_empty() {
        return Err(AppError::Upstream(format!(
            "list {} came back empty",
            list.id
        )));
    }

    // Cast for every title, concurrently. Twenty calls per list, five lists, every few hours —
    // and they all land in the existing TMDB cache, so a comparison of the same titles is free
    // afterwards.
    let mut tasks = JoinSet::new();
    for item in &items {
        let tmdb = Arc::clone(tmdb);
        let kind = list.kind;
        let id = item.id;
        let name = item.name.clone();

        tasks.spawn(async move {
            // A zero id is not a title; `?` inside the task returns None, which the collector
            // below treats as "no credits for this one".
            let selection = Selection {
                id: NonZeroU32::new(id)?,
                kind,
            };

            match tmdb.title_selection(selection).await {
                Ok((_, cast)) => Some((
                    name,
                    cast.into_iter()
                        .map(|claim| Credit {
                            id: claim.id,
                            name: claim.name,
                            image: claim.image,
                        })
                        .collect::<Vec<_>>(),
                )),
                Err(error) => {
                    // One title without credits is a gap in the table, not a failed list.
                    tracing::debug!(%id, %error, "no credits for a discovery title");
                    None
                }
            }
        });
    }

    let mut per_title: Vec<(String, Vec<Credit>)> = Vec::with_capacity(items.len());
    while let Some(joined) = tasks.join_next().await {
        if let Ok(Some(entry)) = joined {
            per_title.push(entry);
        }
    }

    // Sorted back into list order. A `JoinSet` completes in whatever order the upstream answers,
    // so without this the "which films" column changes order between refreshes — the page looks
    // like it is shuffling itself for no reason.
    per_title.sort_by_key(|(title, _)| {
        items
            .iter()
            .position(|item| &item.name == title)
            .unwrap_or(usize::MAX)
    });

    let rows = synergy(&per_title);
    let title_count = items.len();

    store
        .put_list(
            list.id.to_owned(),
            // A bare array, matching what the read path parses. The wrapper struct this
            // replaced serialised as an object and every page 500'd on the mismatch.
            serde_json::to_string(&items).unwrap_or_else(|_| "[]".into()),
            serde_json::to_string(&rows).unwrap_or_else(|_| "[]".into()),
        )
        .await?;

    tracing::info!(
        list = list.id,
        titles = title_count,
        people = rows.len(),
        "refreshed a discovery list"
    );

    Ok((per_title.len(), rows.len()))
}

/// Refresh every list. Sequential across lists, concurrent within one: five calls in flight at
/// once is already more than TMDB needs us to be polite about.
pub async fn refresh_all(state: &AppState) {
    for list in LISTS {
        match refresh_one(&state.tmdb, &state.store, list, &state.config.tmdb_region).await {
            Ok(_) => {}
            // The previous snapshot stays; a stale list is better than an empty page.
            Err(error) => tracing::warn!(list = list.id, %error, "could not refresh a list"),
        }
    }
}

/// Refresh on a timer, and once shortly after startup so a fresh deploy has content.
pub fn spawn_refresher(state: AppState) {
    tokio::spawn(async move {
        // Let the server finish starting before spending the upstream quota.
        tokio::time::sleep(std::time::Duration::from_secs(20)).await;
        refresh_all(&state).await;

        loop {
            tokio::time::sleep(std::time::Duration::from_secs(6 * 60 * 60)).await;
            refresh_all(&state).await;
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cast(people: &[(u32, &str)]) -> Vec<Credit> {
        people
            .iter()
            .map(|(id, name)| Credit {
                id: *id,
                name: (*name).to_owned(),
                image: None,
            })
            .collect()
    }

    #[test]
    fn only_people_in_two_or_more_titles_are_listed() {
        let rows = synergy(&[
            ("A".into(), cast(&[(1, "Only In A"), (2, "Shared")])),
            ("B".into(), cast(&[(2, "Shared"), (3, "Only In B")])),
        ]);

        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].name, "Shared");
        assert_eq!(rows[0].count, 2);
        assert_eq!(rows[0].titles, ["A", "B"]);
    }

    /// A bit part in one title is not a discovery, however famous the person.
    #[test]
    fn a_single_appearance_is_not_a_finding() {
        let rows = synergy(&[("A".into(), cast(&[(1, "Someone")]))]);
        assert!(rows.is_empty());
    }

    /// Being credited twice in the same title is still one title.
    #[test]
    fn a_double_credit_in_one_title_counts_once() {
        let rows = synergy(&[
            ("A".into(), cast(&[(1, "Twice"), (1, "Twice")])),
            ("B".into(), cast(&[(1, "Twice")])),
        ]);

        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].count, 2, "two titles, not three credits");
    }

    #[test]
    fn rows_are_ordered_by_how_many_titles_then_by_name() {
        let rows = synergy(&[
            (
                "A".into(),
                cast(&[(1, "Three"), (2, "Two B"), (3, "Two A")]),
            ),
            (
                "B".into(),
                cast(&[(1, "Three"), (2, "Two B"), (3, "Two A")]),
            ),
            ("C".into(), cast(&[(1, "Three")])),
        ]);

        assert_eq!(rows[0].name, "Three");
        assert_eq!(rows[0].count, 3);
        assert_eq!(
            rows[1].name, "Two A",
            "ties are alphabetical, so the order is stable"
        );
        assert_eq!(rows[2].name, "Two B");
    }

    /// The titles column follows the order the titles were given in, which is the list's ranking.
    #[test]
    fn titles_inside_a_row_follow_the_list_order() {
        let rows = synergy(&[
            ("First".into(), cast(&[(1, "Someone")])),
            ("Second".into(), cast(&[(1, "Someone")])),
            ("Third".into(), cast(&[(1, "Someone")])),
        ]);

        assert_eq!(rows[0].titles, ["First", "Second", "Third"]);
    }

    #[test]
    fn the_table_is_capped() {
        // 300 people, each in two of three titles.
        let cast_all: Vec<Credit> = (0..300)
            .map(|id| Credit {
                id,
                name: format!("Person {id}"),
                image: None,
            })
            .collect();
        let rows = synergy(&[("A".into(), cast_all.clone()), ("B".into(), cast_all)]);

        assert_eq!(rows.len(), MAX_SYNERGY_ROWS);
    }

    #[test]
    fn every_slug_and_id_is_unique_and_resolvable() {
        for list in LISTS {
            assert!(
                by_slug(list.slug).is_some(),
                "{} is not resolvable",
                list.slug
            );
            assert!(by_id(list.id).is_some(), "{} is not resolvable", list.id);
        }

        let mut slugs: Vec<&str> = LISTS.iter().map(|list| list.slug).collect();
        slugs.sort_unstable();
        let unique = slugs.len();
        slugs.dedup();
        assert_eq!(slugs.len(), unique, "a slug is used twice");
    }

    #[test]
    fn only_the_lists_with_dates_are_regional() {
        for list in LISTS {
            let expected = list.slug == "new-movies" || list.slug == "upcoming";
            assert_eq!(
                list.regional, expected,
                "{} regional flag is wrong",
                list.slug
            );
        }
    }

    #[test]
    fn an_unknown_slug_is_not_a_page() {
        assert!(by_slug("best-anime").is_none());
    }
}
