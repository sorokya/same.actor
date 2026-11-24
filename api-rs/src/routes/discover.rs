//! `GET /api/discover` and `GET /api/discover/{section}`.
//!
//! Reads the stored snapshot; never calls TMDB. The refresh job owns that, and a page that waits
//! on an upstream call is a page that goes down when TMDB does.

use axum::extract::{Path, State};
use axum::Json;
use serde::{Deserialize, Serialize};

use crate::discovery::{self, Item, Section, SynergyRow};
use crate::domain::Kind;
use crate::error::AppError;
use crate::extract::AppQuery;
use crate::state::AppState;

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
pub struct DiscoverPage {
    pub section: &'static str,
    /// "Top films" — the section and the kind, which is the page's actual title.
    pub heading: String,
    pub blurb: &'static str,
    pub kind: &'static str,
    pub filter: &'static str,
    /// Both kinds, always, so the control does not move between pages.
    pub kinds: Vec<Choice>,
    /// The second axis, when this section has one for this kind. One entry means no choice to
    /// offer, and the client hides the control rather than showing a single dead option.
    pub filters: Vec<Choice>,
    /// What that axis is: "Ranked by" for top, "Where" for new.
    pub filter_group: &'static str,
    pub refreshed_ts: i64,
    pub items: Vec<Item>,
    pub synergy: Vec<SynergyRow>,
}

#[derive(Debug, Deserialize)]
pub struct PageQuery {
    pub kind: Option<String>,
    pub filter: Option<String>,
}

fn kind_label(kind: Kind) -> &'static str {
    match kind {
        Kind::Movie => "Films",
        Kind::Tv => "Shows",
        Kind::Person => "People",
    }
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
) -> Result<Json<DiscoverPage>, AppError> {
    let Some(section) = Section::from_key(&section_key) else {
        return Err(AppError::NotFound);
    };

    let kind = parse_kind(query.kind.as_deref())?;

    let Some(variant) = discovery::resolve(section, kind, query.filter.as_deref()) else {
        return Err(AppError::Validation(format!(
            "unknown filter for this list: {}",
            query.filter.as_deref().unwrap_or_default()
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

    let filters: Vec<Choice> = discovery::variants(section, kind)
        .filter(|candidate| !candidate.filter.is_empty())
        .map(|candidate| Choice {
            value: candidate.filter,
            label: candidate.filter_label,
        })
        .collect();

    Ok(Json(DiscoverPage {
        section: section.key(),
        heading: format!("{} {}", section.heading(), kind_label(kind).to_lowercase()),
        blurb: section.blurb(),
        kind: kind_key(kind),
        filter: variant.filter,
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
        filter_group: section.filter_group(),
        refreshed_ts: stored.fetched_ts,
        items,
        synergy,
    }))
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
}
