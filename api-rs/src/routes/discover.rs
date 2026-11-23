//! `GET /api/discover` and `GET /api/discover/{slug}`.
//!
//! Reads the stored snapshot; never calls TMDB. The refresh job owns that, and a page that waits
//! on an upstream call is a page that goes down when TMDB does.

use axum::extract::{Path, State};
use axum::Json;
use serde::Serialize;

use crate::discovery::{self, Item, SynergyRow};
use crate::error::AppError;
use crate::state::AppState;

#[derive(Debug, Serialize)]
pub struct DiscoverIndex {
    pub lists: Vec<ListSummary>,
}

#[derive(Debug, Serialize)]
pub struct ListSummary {
    pub slug: &'static str,
    pub heading: &'static str,
    pub blurb: &'static str,
    /// `None` until the first refresh has run.
    pub refreshed_ts: Option<i64>,
}

#[derive(Debug, Serialize)]
pub struct DiscoverPage {
    pub slug: &'static str,
    pub heading: &'static str,
    pub blurb: &'static str,
    /// `movie` or `tv`, so the client can build a correct TMDB link without guessing from the
    /// list's name.
    pub kind: &'static str,
    pub refreshed_ts: i64,
    pub items: Vec<Item>,
    pub synergy: Vec<SynergyRow>,
}

pub async fn index(State(state): State<AppState>) -> Result<Json<DiscoverIndex>, AppError> {
    let mut lists = Vec::with_capacity(discovery::LISTS.len());

    for spec in discovery::LISTS {
        let refreshed_ts = state
            .store
            .list_snapshot(spec.id.to_owned())
            .await?
            .map(|stored| stored.fetched_ts);

        lists.push(ListSummary {
            slug: spec.slug,
            heading: spec.heading,
            blurb: spec.blurb,
            refreshed_ts,
        });
    }

    Ok(Json(DiscoverIndex { lists }))
}

pub async fn show(
    State(state): State<AppState>,
    Path(slug): Path<String>,
) -> Result<Json<DiscoverPage>, AppError> {
    let Some(spec) = discovery::by_slug(&slug) else {
        return Err(AppError::NotFound);
    };

    let Some(stored) = state.store.list_snapshot(spec.id.to_owned()).await? else {
        // The refresh has not run yet. That is a 404 rather than an empty page: an empty list
        // looks like "nothing is popular", which is a lie.
        return Err(AppError::NotFound);
    };

    let items: Vec<Item> = serde_json::from_str(&stored.payload)
        .map_err(|error| AppError::Store(format!("a stored list is unreadable: {error}")))?;
    let synergy: Vec<SynergyRow> = serde_json::from_str(&stored.synergy).unwrap_or_default();

    Ok(Json(DiscoverPage {
        slug: spec.slug,
        heading: spec.heading,
        blurb: spec.blurb,
        kind: match spec.kind {
            crate::domain::Kind::Movie => "movie",
            crate::domain::Kind::Tv => "tv",
            crate::domain::Kind::Person => "person",
        },
        refreshed_ts: stored.fetched_ts,
        items,
        synergy,
    }))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Every list must be reachable by the slug the site links to, and the api index must not
    /// advertise a slug the page handler would refuse.
    #[test]
    fn every_advertised_slug_resolves() {
        for spec in discovery::LISTS {
            assert!(
                discovery::by_slug(spec.slug).is_some(),
                "{} is advertised but not resolvable",
                spec.slug
            );
        }
    }
}
