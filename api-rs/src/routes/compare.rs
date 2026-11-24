//! `/`, `/person`, `/title/{picks}` and `/person/{picks}` — the comparison pages.
//!
//! These were SPA routes with Open Graph tags spliced into the shell. Now the page *is* the
//! result: the table is in the first response, the meta tags describe what is actually there,
//! and a crawler and a person see the same document.
//!
//! The island on top of the page asks this same route for [`FRAGMENT`] — the results region
//! alone — after a selection changes, so the markup for a result exists in exactly one place.

use axum::extract::{Path, State};
use axum::http::HeaderMap;
use axum::response::{Html, IntoResponse, Response};

use crate::compare::compare;
use crate::domain::{Matches, Mode, Selection, SelectionMeta};
use crate::error::AppError;
use crate::extract::Session;
use crate::paths::{canonical, parse_picks};
use crate::state::AppState;
use crate::views::compare::{ComparePage, ResultsFragment};
use crate::views::{render, Head};

/// The header the island sends to ask for the results region instead of a whole page.
pub const FRAGMENT: &str = "x-fragment";

/// The default preview for a page with nothing selected.
const DEFAULT_TITLE: &str = "same.actor — is it the same actor?";
const DEFAULT_DESCRIPTION: &str =
    "Pick two to five films, shows, or actors and see exactly who or what they share.";

pub async fn home(
    State(state): State<AppState>,
    Session(session): Session,
    headers: HeaderMap,
) -> Result<Response, AppError> {
    response(state, Mode::Title, String::new(), session, headers).await
}

pub async fn people(
    State(state): State<AppState>,
    Session(session): Session,
    headers: HeaderMap,
) -> Result<Response, AppError> {
    response(state, Mode::Person, String::new(), session, headers).await
}

pub async fn titles(
    State(state): State<AppState>,
    Path(picks): Path<String>,
    Session(session): Session,
    headers: HeaderMap,
) -> Result<Response, AppError> {
    response(state, Mode::Title, picks, session, headers).await
}

pub async fn persons(
    State(state): State<AppState>,
    Path(picks): Path<String>,
    Session(session): Session,
    headers: HeaderMap,
) -> Result<Response, AppError> {
    response(state, Mode::Person, picks, session, headers).await
}

async fn response(
    state: AppState,
    mode: Mode,
    picks_raw: String,
    session: Option<String>,
    headers: HeaderMap,
) -> Result<Response, AppError> {
    let picks = parse_picks(&picks_raw, mode);

    // Fewer than two selections is not an error: it is the page before anything is compared,
    // which is also `/` and `/person`.
    let (selections, matches, views) = if picks.len() >= 2 {
        match compare(&state.tmdb, mode, &picks).await {
            Ok((selections, matches)) => {
                let found = mode.count(&matches);
                let view = super::findout::view_of(mode, &picks, &selections, found);
                let views = super::findout::note_view(&state, session.as_deref(), view).await;
                (selections, matches, Some(views as u64))
            }
            Err(error) => {
                // A TMDB hiccup costs the results, not the page: the picker still works, and
                // reloading is a reasonable thing to ask of someone.
                tracing::warn!(%error, "could not build a comparison page");
                (Vec::new(), empty_matches(mode), None)
            }
        }
    } else if picks.len() == 1 {
        // One pick is not a comparison, but the page still has to say what is selected — a
        // shared `/title/movie-603` link should name the film, not show a bare id.
        (
            single_meta(&state, mode, picks[0])
                .await
                .into_iter()
                .collect(),
            empty_matches(mode),
            None,
        )
    } else {
        (Vec::new(), empty_matches(mode), None)
    };

    let share_url = format!(
        "{}{}",
        state.config.public_origin.trim_end_matches('/'),
        if picks.is_empty() {
            format!("/{}", mode.as_str())
        } else {
            canonical(mode, &picks)
        }
    );

    let mut head = head_for(&state, mode, &picks, &selections, &matches, &share_url);

    // Every comparison page carries the picker, including the empty one — that is the page you
    // pick from. The browse pages carry no script at all.
    if let Some(css) = state.assets.css.as_deref() {
        head = head.with_css(css);
    }
    if let Some(island) = state.assets.island.as_deref() {
        head = head.with_island(island);
    }
    if let Some(boost) = state.assets.boost.as_deref() {
        head = head.with_boost(boost);
    }

    let mut page = ComparePage::new(head, mode, &selections, &matches, views, share_url);

    // The strip is only worth the query when the page has nothing to show yet, and it is the
    // same cached list the JSON endpoint serves.
    let matched = mode.count(&matches);

    if matched == 0 {
        if let Ok(terms) = super::trending::terms(&state).await {
            let words: Vec<String> = terms.into_iter().map(|term| term.term).collect();
            page = page.with_trending(crate::views::compare::trend_links(&words, mode));
        }

        // Things to click, for the page that has nothing selected: what has been compared
        // lately and worked, or a curated pair when nothing has been compared at all.
        page.pairs = crate::views::start_here::for_mode(&state, mode).await;
    }

    if fragment_requested(&headers) {
        let fragment = ResultsFragment {
            results: page.results.clone(),
        };
        return Ok(fragment_response(render(&fragment)?));
    }

    let mut response = render(&page)?.into_response();

    // A comparison with a real answer is worth caching briefly; the empty picker is not.
    let cache = if picks.len() >= 2 {
        "public, max-age=60"
    } else {
        "no-cache"
    };
    response.headers_mut().insert(
        axum::http::header::CACHE_CONTROL,
        axum::http::HeaderValue::from_static(cache),
    );
    response.headers_mut().insert(
        axum::http::header::VARY,
        axum::http::HeaderValue::from_static("X-Fragment, Cookie"),
    );

    Ok(response)
}

