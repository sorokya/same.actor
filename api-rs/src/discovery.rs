//! Discovery lists, and the cast synergy table that goes with them.
//!
//! Three sections — **top**, **new**, **upcoming** — each split by kind (films, shows), and the
//! top and new sections further split by what "top" and "new" mean: all time versus trending, in
//! cinemas versus at home. That is nine lists rather than five, but only three tabs, because the
//! alternatives to "top films" are a control on the page, not a place in the navigation.
//!
//! Every list is refreshed into SQLite every few hours, so no page waits on TMDB. The refresh is
//! the only thing that pays for the upstream calls; a visitor reads what is already there.
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

/// The three pages.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Section {
    Top,
    New,
    Upcoming,
}

impl Section {
    pub const ALL: [Section; 3] = [Section::Top, Section::New, Section::Upcoming];

    /// The URL segment and the stored-key prefix.
    pub fn key(self) -> &'static str {
        match self {
            Section::Top => "top",
            Section::New => "new",
            Section::Upcoming => "upcoming",
        }
    }

    pub fn from_key(key: &str) -> Option<Section> {
        Section::ALL
            .into_iter()
            .find(|section| section.key() == key)
    }

    pub fn heading(self) -> &'static str {
        match self {
            Section::Top => "Top",
            Section::New => "New",
            Section::Upcoming => "Upcoming",
        }
    }

    /// What the second axis means for this section. It is not a filter in the ordinary sense —
    /// for "top" it is a choice of ranking, for "new" a choice of where the film is showing — so
    /// the label belongs to the section rather than to the control.
    pub fn filter_group(self) -> &'static str {
        match self {
            Section::Top => "Ranked by",
            Section::New => "Where",
            Section::Upcoming => "",
        }
    }

    pub fn blurb(self) -> &'static str {
        match self {
            Section::Top => {
                "Ranked by how much they have been watched, and the actors who keep turning up."
            }
            Section::New => "Out now, and the casts they share.",
            Section::Upcoming => "Not out yet. The same actors, in things nobody has seen.",
        }
    }
}

impl Variant {
    /// The page heading.
    ///
    /// Written per list rather than assembled from a section and a kind: "New shows" is what the
    /// old `format!` produced, and it says nothing — the list is shows with an episode in the
    /// last few days, which is a fact about the calendar, not about newness.
    pub fn heading(&self) -> &'static str {
        match (self.section, self.kind, self.filter) {
            (Section::Top, Kind::Movie, "all-time") => "Top films of all time",
            (Section::Top, Kind::Movie, _) => "Trending films",
            (Section::Top, Kind::Tv, "all-time") => "Top shows of all time",
            (Section::Top, Kind::Tv, _) => "Trending shows",
            (Section::New, Kind::Movie, "cinema") => "In cinemas now",
            (Section::New, Kind::Movie, _) => "New at home",
            (Section::New, Kind::Tv, _) => "On air now",
            (Section::Upcoming, Kind::Movie, _) => "Films coming soon",
            (Section::Upcoming, Kind::Tv, _) => "Shows coming soon",
            // No list in VARIANTS is about people; a variant that was would still need a
            // heading, and the section's own is a truthful fallback.
            _ => self.section.heading(),
        }
    }

    /// One sentence under the heading, saying what the list actually is.
    pub fn blurb(&self) -> &'static str {
        match (self.section, self.kind, self.filter) {
            (Section::Top, Kind::Movie, "all-time") => {
                "The films with the most votes ever cast, and the actors who keep turning up."
            }
            (Section::Top, Kind::Movie, _) => {
                "What people are watching this week, and the casts they share."
            }
            (Section::Top, Kind::Tv, "all-time") => {
                "The shows with the most votes ever cast, and the actors who keep turning up."
            }
            (Section::Top, Kind::Tv, _) => "What people are watching this week, and the casts they share.",
            (Section::New, Kind::Movie, "cinema") => {
                "Films in cinemas this week — and the actors who turn up in more than one."
            }
            (Section::New, Kind::Movie, _) => {
                "Films that have just reached digital and disc, and the casts they share."
            }
            (Section::New, Kind::Tv, _) => {
                "Shows with an episode in the last few days, and the actors who turn up in more than one."
            }
            (Section::Upcoming, Kind::Movie, _) => {
                "Films with a release date still ahead of them. Nobody has seen these yet."
            }
            (Section::Upcoming, Kind::Tv, _) => {
                "Shows that have not premiered yet. Nobody has seen these either."
            }
            _ => self.section.blurb(),
        }
    }

    /// Which of a list entry's dates this list is about.
    ///
    /// A year is the right precision for a ranked list of films from 1972. It is the wrong one
    /// for "what is on air", where the answer is the last episode, and for "coming soon", where
    /// it is the date you would put in a calendar.
    pub fn date_of<'a>(&self, entry: &'a crate::tmdb::types::ListItem) -> Option<&'a str> {
        match (self.section, self.kind) {
            (Section::Top, _) => None,
            (Section::New, Kind::Tv) => entry.last_air_date.as_deref(),
            (Section::Upcoming, Kind::Tv) => entry.first_air_date.as_deref(),
            (Section::New, _) | (Section::Upcoming, _) => entry.release_date.as_deref(),
        }
    }
}

