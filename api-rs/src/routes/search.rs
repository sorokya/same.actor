//! `GET /api/search`

use axum::extract::State;
use axum::Json;
use serde::Deserialize;

use super::cacheable;
use crate::domain::{Mode, SearchResponse};
use crate::error::{invalid, AppError};
use crate::extract::AppQuery;
use crate::state::AppState;

const MIN_TERM_CHARS: usize = 3;
const MAX_TERM_CHARS: usize = 100;
/// Search results change slowly; a minute of caching turns a typing user into one request.
const CACHE_SECONDS: u32 = 60;

#[derive(Debug, Deserialize)]
pub struct SearchQuery {
    pub term: String,
    #[serde(default)]
    pub mode: Mode,
}

pub async fn search(
    State(state): State<AppState>,
    AppQuery(params): AppQuery<SearchQuery>,
) -> Result<axum::response::Response, AppError> {
    let term = params.term.trim();

    let length = term.chars().count();
    if length < MIN_TERM_CHARS {
        return Err(invalid(format!(
            "Search term must be at least {MIN_TERM_CHARS} characters."
        )));
    }
    if length > MAX_TERM_CHARS {
        return Err(invalid(format!(
            "Search term must be at most {MAX_TERM_CHARS} characters."
        )));
    }

    let results = state.tmdb.search(params.mode, term).await?;

    Ok(cacheable(CACHE_SECONDS, Json(SearchResponse { results })))
}