fn fragment_requested(headers: &HeaderMap) -> bool {
    headers
        .get(FRAGMENT)
        .and_then(|value| value.to_str().ok())
        .map(|value| value.eq_ignore_ascii_case("results"))
        .unwrap_or(false)
}

/// A fragment is a response body without a document around it: no caching (its contents change
/// with the selection), and `Vary` so nothing between here and the browser confuses the two.
fn fragment_response(body: Html<String>) -> Response {
    let mut response = body.into_response();
    response.headers_mut().insert(
        axum::http::header::CACHE_CONTROL,
        axum::http::HeaderValue::from_static("no-store"),
    );
    response.headers_mut().insert(
        axum::http::header::VARY,
        axum::http::HeaderValue::from_static("X-Fragment"),
    );
    response
}

/// Resolve one selection on its own, for the page that has one pick and no comparison yet.
async fn single_meta(state: &AppState, mode: Mode, pick: Selection) -> Option<SelectionMeta> {
    match mode {
        Mode::Title => state
            .tmdb
            .title_selection(pick)
            .await
            .ok()
            .map(|(meta, _)| meta),
        Mode::Person => state
            .tmdb
            .person_selection(pick)
            .await
            .ok()
            .map(|(meta, _)| meta),
    }
}

fn empty_matches(mode: Mode) -> Matches {
    match mode {
        Mode::Title => Matches::People(Vec::new()),
        Mode::Person => Matches::Titles(Vec::new()),
    }
}

/// Title, description, canonical URL and share-card image for the page.
fn head_for(
    state: &AppState,
    mode: Mode,
    picks: &[Selection],
    selections: &[SelectionMeta],
    matches: &Matches,
    share_url: &str,
) -> Head {
    let nav = mode.as_str();

    if selections.len() < 2 {
        return Head::new(DEFAULT_TITLE, DEFAULT_DESCRIPTION, nav);
    }

    let names: Vec<&str> = selections.iter().map(|meta| meta.name.as_str()).collect();
    let count = match matches {
        Matches::People(people) => people.len(),
        Matches::Titles(titles) => titles.len(),
    };

    let description = match (mode, count) {
        (Mode::Title, 0) => "No cast shared by every one of these titles.".to_owned(),
        (Mode::Title, n) => format!(
            "{n} actor{} credited in at least two of {}.",
            if n == 1 { "" } else { "s" },
            names.join(", ")
        ),
        (Mode::Person, 0) => "No titles shared by these people.".to_owned(),
        (Mode::Person, n) => format!(
            "{n} title{} featuring at least two of {}.",
            if n == 1 { "" } else { "s" },
            names.join(", ")
        ),
    };

    let origin = state.config.public_origin.trim_end_matches('/');
    let image = format!("{origin}/og{}.png", canonical(mode, picks));

    Head::new(
        format!("{} — same.actor", names.join(" vs ")),
        description,
        nav,
    )
    .with_canonical(share_url.to_owned())
    .with_image(image)
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::http::HeaderValue;

    #[test]
    fn only_the_exact_fragment_header_asks_for_a_fragment() {
        let mut headers = HeaderMap::new();
        assert!(!fragment_requested(&headers), "no header is a whole page");

        headers.insert(FRAGMENT, HeaderValue::from_static("results"));
        assert!(fragment_requested(&headers));

        headers.insert(FRAGMENT, HeaderValue::from_static("RESULTS"));
        assert!(fragment_requested(&headers), "case is not significant");

        headers.insert(FRAGMENT, HeaderValue::from_static("everything"));
        assert!(!fragment_requested(&headers), "an unknown region is a page");
    }

    /// The kind a title row links with comes from the match, not from the mode: a show in a
    /// person comparison must link to `/tv/`, not `/movie/`.
    #[test]
    fn the_empty_state_matches_the_mode() {
        assert!(matches!(empty_matches(Mode::Title), Matches::People(_)));
        assert!(matches!(empty_matches(Mode::Person), Matches::Titles(_)));
    }
}