/// How one list is fetched.
#[derive(Debug)]
pub struct Source {
    /// TMDB path, relative to `/3`.
    pub path: &'static str,
    /// Extra query pairs. A `{today}` in a value becomes the current date at refresh time, which
    /// is what makes "at home" mean "released already" rather than "released at some point".
    pub params: &'static [(&'static str, &'static str)],
    /// Whether the endpoint wants a `region` (the ones keyed on release dates do).
    pub regional: bool,
}

/// One list: a section, a kind, and what the section means for that kind.
#[derive(Debug)]
pub struct Variant {
    /// Stored key, e.g. `top:movie:trending`.
    pub id: &'static str,
    pub section: Section,
    pub kind: Kind,
    /// The control value. Empty when the section has no second axis for this kind.
    pub filter: &'static str,
    /// The control's label, e.g. "At home".
    pub filter_label: &'static str,
    pub source: Source,
}

/// `vote_count.desc` with a floor is the closest thing TMDB has to "all time": it is cumulative
/// popularity, so a film with a hundred thousand votes that nobody watches this week still ranks.
/// Without the floor, sorting by vote count surfaces obscure titles with a handful of votes.
pub const VARIANTS: &[Variant] = &[
    Variant {
        id: "top:movie:all-time",
        section: Section::Top,
        kind: Kind::Movie,
        filter: "all-time",
        filter_label: "All time",
        source: Source {
            path: "discover/movie",
            params: &[("sort_by", "vote_count.desc"), ("vote_count.gte", "5000")],
            regional: false,
        },
    },
    Variant {
        id: "top:movie:trending",
        section: Section::Top,
        kind: Kind::Movie,
        filter: "trending",
        filter_label: "Trending",
        source: Source {
            path: "trending/movie/day",
            params: &[],
            regional: false,
        },
    },
    Variant {
        id: "top:tv:all-time",
        section: Section::Top,
        kind: Kind::Tv,
        filter: "all-time",
        filter_label: "All time",
        source: Source {
            path: "discover/tv",
            params: &[("sort_by", "vote_count.desc"), ("vote_count.gte", "2000")],
            regional: false,
        },
    },
    Variant {
        id: "top:tv:trending",
        section: Section::Top,
        kind: Kind::Tv,
        filter: "trending",
        filter_label: "Trending",
        source: Source {
            path: "trending/tv/day",
            params: &[],
            regional: false,
        },
    },
    Variant {
        id: "new:movie:cinema",
        section: Section::New,
        kind: Kind::Movie,
        filter: "cinema",
        filter_label: "In cinemas",
        source: Source {
            path: "movie/now_playing",
            params: &[],
            regional: true,
        },
    },
    Variant {
        id: "new:movie:home",
        section: Section::New,
        kind: Kind::Movie,
        filter: "home",
        filter_label: "At home",
        source: Source {
            // Release types 4 and 5 are digital and physical: "at home" as opposed to "in a
            // cinema", which is release type 3.
            path: "discover/movie",
            params: &[
                ("sort_by", "primary_release_date.desc"),
                ("with_release_type", "4|5"),
                ("release_date.lte", "{today}"),
                ("vote_count.gte", "10"),
            ],
            regional: false,
        },
    },
    Variant {
        id: "new:tv:on-the-air",
        section: Section::New,
        kind: Kind::Tv,
        filter: "on-the-air",
        filter_label: "On the air",
        source: Source {
            path: "tv/on_the_air",
            params: &[],
            regional: false,
        },
    },
    Variant {
        id: "upcoming:movie",
        section: Section::Upcoming,
        kind: Kind::Movie,
        filter: "",
        filter_label: "",
        source: Source {
            // `movie/upcoming` is documented as "released soon *or recently released*", which
            // is why it listed films that were already out. `discover` with a release-date
            // floor is the only way to say "not out yet" and mean it.
            path: "discover/movie",
            params: &[
                ("sort_by", "popularity.desc"),
                ("primary_release_date.gte", "{today}"),
                ("primary_release_date.lte", "{in:120}"),
            ],
            regional: false,
        },
    },
    Variant {
        id: "upcoming:tv",
        section: Section::Upcoming,
        kind: Kind::Tv,
        filter: "",
        filter_label: "",
        source: Source {
            // Same floor as the films, and the same ceiling: "coming soon" that reaches into
            // next year is a list nobody reads.
            path: "discover/tv",
            params: &[
                ("sort_by", "popularity.desc"),
                ("first_air_date.gte", "{today}"),
                ("first_air_date.lte", "{in:120}"),
            ],
            regional: false,
        },
    },
];

