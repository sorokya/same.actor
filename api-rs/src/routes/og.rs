//! Open Graph: the share card image and the meta tags on a comparison page.

use std::sync::Arc;

use axum::extract::{Path, State};
use axum::http::{header, HeaderValue, StatusCode};
use axum::response::{IntoResponse, Response};
use tokio::task::JoinSet;

use crate::compare::compare;
use crate::domain::{Kind, Matches, Mode, Selection};
use crate::error::AppError;
use crate::og::{self, Card, Pick};
use crate::paths::{canonical, parse_picks};
use crate::state::AppState;

fn parse_mode(raw: &str) -> Option<Mode> {
    match raw {
        "title" => Some(Mode::Title),
        "person" => Some(Mode::Person),
        _ => None,
    }
}

/// `GET /og/{mode}/{picks}.png`
pub async fn image(
    State(state): State<AppState>,
    Path((mode_raw, picks_raw)): Path<(String, String)>,
) -> Result<Response, AppError> {
    let Some(mode) = parse_mode(&mode_raw) else {
        return Err(AppError::NotFound);
    };

    // The extension is part of the URL so scrapers that sniff the type are happy.
    let picks_raw = picks_raw.trim_end_matches(".png");
    let picks = parse_picks(picks_raw, mode);
    if picks.len() < 2 {
        return Err(AppError::NotFound);
    }

    let key = canonical(mode, &picks);
    if let Some(hit) = state.og_cache.get(&key).await {
        return Ok(png_response(hit.as_slice().to_vec()));
    }

    let png = render_card(&state, mode, &picks).await?;
    state.og_cache.insert(key, Arc::new(png.clone())).await;

    Ok(png_response(png))
}

async fn render_card(
    state: &AppState,
    mode: Mode,
    picks: &[Selection],
) -> Result<Vec<u8>, AppError> {
    let (selections, matches) = compare(&state.tmdb, mode, picks).await?;
    let count = match &matches {
        Matches::People(people) => people.len(),
        Matches::Titles(titles) => titles.len(),
    };

    // Artwork and the attribution mark. Neither can fail the card: a missing poster leaves a
    // placeholder, a missing logo leaves a log line.
    let images = fetch_images(state, &selections).await;
    let logo = state.tmdb.fetch_image(&state.config.tmdb_logo_url).await;

    let card = Card {
        heading: match mode {
            Mode::Title => "What do these films share?".to_owned(),
            Mode::Person => "What do these people share?".to_owned(),
        },
        picks: selections
            .iter()
            .zip(images)
            .map(|(selection, image)| Pick {
                name: selection.name.clone(),
                year: selection.year,
                image,
                round: selection.kind == Kind::Person,
            })
            .collect(),
        count,
        count_label: match (mode, count) {
            (_, 0) => "NO SHARED CREDITS",
            (Mode::Title, _) => "ACTORS IN 2+ OF THESE FILMS",
            (Mode::Person, _) => "TITLES SHARED BY 2+ OF THEM",
        },
        mode_label: match mode {
            Mode::Title => "TITLES",
            Mode::Person => "PEOPLE",
        },
    };

    og::render(&card, logo.as_deref().map(Vec::as_slice)).map_err(AppError::Internal)
}

/// Fetch every selection's artwork concurrently, at the size the card draws it.
///
/// Any single failure leaves a placeholder in that row; artwork is never allowed to cost the
/// card, and the TMDB query is already cached, so this only pays for images.
async fn fetch_images(
    state: &AppState,
    selections: &[crate::domain::SelectionMeta],
) -> Vec<Option<Arc<Vec<u8>>>> {
    let mut tasks = JoinSet::new();

    for (index, selection) in selections.iter().enumerate() {
        let Some(url) = selection.image.clone() else {
            continue;
        };
        let tmdb = Arc::clone(&state.tmdb);
        tasks.spawn(async move { (index, tmdb.fetch_image(&og::thumb_url(&url)).await) });
    }

    let mut out: Vec<Option<Arc<Vec<u8>>>> = vec![None; selections.len()];
    while let Some(joined) = tasks.join_next().await {
        if let Ok((index, image)) = joined {
            if let Some(slot) = out.get_mut(index) {
                *slot = image;
            }
        }
    }

    out
}

fn png_response(body: Vec<u8>) -> Response {
    let mut response = (StatusCode::OK, body).into_response();
    let headers = response.headers_mut();
    headers.insert(header::CONTENT_TYPE, HeaderValue::from_static("image/png"));
    headers.insert(
        header::CACHE_CONTROL,
        HeaderValue::from_static("public, max-age=3600"),
    );
    response
}
