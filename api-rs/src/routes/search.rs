//! `GET /api/search`

use axum::extract::{Request, State};
use axum::Json;
use serde::Deserialize;

use super::cacheable;
use crate::domain::{Mode, SearchResponse};
use crate::error::{invalid, AppError};
use crate::extract::AppQuery;
use crate::session::SessionId;
use crate::state::AppState;
use crate::store::searches::SearchEvent;

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
    request: Request,
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

    // What people look for — and what they look for and do not find — is the most useful
    // product signal this app produces. Offered, not awaited: the search has already been
    // answered and the visitor should not wait for the counter.
    state.store.record_search(SearchEvent {
        ts: crate::clock::now_unix(),
        mode: params.mode.as_str().to_owned(),
        term: term.to_owned(),
        term_norm: crate::store::searches::normalise(term),
        results: results.len() as u32,
        session: request
            .extensions()
            .get::<SessionId>()
            .map(|id| id.0.clone()),
        // The address is already on the request row; duplicating it here would be a second
        // copy of personal data for no gain, so it is left out.
        ip: None,
    });

    Ok(cacheable(CACHE_SECONDS, Json(SearchResponse { results })))
}