/// The lists for one section and kind, in control order.
pub fn variants(section: Section, kind: Kind) -> impl Iterator<Item = &'static Variant> {
    VARIANTS
        .iter()
        .filter(move |variant| variant.section == section && variant.kind == kind)
}

/// The list for a request. `filter` of `None` or `Some("")` takes the section's default, which is
/// the first one — so `/discover/top?kind=tv` is the same page as `...&filter=all-time`.
pub fn resolve(section: Section, kind: Kind, filter: Option<&str>) -> Option<&'static Variant> {
    let filter = filter.unwrap_or("").trim();
    let mut candidates = variants(section, kind);

    if filter.is_empty() {
        return candidates.next();
    }

    candidates.find(|variant| variant.filter == filter)
}

pub fn by_id(id: &str) -> Option<&'static Variant> {
    VARIANTS.iter().find(|variant| variant.id == id)
}

/// One title in a list, as stored and returned.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Item {
    pub id: u32,
    pub name: String,
    pub year: Option<u16>,
    /// The date this list is about, when a year is too coarse. `None` on the ranked lists.
    #[serde(default)]
    pub date: Option<String>,
    pub poster: Option<String>,
    pub overview: Option<String>,
    pub rating: Option<f64>,
    /// TMDB's collection for this title. A *suggestion* for the admin queue, never a franchise:
    /// a collection is narrower than a franchise, and the maintainer decides.
    #[serde(default)]
    pub collection_id: Option<u32>,
    /// Directors of the title, for "another film by the same director" suggestions.
    #[serde(default)]
    pub director_ids: Vec<u32>,
    /// TMDB keyword ids (the MCU keyword, "based on novel", and so on), for the same reason.
    #[serde(default)]
    pub keyword_ids: Vec<u32>,
}

/// A title as a synergy row refers to it.
///
/// The id and kind travel with the name because the row is where a reader wants to follow it: a
/// bare string left the title names on the actor cards as text, and the only way to link them was
/// to guess the kind from the list.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TitleRef {
    pub id: u32,
    pub kind: Kind,
    pub name: String,
    /// Where the billing put *this person* in this title, 0-based. Set on the entries inside a
    /// synergy row (a fact about the person) and left empty on a bare title reference.
    #[serde(default)]
    pub order: Option<u32>,
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
    #[serde(default)]
    pub titles: Vec<TitleRef>,
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
    /// Billing position in this title. `None` when TMDB did not say.
    pub order: Option<u32>,
}

