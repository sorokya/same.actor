//! `/discover/{section}` — the page, and the JSON under `/api/discover`.
//!
//! Both read the same snapshot through [`load`], so the page a person sees and the JSON a script
//! sees cannot disagree about what is in a list. Neither calls TMDB: the refresh job owns that,
//! and a page that waits on an upstream call is a page that goes down when TMDB does.

use axum::extract::{Path, State};
use axum::response::{IntoResponse, Response};
use axum::Json;
use serde::{Deserialize, Serialize};

use crate::discovery::{self, Item, Section, SynergyRow, Variant};
use crate::domain::Kind;
use crate::error::AppError;
use crate::extract::AppQuery;
use crate::state::AppState;
use crate::views::discover::{DiscoverPage, Snapshot};
use crate::views::{render, Head};

/// One section's snapshot, already parsed.
pub struct Loaded {
    pub section: Section,
    pub kind: Kind,
    pub variant: &'static Variant,
    pub refreshed_ts: i64,
    pub items: Vec<Item>,
    pub synergy: Vec<SynergyRow>,
}

#[derive(Debug, Deserialize)]
pub struct PageQuery {
    pub kind: Option<String>,
    pub filter: Option<String>,
    /// Franchise slugs the reader switched off, comma separated.
    pub hide: Option<String>,
    /// `0` when cameos are hidden. Absent means they are shown, which is the default.
    pub cameos: Option<String>,
    /// A pinned actor, for the side panel and the row highlight.
    pub actor: Option<u32>,
    /// A pinned title, the other half of the same idea.
    pub title: Option<u32>,
}

fn kind_key(kind: Kind) -> &'static str {
    match kind {
        Kind::Movie => "movie",
        Kind::Tv => "tv",
        Kind::Person => "person",
    }
}

fn parse_kind(raw: Option<&str>) -> Result<Kind, AppError> {
    match raw.unwrap_or("").trim() {
        "" | "movie" => Ok(Kind::Movie),
        "tv" => Ok(Kind::Tv),
        other => Err(AppError::Validation(format!(
            "unknown kind: expected movie or tv, got {other}"
        ))),
    }
}

/// Resolve a request and read its stored list.
pub async fn load(
    state: &AppState,
    section_key: &str,
    kind_raw: Option<&str>,
    filter: Option<&str>,
) -> Result<Loaded, AppError> {
    let Some(section) = Section::from_key(section_key) else {
        return Err(AppError::NotFound);
    };

    let kind = parse_kind(kind_raw)?;

    let Some(variant) = discovery::resolve(section, kind, filter) else {
        return Err(AppError::Validation(format!(
            "unknown filter for this list: {}",
            filter.unwrap_or_default()
        )));
    };

    let Some(stored) = state.store.list_snapshot(variant.id.to_owned()).await? else {
        // The refresh has not run yet. That is a 404 rather than an empty page: an empty list
        // looks like "nothing is popular", which is a lie.
        return Err(AppError::NotFound);
    };

    let items: Vec<Item> = serde_json::from_str(&stored.payload)
        .map_err(|error| AppError::Store(format!("a stored list is unreadable: {error}")))?;
    let synergy: Vec<SynergyRow> = serde_json::from_str(&stored.synergy).unwrap_or_default();

    Ok(Loaded {
        section,
        kind,
        variant,
        refreshed_ts: stored.fetched_ts,
        items,
        synergy,
    })
}

#[derive(Debug, Serialize)]
pub struct DiscoverIndex {
    pub sections: Vec<SectionSummary>,
}

#[derive(Debug, Serialize)]
pub struct SectionSummary {
    pub key: &'static str,
    pub heading: &'static str,
    pub blurb: &'static str,
    /// The most recent refresh across this section's lists, or `None` if none has run.
    pub refreshed_ts: Option<i64>,
}

/// One option in a segmented control.
#[derive(Debug, Serialize)]
pub struct Choice {
    pub value: &'static str,
    pub label: &'static str,
}

