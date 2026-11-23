//! The comparison itself: fetch every selection, intersect, return both the resolved labels
//! and the matches.
//!
//! Shared by `POST /api/findout` and the Open Graph routes, so the card and the page can
//! never disagree about what a comparison contains.

use std::sync::Arc;

use tokio::task::JoinSet;

use crate::domain::{Matches, Mode, Selection, SelectionMeta};
use crate::error::AppError;
use crate::intersect::{intersect_people, intersect_titles, PersonClaim, TitleClaim};
use crate::tmdb::Tmdb;

/// One credit task's result: the position it was requested at, the resolved label, and the
/// credits themselves. This shape is shared by both fetchers and appears in three
/// signatures, so it gets a name.
type Fetched<C> = (usize, SelectionMeta, Vec<C>);

pub async fn compare(
    tmdb: &Arc<Tmdb>,
    mode: Mode,
    picks: &[Selection],
) -> Result<(Vec<SelectionMeta>, Matches), AppError> {
    match mode {
        Mode::Title => {
            let (selections, claims) = fetch_casts(tmdb, picks).await?;
            Ok((selections, Matches::People(intersect_people(&claims))))
        }
        Mode::Person => {
            let (selections, claims) = fetch_filmographies(tmdb, picks).await?;
            Ok((selections, Matches::Titles(intersect_titles(&claims))))
        }
    }
}

/// Cast of every selected title, in request order.
async fn fetch_casts(
    tmdb: &Arc<Tmdb>,
    picks: &[Selection],
) -> Result<(Vec<SelectionMeta>, Vec<(u32, Vec<PersonClaim>)>), AppError> {
    let mut tasks = JoinSet::new();

    for (index, entry) in picks.iter().copied().enumerate() {
        let tmdb = Arc::clone(tmdb);
        tasks.spawn(async move {
            let (meta, claims) = tmdb.title_selection(entry).await?;
            Ok::<_, AppError>((index, meta, claims))
        });
    }

    collect(tasks, picks.len()).await
}

/// Filmography of every selected person, in request order.
async fn fetch_filmographies(
    tmdb: &Arc<Tmdb>,
    picks: &[Selection],
) -> Result<(Vec<SelectionMeta>, Vec<(u32, Vec<TitleClaim>)>), AppError> {
    let mut tasks = JoinSet::new();

    for (index, entry) in picks.iter().copied().enumerate() {
        let tmdb = Arc::clone(tmdb);
        tasks.spawn(async move {
            let (meta, claims) = tmdb.person_selection(entry).await?;
            Ok::<_, AppError>((index, meta, claims))
        });
    }

    collect(tasks, picks.len()).await
}

/// Drain a set of credit tasks into `(selections, per-selection claims)`.
///
/// A panicking task becomes a `JoinError` and therefore a 500 for that one request rather
/// than an aborted process — the structural fix for the review's C2.
async fn collect<C: Send + 'static>(
    mut tasks: JoinSet<Result<Fetched<C>, AppError>>,
    expected: usize,
) -> Result<(Vec<SelectionMeta>, Vec<(u32, Vec<C>)>), AppError> {
    let mut fetched = Vec::with_capacity(expected);

    while let Some(joined) = tasks.join_next().await {
        match joined {
            Ok(result) => fetched.push(result?),
            Err(error) => return Err(AppError::Internal(format!("credit task failed: {error}"))),
        }
    }

    // JoinSet yields in completion order; the response must not depend on which upstream
    // call happened to answer first.
    fetched.sort_by_key(|(index, _, _)| *index);

    let mut selections = Vec::with_capacity(fetched.len());
    let mut claims = Vec::with_capacity(fetched.len());
    for (_, meta, list) in fetched {
        claims.push((meta.id, list));
        selections.push(meta);
    }

    Ok((selections, claims))
}