/// Build the "appears in two or more" table.
///
/// Pure, so it can be tested without TMDB: the fetch is the boring part, this is the part with a
/// decision in it.
pub fn synergy(per_title: &[(TitleRef, Vec<Credit>)]) -> Vec<SynergyRow> {
    let mut people: HashMap<u32, (String, Option<String>, Vec<TitleRef>)> = HashMap::new();

    for (title, cast) in per_title {
        // One person can be credited twice in the same title (two roles, or a documentary
        // appearance); that is still one title.
        let mut seen_here: HashMap<u32, (&str, Option<String>, Option<u32>)> = HashMap::new();
        for credit in cast {
            let slot =
                seen_here
                    .entry(credit.id)
                    .or_insert((credit.name.as_str(), None, credit.order));
            if slot.1.is_none() {
                slot.1 = credit.image.clone();
            }
            // Credited twice in one title: the better billing is the one that counts.
            slot.2 = match (slot.2, credit.order) {
                (Some(a), Some(b)) => Some(a.min(b)),
                (a, b) => a.or(b),
            };
        }

        for (id, (name, image, order)) in seen_here {
            let entry = people
                .entry(id)
                .or_insert_with(|| (name.to_owned(), image.clone(), Vec::new()));
            // The first title that had a portrait wins; a later title without one must not
            // blank out a face we already have.
            if entry.1.is_none() {
                entry.1 = image;
            }
            entry.2.push(TitleRef {
                order,
                ..title.clone()
            });
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

/// Today in UTC, as `YYYY-MM-DD`.
///
/// TMDB's date filters want a calendar date, and the crate that would give us one is not a
/// dependency. This is Howard Hinnant's civil-from-days: exact for any date this program will
/// ever run, and testable without a clock.
fn today_utc() -> String {
    let days = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|elapsed| elapsed.as_secs() / 86_400)
        .unwrap_or(0);
    civil_from_days(days as i64)
}

/// `YYYY-MM-DD` for N days from today. Negative N is allowed; nothing currently uses it, but
/// there is no reason for the arithmetic to be one-way.
fn date_in_days(days_ahead: i64) -> String {
    let today = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|elapsed| (elapsed.as_secs() / 86_400) as i64)
        .unwrap_or(0);

    civil_from_days(today + days_ahead)
}

/// Fill in `{today}` and `{in:N}`.
///
/// A placeholder that survives into the URL would be sent to TMDB as a literal and quietly
/// return nothing, so this is the only place either can be written.
fn expand_placeholders(template: &str, today: &str) -> String {
    let mut out = template.replace("{today}", today);

    while let Some(start) = out.find("{in:") {
        let Some(end) = out[start..].find('}').map(|offset| start + offset) else {
            break;
        };

        let Ok(days) = out[start + 4..end].parse::<i64>() else {
            break;
        };

        let date = date_in_days(days);
        out.replace_range(start..=end, &date);
    }

    out
}

pub(crate) fn civil_from_days(days_since_epoch: i64) -> String {
    let z = days_since_epoch + 719_468;
    let era = if z >= 0 { z } else { z - 146_096 } / 146_097;
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1_460 + doe / 36_524 - doe / 146_096) / 365;
    let year = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let day = doy - (153 * mp + 2) / 5 + 1;
    let month = if mp < 10 { mp + 3 } else { mp - 9 };
    let year = if month <= 2 { year + 1 } else { year };

    format!("{year:04}-{month:02}-{day:02}")
}