#[derive(Debug, Serialize)]
pub struct DiscoverPageJson {
    pub section: &'static str,
    /// "Top films" — the section and the kind, which is the page's actual title.
    pub heading: String,
    pub blurb: &'static str,
    pub kind: &'static str,
    pub filter: &'static str,
    /// Both kinds, always, so the control does not move between pages.
    pub kinds: Vec<Choice>,
    /// The second axis, when this section has one for this kind. One entry means no choice to
    /// offer, and the page hides the control rather than showing a single dead option.
    pub filters: Vec<Choice>,
    /// What that axis is: "Ranked by" for top, "Where" for new.
    pub filter_group: &'static str,
    pub refreshed_ts: i64,
    pub items: Vec<Item>,
    pub synergy: Vec<SynergyRow>,
}

pub async fn index(State(state): State<AppState>) -> Result<Json<DiscoverIndex>, AppError> {
    let mut sections = Vec::with_capacity(Section::ALL.len());

    for section in Section::ALL {
        let mut newest: Option<i64> = None;
        for variant in discovery::VARIANTS
            .iter()
            .filter(|variant| variant.section == section)
        {
            if let Some(stored) = state.store.list_snapshot(variant.id.to_owned()).await? {
                newest = Some(newest.map_or(stored.fetched_ts, |ts| ts.max(stored.fetched_ts)));
            }
        }

        sections.push(SectionSummary {
            key: section.key(),
            heading: section.heading(),
            blurb: section.blurb(),
            refreshed_ts: newest,
        });
    }

    Ok(Json(DiscoverIndex { sections }))
}

pub async fn show(
    State(state): State<AppState>,
    Path(section_key): Path<String>,
    AppQuery(query): AppQuery<PageQuery>,
) -> Result<Json<DiscoverPageJson>, AppError> {
    let loaded = load(
        &state,
        &section_key,
        query.kind.as_deref(),
        query.filter.as_deref(),
    )
    .await?;

    let filters: Vec<Choice> = discovery::variants(loaded.section, loaded.kind)
        .filter(|candidate| !candidate.filter.is_empty())
        .map(|candidate| Choice {
            value: candidate.filter,
            label: candidate.filter_label,
        })
        .collect();

    Ok(Json(DiscoverPageJson {
        section: loaded.section.key(),
        heading: loaded.variant.heading().to_owned(),
        blurb: loaded.variant.blurb(),
        kind: kind_key(loaded.kind),
        filter: loaded.variant.filter,
        kinds: vec![
            Choice {
                value: "movie",
                label: "Films",
            },
            Choice {
                value: "tv",
                label: "Shows",
            },
        ],
        filters,
        filter_group: loaded.section.filter_group(),
        refreshed_ts: loaded.refreshed_ts,
        items: loaded.items,
        synergy: loaded.synergy,
    }))
}

/// The HTML half, at `/discover/{section}`.
pub async fn page(
    State(state): State<AppState>,
    Path(section_key): Path<String>,
    AppQuery(query): AppQuery<PageQuery>,
) -> Result<Response, AppError> {
    let loaded = match load(
        &state,
        &section_key,
        query.kind.as_deref(),
        query.filter.as_deref(),
    )
    .await
    {
        Ok(loaded) => loaded,
        Err(AppError::NotFound) => return Ok(crate::routes::not_found_page(&state)),
        Err(error) => return Err(error),
    };

    let mut head = Head::new(
        format!("{} — same.actor", loaded.variant.heading()),
        format!(
            "{}. Refreshed every few hours.",
            loaded.variant.blurb().trim_end_matches('.')
        ),
        loaded.section.key(),
    );

    if let Some(css) = state.assets.css.as_deref() {
        head = head.with_css(css);
    }
    if let Some(boost) = state.assets.boost.as_deref() {
        head = head.with_boost(boost);
    }

    head = head.with_canonical(format!(
        "{}{}",
        state.config.public_origin.trim_end_matches('/'),
        canonical_path(loaded.section, loaded.kind, loaded.variant.filter)
    ));

    let derived = crate::views::discover::Derived {
        placements: state.store.placements().await?,
        overrides: state.store.all_cameos().await?,
        filter: synergy_filter(query.hide.as_deref(), query.cameos.as_deref()),
        focus: match (query.actor, query.title) {
            (Some(actor), _) => crate::views::discover::Focus::Actor(actor),
            (None, Some(title)) => crate::views::discover::Focus::Title(title),
            _ => crate::views::discover::Focus::Bridges,
        },
    };

    let view = DiscoverPage::new(
        head,
        &Snapshot {
            section: loaded.section,
            kind: loaded.kind,
            variant: loaded.variant,
            refreshed_ts: loaded.refreshed_ts,
            items: &loaded.items,
            synergy: &loaded.synergy,
        },
        crate::clock::now_unix(),
        &derived,
    );

    Ok(render(&view)?.into_response())
}

