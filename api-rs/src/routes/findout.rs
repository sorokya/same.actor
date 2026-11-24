//! `POST /api/findout`

use std::collections::HashSet;

use axum::extract::State;
use axum::Json;
use serde::Deserialize;

use super::cacheable;
use crate::compare::compare;
use crate::domain::{FindOutResponse, Kind, Mode, Selection, MAX_SELECTIONS, MIN_SELECTIONS};
use crate::error::{invalid, AppError};
use crate::extract::{AppJson, Session};
use crate::state::AppState;
use crate::store::comparisons;
use crate::store::comparisons::NewView;

/// The parts of a comparison that the analytics layer needs: a stable key, a human label, and
/// what the set actually was.
pub(crate) fn view_of(
    mode: Mode,
    selection: &[crate::domain::Selection],
    selections: &[crate::domain::SelectionMeta],
    matches: usize,
) -> NewView {
    let picks: Vec<String> = selection
        .iter()
        .map(|entry| format!("{}-{}", entry.kind.as_str(), entry.id.get()))
        .collect();

    NewView {
        key: comparisons::key(mode.as_str(), &picks),
        mode: mode.as_str().to_owned(),
        label: selections
            .iter()
            .map(|meta| meta.name.as_str())
            .collect::<Vec<_>>()
            .join(" + "),
        picks_json: serde_json::json!(selections
            .iter()
            .map(|meta| serde_json::json!({
                "id": meta.id,
                "kind": meta.kind.as_str(),
                "name": meta.name,
                "year": meta.year,
                // Stored, not fetched: the home page draws a card from this row, and a card
                // that has to ask TMDB for artwork before it can render is not cheap.
                "image": meta.image,
            }))
            .collect::<Vec<_>>())
        .to_string(),
        matches,
    }
}

/// Record a view unless this visitor has already looked at this set recently, and return the
/// count to show.
pub(crate) async fn note_view(state: &AppState, session: Option<&str>, view: NewView) -> i64 {
    let counted = state.views.first_sighting(session, &view.key).await;

    // Read *before* offering the write, then add this view ourselves. The write is batched and
    // asynchronous, so reading afterwards would always miss it — the first visitor to any
    // comparison would be told it had never been viewed.
    let existing = state.store.view_count(view.key.clone()).await.unwrap_or(0);

    if counted {
        state.store.note_view(view);
        existing + 1
    } else {
        existing
    }
}

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
    Session(session): Session,
    AppJson(body): AppJson<FindOutRequest>,
) -> Result<axum::response::Response, AppError> {
    validate(body.mode, &body.selection)?;

    let (selections, matches) = compare(&state.tmdb, body.mode, &body.selection).await?;

    let found = body.mode.count(&matches);
    let view = view_of(body.mode, &body.selection, &selections, found);
    let views = note_view(&state, session.as_deref(), view).await;

    Ok(cacheable(
        CACHE_SECONDS,
        Json(FindOutResponse {
            mode: body.mode,
            selections,
            matches,
            views,
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