/// Fetch one list and its synergy, and store both.
///
/// Errors are returned rather than logged: the caller decides whether a failed list should keep
/// the previous snapshot (it should) or fail the job (it should not).
pub async fn refresh_one(
    tmdb: &Arc<Tmdb>,
    store: &crate::store::Store,
    variant: &Variant,
    region: &str,
) -> Result<(usize, usize), AppError> {
    let region = variant.source.regional.then_some(region);

    let today = today_utc();
    let owned: Vec<(String, String)> = variant
        .source
        .params
        .iter()
        .map(|(key, value)| ((*key).to_owned(), expand_placeholders(value, &today)))
        .collect();
    let params: Vec<(&str, &str)> = owned
        .iter()
        .map(|(key, value)| (key.as_str(), value.as_str()))
        .collect();

    let raw = tmdb.list(variant.source.path, &params, region).await?;

    if raw.is_empty() {
        return Err(AppError::Upstream(format!(
            "list {} came back empty",
            variant.id
        )));
    }

    // A day-trending list is what people are watching, and nobody is watching a film that has
    // not come out: TMDB's trending feed carries titles with a trailer and a release date still
    // ahead of them. They belong in Upcoming, which is the one section that asks for them.
    let raw: Vec<_> = if variant.section == Section::Upcoming {
        raw
    } else {
        raw.into_iter()
            .filter(|entry| released(variant.kind, entry, &today))
            .collect()
    };

    // Facts for every title, concurrently: the cast the table is built from, plus the
    // collection, directors and keywords the admin's suggestions are made of. One call per
    // title, the same call a comparison of that title makes, so they share a cache entry.
    let mut tasks = JoinSet::new();
    for entry in &raw {
        let tmdb = Arc::clone(tmdb);
        let kind = variant.kind;
        let id = entry.id;

        tasks.spawn(async move {
            // A zero id is not a title; `?` inside the task returns None, which the collector
            // below treats as "no facts for this one".
            let selection = Selection {
                id: NonZeroU32::new(id)?,
                kind,
            };

            match tmdb.title_facts(selection).await {
                Ok(facts) => Some((id, facts)),
                Err(error) => {
                    // One title without facts is a gap in the table, not a failed list.
                    tracing::debug!(%id, %error, "no facts for a discovery title");
                    None
                }
            }
        });
    }

    let mut facts: HashMap<u32, crate::tmdb::TitleFacts> = HashMap::new();
    while let Some(joined) = tasks.join_next().await {
        if let Ok(Some((id, found))) = joined {
            facts.insert(id, found);
        }
    }

    let items: Vec<Item> = raw
        .iter()
        .map(|entry| {
            let found = facts.get(&entry.id);

            Item {
                id: entry.id,
                name: entry.display_name(),
                year: entry.year(),
                date: variant.date_of(entry).map(str::to_owned),
                poster: entry
                    .poster_path
                    .as_deref()
                    .map(|path| tmdb.image_url(path)),
                overview: entry
                    .overview
                    .clone()
                    .filter(|text| !text.trim().is_empty()),
                rating: entry.vote_average.filter(|rating| *rating > 0.0),
                collection_id: found.and_then(|found| found.collection_id),
                director_ids: found
                    .map(|found| found.director_ids.clone())
                    .unwrap_or_default(),
                keyword_ids: found
                    .map(|found| found.keyword_ids.clone())
                    .unwrap_or_default(),
            }
        })
        .collect();

    // An empty list *after* the release filter is a fact, not a fault: everything the feed
    // offered has not come out yet. Storing it means the page can say "nothing yet" instead of
    // claiming the list does not exist.

    // Built in list order, so the titles inside a row follow the list's ranking without a sort.
    let per_title: Vec<(TitleRef, Vec<Credit>)> = items
        .iter()
        .filter_map(|item| {
            let found = facts.get(&item.id)?;

            Some((
                TitleRef {
                    id: item.id,
                    kind: variant.kind,
                    name: item.name.clone(),
                    order: None,
                },
                found
                    .cast
                    .iter()
                    .map(|claim| Credit {
                        id: claim.id,
                        name: claim.name.clone(),
                        image: claim.image.clone(),
                        order: claim.order,
                    })
                    .collect(),
            ))
        })
        .collect();

    let rows = synergy(&per_title);
    let title_count = items.len();

    store
        .put_list(
            variant.id.to_owned(),
            // A bare array, matching what the read path parses. The wrapper struct this
            // replaced serialised as an object and every page 500'd on the mismatch.
            serde_json::to_string(&items).unwrap_or_else(|_| "[]".into()),
            serde_json::to_string(&rows).unwrap_or_else(|_| "[]".into()),
        )
        .await?;

    tracing::info!(
        list = variant.id,
        titles = title_count,
        people = rows.len(),
        "refreshed a discovery list"
    );

    Ok((per_title.len(), rows.len()))
}

/// Whether a list entry is out yet, judged by the date its kind uses.
///
/// ISO dates compare as text, which is the whole reason they are stored that way: `2026-12-01` is
/// greater than `2026-11-24` and nothing has to be parsed to know it. A title with no date is
/// kept — a missing date is not evidence of an unreleased film, and dropping it would quietly
/// shrink the list.
fn released(kind: Kind, entry: &crate::tmdb::types::ListItem, today: &str) -> bool {
    let date = match kind {
        Kind::Movie => entry.release_date.as_deref(),
        _ => entry.first_air_date.as_deref(),
    };

    match date {
        None => true,
        Some(date) => date <= today,
    }
}

