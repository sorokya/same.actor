//! Open Graph: the share card image and the meta tags on a comparison page.

use std::sync::Arc;

use axum::body::Body;
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

/// `GET /title/{picks}` — the SPA, with meta tags injected so a shared link unfurls with the
/// right title, description and card.
pub async fn page_title(
    State(state): State<AppState>,
    Path(picks): Path<String>,
) -> Result<Response, AppError> {
    page(state, Mode::Title, picks).await
}

/// `GET /person/{picks}` — as above, for the people comparison.
pub async fn page_person(
    State(state): State<AppState>,
    Path(picks): Path<String>,
) -> Result<Response, AppError> {
    page(state, Mode::Person, picks).await
}

async fn page(state: AppState, mode: Mode, picks_raw: String) -> Result<Response, AppError> {
    let Some(template) = state.index_html.clone() else {
        // No built SPA (API-only deployment): nothing to render.
        return Err(AppError::NotFound);
    };

    let picks = parse_picks(&picks_raw, mode);

    // A comparison this short has nothing to describe; serve the page untouched.
    if picks.len() < 2 {
        return Ok(html_response(template.to_string(), "no-cache"));
    }

    // Metadata is best-effort: a TMDB hiccup should cost the preview, never the page.
    let meta = match compare(&state.tmdb, mode, &picks).await {
        Ok((selections, matches)) => {
            let count = match &matches {
                Matches::People(people) => people.len(),
                Matches::Titles(titles) => titles.len(),
            };
            Some((selections, count))
        }
        Err(error) => {
            tracing::warn!(%error, "could not build Open Graph tags; serving the page without them");
            None
        }
    };

    let Some((selections, count)) = meta else {
        return Ok(html_response(template.to_string(), "no-store"));
    };

    let names: Vec<&str> = selections.iter().map(|s| s.name.as_str()).collect();
    let title = format!("{} — same.actor", names.join(" vs "));
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
    let canonical_path = canonical(mode, &picks);
    // `/title/movie-603,tv-1396` → `/og/title/movie-603,tv-1396.png`
    let image = format!("{origin}/og{canonical_path}.png");

    let block = format!(
        r#"<title>{title}</title>
<meta name="description" content="{description}" />
<link rel="canonical" href="{origin}{canonical_path}" />
<meta property="og:type" content="website" />
<meta property="og:site_name" content="same.actor" />
<meta property="og:title" content="{title}" />
<meta property="og:description" content="{description}" />
<meta property="og:url" content="{origin}{canonical_path}" />
<meta property="og:image" content="{image}" />
<meta property="og:image:type" content="image/png" />
<meta property="og:image:width" content="1200" />
<meta property="og:image:height" content="630" />
<meta property="og:image:alt" content="{description}" />
<meta name="twitter:card" content="summary_large_image" />
<meta name="twitter:title" content="{title}" />
<meta name="twitter:description" content="{description}" />
<meta name="twitter:image" content="{image}" />"#,
        title = esc(&title),
        description = esc(&description),
        image = esc(&image),
    );

    let html = replace_head(template.as_ref(), &block);
    Ok(html_response(html, "public, max-age=300"))
}

/// Swap the `<!--og:start--> … <!--og:end-->` region of the template for generated tags.
///
/// The region carries a working default, so a plain `/` still has sane previews; replacing
/// the region (rather than appending) is what keeps one `<title>` in the document.
fn replace_head(template: &str, block: &str) -> String {
    const START: &str = "<!--og:start-->";
    const END: &str = "<!--og:end-->";

    if let (Some(start), Some(end)) = (template.find(START), template.find(END)) {
        if start < end {
            let mut out = String::with_capacity(template.len() + block.len());
            out.push_str(&template[..start + START.len()]);
            out.push('\n');
            out.push_str(block);
            out.push('\n');
            out.push_str(&template[end..]);
            return out;
        }
    }

    // No marker (an older build, or a hand-written template): fall back to the head close.
    template.replacen("</head>", &format!("{block}\n</head>"), 1)
}

fn html_response(html: String, cache: &'static str) -> Response {
    let mut response = (StatusCode::OK, Body::from(html)).into_response();
    let headers = response.headers_mut();
    headers.insert(
        header::CONTENT_TYPE,
        HeaderValue::from_static("text/html; charset=utf-8"),
    );
    headers.insert(header::CACHE_CONTROL, HeaderValue::from_static(cache));
    response
}

fn esc(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    for ch in text.chars() {
        match ch {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            '\'' => out.push_str("&#39;"),
            _ => out.push(ch),
        }
    }
    out
}