/// The matrix filters, read out of the query string.
///
/// `cameos=0` means "hidden", because a switch whose default is off reads better as a parameter
/// that only appears when it is on.
fn synergy_filter(hide: Option<&str>, cameos: Option<&str>) -> crate::synergy::Filter {
    crate::synergy::Filter {
        hide: hide
            .unwrap_or_default()
            .split(',')
            .map(str::trim)
            .filter(|slug| !slug.is_empty())
            .map(str::to_owned)
            .collect(),
        hide_cameos: cameos == Some("0"),
    }
}

/// The canonical path for a list. The default page for a section is the bare path, so
/// `/discover/top` and `/discover/top?kind=movie&filter=all-time` are one page to a crawler.
fn canonical_path(section: Section, kind: Kind, filter: &str) -> String {
    let default_filter = discovery::resolve(section, kind, None)
        .map(|variant| variant.filter)
        .unwrap_or("");

    if kind == Kind::Movie && filter == default_filter {
        return format!("/discover/{}", section.key());
    }

    if filter == default_filter {
        return format!("/discover/{}?kind={}", section.key(), kind_key(kind));
    }

    format!(
        "/discover/{}?kind={}&filter={filter}",
        section.key(),
        kind_key(kind)
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Every section the site links to must resolve for both kinds, and every filter it
    /// advertises must resolve too — the control cannot offer a page that 400s.
    #[test]
    fn every_advertised_page_resolves() {
        for section in Section::ALL {
            for kind in [Kind::Movie, Kind::Tv] {
                assert!(
                    discovery::resolve(section, kind, None).is_some(),
                    "{} has no default for {kind:?}",
                    section.key()
                );

                for variant in discovery::variants(section, kind) {
                    assert!(
                        discovery::resolve(section, kind, Some(variant.filter)).is_some(),
                        "{} advertises a filter it cannot resolve",
                        variant.id
                    );
                }
            }
        }
    }

    /// A section with one list must not render a one-option control.
    #[test]
    fn a_single_list_section_offers_no_filter_choice() {
        let count = discovery::variants(Section::Upcoming, Kind::Movie)
            .filter(|variant| !variant.filter.is_empty())
            .count();
        assert_eq!(count, 0);
    }

    #[test]
    fn an_unknown_kind_is_a_validation_error() {
        assert!(parse_kind(Some("person")).is_err());
        assert_eq!(parse_kind(None).expect("default"), Kind::Movie);
        assert_eq!(parse_kind(Some("tv")).expect("tv"), Kind::Tv);
    }

    /// The bare path is the canonical form of the default list, so it must not advertise a
    /// query string it does not need.
    #[test]
    fn the_default_list_has_a_bare_canonical_path() {
        assert_eq!(
            canonical_path(Section::Top, Kind::Movie, "all-time"),
            "/discover/top"
        );
        assert_eq!(
            canonical_path(Section::Top, Kind::Movie, "trending"),
            "/discover/top?kind=movie&filter=trending"
        );
        assert_eq!(
            canonical_path(Section::Top, Kind::Tv, "all-time"),
            "/discover/top?kind=tv"
        );
        assert_eq!(
            canonical_path(Section::Upcoming, Kind::Movie, ""),
            "/discover/upcoming"
        );
    }
}