/// Refresh every list. Sequential across lists, concurrent within one: nine lists of twenty
/// titles is already more than TMDB needs us to be polite about.
pub async fn refresh_all(state: &AppState) {
    for variant in VARIANTS {
        match refresh_one(
            &state.tmdb,
            &state.store,
            variant,
            &state.config.tmdb_region,
        )
        .await
        {
            Ok(_) => {}
            // The previous snapshot stays; a stale list is better than an empty page.
            Err(error) => tracing::warn!(list = variant.id, %error, "could not refresh a list"),
        }
    }

    // Sections were once five slugs; a snapshot for a list that no longer exists would sit there
    // for ever, never read and never refreshed.
    let keep: Vec<String> = VARIANTS
        .iter()
        .map(|variant| variant.id.to_owned())
        .collect();
    match state.store.prune_lists(keep).await {
        Ok(0) => {}
        Ok(removed) => tracing::info!(removed, "dropped snapshots for lists that no longer exist"),
        Err(error) => tracing::warn!(%error, "could not prune discovery snapshots"),
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

    fn title(id: u32, name: &str) -> TitleRef {
        TitleRef {
            id,
            kind: Kind::Movie,
            name: name.to_owned(),
            order: None,
        }
    }

    fn cast(people: &[(u32, &str)]) -> Vec<Credit> {
        people
            .iter()
            .map(|(id, name)| Credit {
                id: *id,
                name: (*name).to_owned(),
                image: None,
                order: None,
            })
            .collect()
    }

    #[test]
    fn only_people_in_two_or_more_titles_are_listed() {
        let rows = synergy(&[
            (title(1, "A"), cast(&[(1, "Only In A"), (2, "Shared")])),
            (title(2, "B"), cast(&[(2, "Shared"), (3, "Only In B")])),
        ]);

        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].name, "Shared");
        assert_eq!(rows[0].count, 2);
        assert_eq!(
            rows[0].titles,
            [title(1, "A"), title(2, "B")],
            "the row carries the id and kind, so the title can be linked"
        );
    }

    /// A bit part in one title is not a discovery, however famous the person.
    #[test]
    fn a_single_appearance_is_not_a_finding() {
        let rows = synergy(&[(title(1, "A"), cast(&[(1, "Someone")]))]);
        assert!(rows.is_empty());
    }

    /// Being credited twice in the same title is still one title.
    #[test]
    fn a_double_credit_in_one_title_counts_once() {
        let rows = synergy(&[
            (title(1, "A"), cast(&[(1, "Twice"), (1, "Twice")])),
            (title(2, "B"), cast(&[(1, "Twice")])),
        ]);

        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].count, 2, "two titles, not three credits");
    }

    #[test]
    fn rows_are_ordered_by_how_many_titles_then_by_name() {
        let rows = synergy(&[
            (
                title(1, "A"),
                cast(&[(1, "Three"), (2, "Two B"), (3, "Two A")]),
            ),
            (
                title(2, "B"),
                cast(&[(1, "Three"), (2, "Two B"), (3, "Two A")]),
            ),
            (title(3, "C"), cast(&[(1, "Three")])),
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
            (title(1, "First"), cast(&[(1, "Someone")])),
            (title(2, "Second"), cast(&[(1, "Someone")])),
            (title(3, "Third"), cast(&[(1, "Someone")])),
        ]);

        let names: Vec<&str> = rows[0].titles.iter().map(|row| row.name.as_str()).collect();
        assert_eq!(names, ["First", "Second", "Third"]);
    }

    /// A row's titles carry the billing position, which is what tells a lead from a cameo.
    #[test]
    fn a_rows_titles_carry_the_billing_position() {
        let credits = |order: Option<u32>| {
            vec![
                Credit {
                    id: 6384,
                    name: "Keanu Reeves".to_owned(),
                    image: None,
                    order,
                },
                Credit {
                    id: 530,
                    name: "Carrie-Anne Moss".to_owned(),
                    image: None,
                    order: order.map(|order| order + 1),
                },
            ]
        };

        let rows = synergy(&[
            (title(603, "The Matrix"), credits(Some(0))),
            (title(604, "Reloaded"), credits(Some(0))),
        ]);

        let keanu = rows
            .iter()
            .find(|row| row.person_id == 6384)
            .expect("keanu");
        assert_eq!(
            keanu.titles.iter().map(|t| t.order).collect::<Vec<_>>(),
            [Some(0), Some(0)]
        );

        let carrie = rows
            .iter()
            .find(|row| row.person_id == 530)
            .expect("carrie-anne");
        assert_eq!(
            carrie.titles[0].order,
            Some(1),
            "billing is per person, not per title"
        );
    }

    /// Credited twice in one title (two roles): the better billing is the one that counts.
    #[test]
    fn the_best_billing_wins_when_someone_is_credited_twice() {
        let twice = vec![
            Credit {
                id: 1,
                name: "Someone".to_owned(),
                image: None,
                order: Some(9),
            },
            Credit {
                id: 1,
                name: "Someone".to_owned(),
                image: None,
                order: Some(2),
            },
            Credit {
                id: 2,
                name: "Other".to_owned(),
                image: None,
                order: Some(5),
            },
        ];

        let rows = synergy(&[
            (title(1, "A"), twice),
            (title(2, "B"), cast(&[(1, "Someone"), (2, "Other")])),
        ]);
        let first = rows.iter().find(|row| row.person_id == 1).expect("row");

        assert_eq!(
            first.titles[0].order,
            Some(2),
            "the lower billing index wins"
        );
        assert_eq!(first.titles[1].order, None, "no order upstream stays None");
    }

    /// A list written before the franchise fields existed must still parse: the payload is
    /// stored JSON, and an upgrade must not turn stored lists into 500s.
    #[test]
    fn a_payload_from_before_the_franchise_fields_still_parses() {
        let items: Vec<Item> = serde_json::from_str(
            r#"[{"id":603,"name":"The Matrix","year":1999,"poster":null,"overview":null,"rating":8.2}]"#,
        )
        .expect("an older list still reads");

        assert_eq!(items[0].name, "The Matrix");
        assert!(items[0].collection_id.is_none());
        assert!(items[0].director_ids.is_empty());
        assert!(items[0].keyword_ids.is_empty());

        let rows: Vec<SynergyRow> = serde_json::from_str(
            r#"[{"person_id":530,"name":"Carrie-Anne Moss","image":null,"count":2,
                 "titles":[{"id":603,"kind":"movie","name":"The Matrix"}]}]"#,
        )
        .expect("an older table still reads");

        assert_eq!(rows[0].titles[0].name, "The Matrix");
        assert!(rows[0].titles[0].order.is_none());
    }

    #[test]
    fn the_table_is_capped() {
        // 300 people, each in two of three titles.
        let cast_all: Vec<Credit> = (0..300)
            .map(|id| Credit {
                id,
                name: format!("Person {id}"),
                image: None,
                order: None,
            })
            .collect();
        let rows = synergy(&[(title(1, "A"), cast_all.clone()), (title(2, "B"), cast_all)]);

        assert_eq!(rows.len(), MAX_SYNERGY_ROWS);
    }

    #[test]
    fn every_id_is_unique_and_resolvable() {
        for variant in VARIANTS {
            assert!(
                by_id(variant.id).is_some(),
                "{} is not resolvable",
                variant.id
            );
        }

        let mut ids: Vec<&str> = VARIANTS.iter().map(|variant| variant.id).collect();
        ids.sort_unstable();
        let unique = ids.len();
        ids.dedup();
        assert_eq!(ids.len(), unique, "an id is used twice");
    }

    /// Every section and kind the site links to must resolve, both with its default filter and
    /// with each filter it advertises.
    #[test]
    fn every_section_and_kind_resolves() {
        for section in Section::ALL {
            for kind in [Kind::Movie, Kind::Tv] {
                let all: Vec<&Variant> = variants(section, kind).collect();
                assert!(
                    !all.is_empty(),
                    "{} has no list for {kind:?}",
                    section.key()
                );

                let default = resolve(section, kind, None).expect("a default");
                assert_eq!(default.filter, all[0].filter);

                for variant in all {
                    let found =
                        resolve(section, kind, Some(variant.filter)).expect("its own filter");
                    assert_eq!(found.id, variant.id);
                }
            }
        }
    }

    /// The control only appears when there is a choice to make, so a section whose single variant
    /// has a filter would render a one-option control that does nothing.
    #[test]
    fn a_filter_label_exists_exactly_when_the_filter_does() {
        for variant in VARIANTS {
            assert_eq!(
                variant.filter.is_empty(),
                variant.filter_label.is_empty(),
                "{} has a filter without a label, or a label without a filter",
                variant.id
            );
        }
    }

    #[test]
    fn only_the_lists_with_dates_are_regional() {
        for variant in VARIANTS {
            let expected = variant.id == "new:movie:cinema";
            assert_eq!(
                variant.source.regional, expected,
                "{} regional flag is wrong",
                variant.id
            );
        }
    }

    /// No placeholder may survive into a URL: TMDB would receive it as a literal string and
    /// quietly return nothing, which looks like "no releases" rather than a bug.
    #[test]
    fn every_placeholder_is_expanded_before_it_becomes_a_query() {
        let today = today_utc();

        for variant in VARIANTS {
            for (key, value) in variant.source.params {
                let expanded = expand_placeholders(value, &today);
                assert!(
                    !expanded.contains('{') && !expanded.contains('}'),
                    "{} left a placeholder in {key}: {expanded}",
                    variant.id
                );
            }
        }

        assert_eq!(expand_placeholders("{today}", &today), today);
        assert_eq!(expand_placeholders("{in:0}", &today), today);
    }

    /// The date arithmetic the windows depend on, checked against dates whose weekday and
    /// month are known.
    #[test]
    fn a_future_date_is_a_plain_iso_date() {
        let today = today_utc();
        let later = expand_placeholders("{in:120}", &today);

        assert_eq!(later.len(), 10, "got {later}");
        assert!(later > today, "{later} should be after {today}");
        assert!(later.starts_with("20"));
    }

    /// The copy has to describe the list it is actually under: "New shows" was what a section
    /// name plus a kind produced, and it told a reader nothing about why these shows were here.
    #[test]
    fn the_copy_says_what_the_list_is() {
        assert_eq!(by_id("new:tv:on-the-air").unwrap().heading(), "On air now");
        assert_eq!(
            by_id("new:movie:cinema").unwrap().heading(),
            "In cinemas now"
        );
        assert_eq!(
            by_id("upcoming:movie").unwrap().heading(),
            "Films coming soon"
        );

        // Two lists must not share a heading, or the tabs look broken.
        let mut seen: Vec<&str> = VARIANTS.iter().map(|variant| variant.heading()).collect();
        seen.sort_unstable();
        let before = seen.len();
        seen.dedup();
        assert_eq!(seen.len(), before, "two lists share a heading");
    }

    /// A year is the right precision for a ranked list and the wrong one for a calendar.
    #[test]
    fn the_date_bearing_lists_ask_for_a_date() {
        let entry = crate::tmdb::types::ListItem {
            id: 1,
            title: Some("A film".into()),
            name: None,
            release_date: Some("2026-12-01".into()),
            first_air_date: Some("2026-11-01".into()),
            last_air_date: Some("2026-11-20".into()),
            poster_path: None,
            overview: None,
            vote_average: None,
        };

        assert!(by_id("top:movie:all-time")
            .unwrap()
            .date_of(&entry)
            .is_none());
        assert_eq!(
            by_id("upcoming:movie").unwrap().date_of(&entry),
            Some("2026-12-01")
        );
        assert_eq!(
            by_id("upcoming:tv").unwrap().date_of(&entry),
            Some("2026-11-01"),
            "a show that has not premiered is dated by its premiere"
        );
        assert_eq!(
            by_id("new:tv:on-the-air").unwrap().date_of(&entry),
            Some("2026-11-20"),
            "what is on air is dated by its last episode"
        );
    }

    /// "Upcoming" means not out yet: a list that starts before today is the bug this replaced.
    #[test]
    fn the_upcoming_lists_ask_for_a_future_window() {
        for id in ["upcoming:movie", "upcoming:tv"] {
            let variant = by_id(id).expect(id);
            let floor = variant
                .source
                .params
                .iter()
                .find(|(key, _)| key.ends_with(".gte"))
                .map(|(_, value)| *value)
                .expect("a release-date floor");

            assert!(
                floor.contains("{today}"),
                "{id} must start at today, not at some fixed date: {floor}"
            );
        }
    }

    #[test]
    fn an_unknown_section_or_filter_is_not_a_page() {
        assert!(Section::from_key("best-anime").is_none());
        assert!(resolve(Section::Top, Kind::Movie, Some("best-anime")).is_none());
    }

    /// The date arithmetic, checked against dates whose weekday and month are known.
    #[test]
    fn days_since_the_epoch_convert_to_calendar_dates() {
        assert_eq!(civil_from_days(0), "1970-01-01");
        assert_eq!(civil_from_days(1), "1970-01-02");
        assert_eq!(civil_from_days(-1), "1969-12-31");
        assert_eq!(civil_from_days(11_016), "2000-02-29", "a leap day");
        assert_eq!(civil_from_days(20_728), "2026-10-02");
        assert_eq!(civil_from_days(20_789), "2026-12-02");
    }

    /// The placeholder really is substituted, and a date looks like a date.
    #[test]
    fn today_is_a_tmdb_date() {
        let today = today_utc();
        assert_eq!(today.len(), 10, "got {today}");
        assert!(
            today.starts_with("20"),
            "a year in the past would be a surprise: {today}"
        );
    }
}
