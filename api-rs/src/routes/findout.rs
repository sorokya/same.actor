//! `POST /api/findout`

use std::collections::HashSet;

use axum::extract::State;
use axum::Json;
use serde::Deserialize;
use tokio::task::JoinSet;

use super::cacheable;
use crate::domain::{
    FindOutResponse, Kind, Matches, Mode, Selection, SelectionMeta, MAX_SELECTIONS, MIN_SELECTIONS,
};
use crate::error::{invalid, AppError};
use crate::extract::AppJson;
use crate::intersect::{intersect_people, intersect_titles};
use crate::state::AppState;

/// Credits are stable; ten minutes of caching keeps repeat comparisons off the upstream quota
/// without ever being stale in a way that matters (review H6).
const CACHE_SECONDS: u32 = 600;

#[derive(Debug, Deserialize)]
pub struct FindOutRequest {
    pub mode: Mode,
    pub selection: Vec<Selection>,
}

pub async fn findout(
    State(state): State<AppState>,
    AppJson(body): AppJson<FindOutRequest>,
) -> Result<axum::response::Response, AppError> {
    validate(body.mode, &body.selection)?;

    let (selections, matches) = match body.mode {
        Mode::Title => {
            let (selections, per_selection) = fetch_titles(state, &body.selection).await?;
            (
                selections,
                Matches::People(intersect_people(&per_selection)),
            )
        }
        Mode::Person => {
            let (selections, per_selection) = fetch_people(state, &body.selection).await?;
            (
                selections,
                Matches::Titles(intersect_titles(&per_selection)),
            )
        }
    };

    Ok(cacheable(
        CACHE_SECONDS,
        Json(FindOutResponse {
            mode: body.mode,
            selections,
            matches,
        }),
    ))
}

/// Reject anything the type system could not already reject.
///
/// This is the server-side guard the original API never had: it accepted a single selection,
/// then dereferenced a `null` and took the process down (review C2/M7).
fn validate(mode: Mode, selection: &[Selection]) -> Result<(), AppError> {
    if selection.len() < MIN_SELECTIONS || selection.len() > MAX_SELECTIONS {
        return Err(invalid(format!(
            "Select between {MIN_SELECTIONS} and {MAX_SELECTIONS} entries to compare."
        )));
    }

    let mut seen = HashSet::with_capacity(selection.len());
    for entry in selection {
        if !seen.insert(entry.id.get()) {
            return Err(invalid("The same entry is selected more than once."));
        }
    }

    match mode {
        Mode::Title => {
            if selection.iter().any(|s| !s.kind.is_title()) {
                return Err(invalid(
                    "Comparing titles requires movie or tv entries only.",
                ));
            }
        }
        Mode::Person => {
            if selection.iter().any(|s| s.kind != Kind::Person) {
                return Err(invalid("Comparing people requires person entries only."));
            }
        }
    }

    Ok(())
}

/// Fetch every selection concurrently and return the resolved labels plus the per-selection
/// claims, in request order.
///
/// A panicking task becomes a `JoinError` and therefore a 500 for that one request, rather
/// than an aborted process. That is the structural fix for C2: upstream failure and internal
/// bugs are both contained.
async fn fetch_people(
    state: AppState,
    selection: &[Selection],
) -> Result<
    (
        Vec<SelectionMeta>,
        Vec<(u32, Vec<crate::intersect::TitleClaim>)>,
    ),
    AppError,
> {
    let mut tasks = JoinSet::new();

    for (index, entry) in selection.iter().copied().enumerate() {
        let tmdb = state.tmdb.clone();
        tasks.spawn(async move {
            let (meta, claims) = tmdb.person_selection(entry).await?;
            Ok::<_, AppError>((index, meta, claims))
        });
    }

    let mut fetched = Vec::with_capacity(selection.len());
    while let Some(joined) = tasks.join_next().await {
        fetched.push(join(joined)?);
    }

    Ok(in_request_order(fetched))
}

async fn fetch_titles(
    state: AppState,
    selection: &[Selection],
) -> Result<
    (
        Vec<SelectionMeta>,
        Vec<(u32, Vec<crate::intersect::PersonClaim>)>,
    ),
    AppError,
> {
    let mut tasks = JoinSet::new();

    for (index, entry) in selection.iter().copied().enumerate() {
        let tmdb = state.tmdb.clone();
        tasks.spawn(async move {
            let (meta, claims) = tmdb.title_selection(entry).await?;
            Ok::<_, AppError>((index, meta, claims))
        });
    }

    let mut fetched = Vec::with_capacity(selection.len());
    while let Some(joined) = tasks.join_next().await {
        fetched.push(join(joined)?);
    }

    Ok(in_request_order(fetched))
}

/// JoinSet yields in completion order; the response must not depend on which upstream call
/// happened to answer first.
fn in_request_order<C>(
    mut fetched: Vec<(usize, SelectionMeta, Vec<C>)>,
) -> (Vec<SelectionMeta>, Vec<(u32, Vec<C>)>) {
    fetched.sort_by_key(|(index, _, _)| *index);

    let mut selections = Vec::with_capacity(fetched.len());
    let mut claims = Vec::with_capacity(fetched.len());
    for (_, meta, list) in fetched {
        claims.push((meta.id, list));
        selections.push(meta);
    }

    (selections, claims)
}

fn join<T>(joined: Result<Result<T, AppError>, tokio::task::JoinError>) -> Result<T, AppError> {
    match joined {
        Ok(result) => result,
        Err(error) => Err(AppError::Internal(format!("credit task failed: {error}"))),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::num::NonZeroU32;

    fn sel(id: u32, kind: Kind) -> Selection {
        Selection {
            id: NonZeroU32::new(id).unwrap(),
            kind,
        }
    }

    #[test]
    fn a_single_selection_is_rejected_before_any_io() {
        // The exact request that used to crash the Node process.
        let err = validate(Mode::Person, &[sel(603, Kind::Movie)]).unwrap_err();
        assert!(matches!(err, AppError::Validation(_)));
    }

    #[test]
    fn more_than_five_selections_are_rejected() {
        let six: Vec<Selection> = (1..=6).map(|i| sel(i, Kind::Movie)).collect();
        assert!(validate(Mode::Title, &six).is_err());
    }

    #[test]
    fn zero_selections_are_rejected() {
        assert!(validate(Mode::Title, &[]).is_err());
    }

    #[test]
    fn five_selections_are_accepted() {
        let five: Vec<Selection> = (1..=5).map(|i| sel(i, Kind::Movie)).collect();
        assert!(validate(Mode::Title, &five).is_ok());
    }

    #[test]
    fn persons_and_titles_cannot_be_mixed() {
        let mixed = [sel(603, Kind::Movie), sel(6384, Kind::Person)];
        assert!(validate(Mode::Title, &mixed).is_err());
        assert!(validate(Mode::Person, &mixed).is_err());
    }

    #[test]
    fn a_person_compared_as_a_title_is_rejected() {
        let people = [sel(6384, Kind::Person), sel(530, Kind::Person)];
        assert!(validate(Mode::Person, &people).is_ok());
        assert!(validate(Mode::Title, &people).is_err());
    }

    #[test]
    fn duplicate_selections_are_rejected() {
        let duplicated = [sel(603, Kind::Movie), sel(603, Kind::Movie)];
        assert!(validate(Mode::Title, &duplicated).is_err());
    }
}
