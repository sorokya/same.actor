//! `GET /api/trending` — what people are searching for right now.
//!
//! Cached for a minute in memory: the strip sits on the home page, and one query a minute is
//! cheaper than one per page view. It also means a burst of searches does not show up instantly,
//! which is the right trade for a page that is meant to look stable rather than live.
//!
//! Only terms searched at least [`MIN_SEARCHES`] times are published. Below that, the page would
//! be a log of individual people's one-off queries, which is not something to put on a public
//! page — and would also mostly be typos.

use axum::extract::State;
use axum::Json;
use serde::Serialize;

use crate::error::AppError;
use crate::state::AppState;
use crate::store::searches::TrendingTerm;

/// How many times a term must have been searched before it is shown.
const MIN_SEARCHES: i64 = 3;
/// How many terms the strip carries.
const LIMIT: u32 = 8;
/// The window, in days. Beyond a day, "trending" stops meaning anything.
const DAYS: i64 = 1;
// Compile-time, not a test: lowering these would publish individual people's searches or turn
// "trending" into "popular last month", and that should fail the build rather than a test run.
const _: () = assert!(MIN_SEARCHES >= 3, "a single search must never be published");
const _: () = assert!(LIMIT <= 20, "a strip, not a directory");
const _: () = assert!(DAYS == 1, "trending is about today");

/// Cache slot. There is one public view of this data, so the key is a constant; it exists so a
/// future "7-day" strip gets its own entry rather than fighting over one.
const CACHE_SLOT: u8 = 0;

#[derive(Debug, Serialize)]
pub struct TrendingResponse {
    pub window: &'static str,
    pub terms: Vec<TrendingTerm>,
}

/// The terms, cached. Shared by the JSON endpoint and the page that renders the strip, so the
/// cache has one writer and the two views cannot disagree.
pub async fn terms(state: &AppState) -> Result<Vec<TrendingTerm>, AppError> {
    if let Some(cached) = state.trending_cache.get(&CACHE_SLOT).await {
        return Ok(cached.as_ref().clone());
    }

    let terms = state.store.trending(DAYS, MIN_SEARCHES, LIMIT).await?;

    state
        .trending_cache
        .insert(CACHE_SLOT, std::sync::Arc::new(terms.clone()))
        .await;

    Ok(terms)
}

pub async fn trending(State(state): State<AppState>) -> Result<Json<TrendingResponse>, AppError> {
    Ok(Json(TrendingResponse {
        window: "24h",
        terms: terms(&state).await?,
    }))
}
