//! The franchise admin: pages, forms, and the suggestions the queue offers.
//!
//! Curation is a hundred small decisions made once, so every action here is a plain form that
//! posts and redirects. The suggestions are computed from what the discovery refresh already
//! stored — a shared TMDB collection, a shared director, overlapping keywords — and are offered as
//! buttons, never applied automatically.
//!
//! One thing is fetched live: a title's cast, on the cameo page. The stored synergy table only
//! keeps people who appear in two or more titles, which is exactly the set a cameo flag is *not*
//! about — the actor who walks through one scene and nothing else is the one worth flagging.

use std::collections::{HashMap, HashSet};

use axum::extract::{Form, Path, Query, State};
use axum::response::{IntoResponse, Redirect, Response};
use axum::routing::{get, post};
use axum::Router;
use serde::Deserialize;

use crate::discovery::Item;
use crate::domain::{Kind, Selection};
use crate::error::AppError;
use crate::state::AppState;
use crate::store::franchises::{Assign, Cameo, Franchise};
use crate::views::franchise::{
    CameosPage, CastRow, FranchisePage, FranchisesPage, QueuePage, QueueRow, Suggestion, TitleRow,
};
use crate::views::render;

/// Billing positions at or beyond this are treated as cameos unless an override says otherwise.
///
/// Fifteen is a starting guess, not a measurement: it was picked to be past the leads and the
/// strong supporting cast of a twenty-title list. It is one constant, and the override table is
/// what makes being wrong about it cheap.
pub const CAMEO_CUTOFF: u32 = 15;

/// The curation pages, mounted under `/admin/franchises`.
pub fn router() -> Router<AppState> {
    Router::new()
        .route("/", get(list).post(create))
        .route("/queue", get(queue))
        .route("/{id}/delete", post(delete))
        .route("/{slug}", get(one))
        .route("/{slug}/rename", post(rename))
        .route("/{slug}/add", post(add))
        .route("/{slug}/remove/{kind}/{id}", post(remove))
        .route("/{slug}/collection/{collection_id}", post(add_collection))
        .route("/{slug}/cameos/{kind}/{id}", get(cameos).post(set_cameo))
        .route(
            "/{slug}/cameos/{kind}/{id}/{person_id}/clear",
            post(clear_cameo),
        )
}

/// A `303 See Other` back to a page.
///
/// The POST handlers all redirect, and the status matters: `303` makes the browser follow with a
/// GET, so a refresh cannot re-submit the form. `Redirect::to` wants a `&'static str`, which a
/// page address built from a slug is not.
fn see_other(location: String) -> Response {
    let mut response = axum::http::StatusCode::SEE_OTHER.into_response();

    if let Ok(value) = axum::http::HeaderValue::from_str(&location) {
        response
            .headers_mut()
            .insert(axum::http::header::LOCATION, value);
    }

    response
}

fn page_head(state: &AppState, title: &str, description: &str) -> crate::views::Head {
    let mut head = crate::views::Head::new(title, description, "admin")
        .noindex()
        .with_active("franchises");
    if let Some(css) = state.assets.css.as_deref() {
        head = head.with_css(css);
    }
    head
}

// ---------------------------------------------------------------- the list

pub async fn list(State(state): State<AppState>) -> Result<Response, AppError> {
    let franchises = state.store.franchises().await?;
    let unassigned = unassigned_items(&state).await?.len();

    let page = FranchisesPage::new(
        page_head(
            &state,
            "Franchises — same.actor",
            "Curated franchise groups.",
        ),
        &franchises,
        unassigned,
    );

    Ok(render(&page)?.into_response())
}

#[derive(Debug, Deserialize)]
pub struct NameForm {
    pub name: String,
}

pub async fn create(
    State(state): State<AppState>,
    Form(form): Form<NameForm>,
) -> Result<Response, AppError> {
    let name = form.name.trim();

    if name.is_empty() {
        return Ok(Redirect::to("/admin/franchises").into_response());
    }

    state.store.create_franchise(name.to_owned()).await?;

    Ok(Redirect::to("/admin/franchises").into_response())
}

pub async fn delete(
    State(state): State<AppState>,
    Path(id): Path<i64>,
) -> Result<Response, AppError> {
    state.store.delete_franchise(id).await?;

    Ok(Redirect::to("/admin/franchises").into_response())
}

pub async fn rename(
    State(state): State<AppState>,
    Path(slug): Path<String>,
    Form(form): Form<NameForm>,
) -> Result<Response, AppError> {
    let Some(franchise) = state.store.franchise(slug.clone()).await? else {
        return Err(AppError::NotFound);
    };

    let name = form.name.trim();
    if !name.is_empty() {
        state
            .store
            .rename_franchise(franchise.id, name.to_owned())
            .await?;
    }

    Ok(see_other(format!("/admin/franchises/{slug}")))
}

// ---------------------------------------------------------------- one franchise

#[derive(Debug, Deserialize)]
pub struct SearchQuery {
    pub q: Option<String>,
}

pub async fn one(
    State(state): State<AppState>,
    Path(slug): Path<String>,
    Query(query): Query<SearchQuery>,
) -> Result<Response, AppError> {
    let Some(franchise) = state.store.franchise(slug.clone()).await? else {
        return Err(AppError::NotFound);
    };

    let titles = state.store.franchise_titles(franchise.id).await?;
    let mut page = FranchisePage::new(
        page_head(
            &state,
            &format!("{} — same.actor", franchise.name),
            "One franchise.",
        ),
        &franchise,
        &titles,
    );

    let term = query.q.unwrap_or_default().trim().to_owned();
    if !term.is_empty() {
        page.results = search_titles(&state, &term).await;
        page.query = term;
    }

    // A collection import needs a title in this franchise that has one — the collection is then
    // offered for the whole group, not for the title that happened to carry it.
    if let Some((collection_id, name, parts)) = collection_for(&state, &franchise).await {
        page.collection_id = collection_id.to_string();
        page.collection_name = name;
        page.collection_parts = parts;
    }

    Ok(render(&page)?.into_response())
}

async fn search_titles(state: &AppState, term: &str) -> Vec<TitleRow> {
    state
        .tmdb
        .search(crate::domain::Mode::Title, term)
        .await
        .unwrap_or_default()
        .into_iter()
        .filter(|result| matches!(result.kind, Kind::Movie | Kind::Tv))
        .map(|result| TitleRow {
            kind: result.kind.as_str().to_owned(),
            id: result.id,
            name: result.name,
            poster: result.image.unwrap_or_default(),
            person: false,
        })
        .collect()
}

/// The TMDB collection this franchise's titles belong to, if they agree on one.
async fn collection_for(state: &AppState, franchise: &Franchise) -> Option<(u32, String, usize)> {
    let items = all_items(state).await.ok()?;
    let assigned = state.store.franchise_titles(franchise.id).await.ok()?;
    let held: HashSet<(String, u32)> = assigned
        .iter()
        .map(|title| (title.kind.clone(), title.media_id))
        .collect();

    let collection = items
        .iter()
        .filter(|item| held.contains(&(item.kind.clone(), item.id)))
        .find_map(|item| item.collection_id)?;

    let detail = state.tmdb.collection(collection).await.ok()?;
    if detail.parts.len() < 2 {
        return None;
    }

    let name = detail
        .name
        .filter(|name| !name.trim().is_empty())
        .unwrap_or_else(|| format!("Collection {collection}"));

    Some((collection, name, detail.parts.len()))
}

#[derive(Debug, Deserialize)]
pub struct AddForm {
    pub kind: String,
    pub media_id: u32,
    pub name: String,
    #[serde(default)]
    pub poster: String,
}

pub async fn add(
    State(state): State<AppState>,
    Path(slug): Path<String>,
    Form(form): Form<AddForm>,
) -> Result<Response, AppError> {
    let Some(franchise) = state.store.franchise(slug.clone()).await? else {
        return Err(AppError::NotFound);
    };

    state
        .store
        .assign_franchise(Assign {
            franchise_id: franchise.id,
            kind: form.kind,
            media_id: form.media_id,
            name: form.name,
            poster: Some(form.poster).filter(|poster| !poster.is_empty()),
        })
        .await?;

    Ok(see_other(format!("/admin/franchises/{slug}")))
}

pub async fn remove(
    State(state): State<AppState>,
    Path((slug, kind, id)): Path<(String, String, u32)>,
) -> Result<Response, AppError> {
    state.store.unassign_franchise(kind, id).await?;

    Ok(see_other(format!("/admin/franchises/{slug}")))
}

/// Import every part of a TMDB collection in one action.
pub async fn add_collection(
    State(state): State<AppState>,
    Path((slug, collection_id)): Path<(String, u32)>,
) -> Result<Response, AppError> {
    let Some(franchise) = state.store.franchise(slug.clone()).await? else {
        return Err(AppError::NotFound);
    };

    let detail = match state.tmdb.collection(collection_id).await {
        Ok(detail) => detail,
        Err(error) => {
            tracing::warn!(%error, collection_id, "could not read a TMDB collection");
            return Ok(see_other(format!("/admin/franchises/{slug}")));
        }
    };

    for part in detail.parts {
        let kind = match part_kind(&part) {
            Some(kind) => kind,
            None => continue,
        };

        state
            .store
            .assign_franchise(Assign {
                franchise_id: franchise.id,
                kind: kind.as_str().to_owned(),
                media_id: part.id,
                name: part.display_name(),
                poster: part
                    .poster_path
                    .as_deref()
                    .map(|path| state.tmdb.image_url(path)),
            })
            .await?;
    }

    Ok(see_other(format!("/admin/franchises/{slug}")))
}

/// A collection part is a film unless TMDB labels it a show.
fn part_kind(part: &crate::tmdb::types::ListItem) -> Option<Kind> {
    match part.media_type.as_deref() {
        Some("movie") => Some(Kind::Movie),
        Some("tv") => Some(Kind::Tv),
        _ => Some(Kind::Movie),
    }
}

// ---------------------------------------------------------------- the queue

pub async fn queue(State(state): State<AppState>) -> Result<Response, AppError> {
    let items = unassigned_items(&state).await?;

    // Facets come from *every* title, not from the queue: the whole point of a suggestion is that
    // a franchise already holds a sibling, and the sibling is by definition not in the queue.
    let facets = facets(&state, &all_items(&state).await?).await?;
    let unassigned = items.len();

    let rows: Vec<QueueRow> = items
        .iter()
        .map(|item| QueueRow {
            kind: item.kind.clone(),
            id: item.id,
            name: item.name.clone(),
            year: item.year.map(|year| year.to_string()).unwrap_or_default(),
            poster: item.poster.clone().unwrap_or_default(),
            suggestions: suggestions(item, &facets),
        })
        .collect();

    let page = QueuePage {
        head: page_head(
            &state,
            "Unassigned — same.actor",
            "Titles with no franchise.",
        ),
        rows,
        unassigned,
        error: String::new(),
        message: String::new(),
    };

    Ok(render(&page)?.into_response())
}

/// One stored list item, with the fields the suggestions are built from.
#[derive(Debug, Clone)]
pub struct QueuedItem {
    pub kind: String,
    pub id: u32,
    pub name: String,
    pub year: Option<u16>,
    pub poster: Option<String>,
    pub collection_id: Option<u32>,
    pub director_ids: Vec<u32>,
    pub keyword_ids: Vec<u32>,
}

/// Every title in any stored list, deduplicated by kind and id.
async fn all_items(state: &AppState) -> Result<Vec<QueuedItem>, AppError> {
    let payloads = state.store.all_list_payloads().await?;
    let mut seen: HashSet<(String, u32)> = HashSet::new();
    let mut items = Vec::new();

    for (list_id, payload) in payloads {
        // The list id says whether its titles are films or shows, and that is the only place the
        // kind survives: an `Item` is the same shape for both.
        let kind = if list_id.contains(":tv") {
            "tv"
        } else {
            "movie"
        };

        let parsed: Vec<Item> = match serde_json::from_str(&payload) {
            Ok(parsed) => parsed,
            // A payload this build cannot read is skipped rather than fatal: the queue is a
            // convenience, and losing one list's suggestions beats a 500 on the admin page.
            Err(_) => continue,
        };

        for item in parsed {
            if !seen.insert((kind.to_owned(), item.id)) {
                continue;
            }

            items.push(QueuedItem {
                kind: kind.to_owned(),
                id: item.id,
                name: item.name,
                year: item.year,
                poster: item.poster,
                collection_id: item.collection_id,
                director_ids: item.director_ids,
                keyword_ids: item.keyword_ids,
            });
        }
    }

    Ok(items)
}

/// Titles in a list that no franchise holds.
async fn unassigned_items(state: &AppState) -> Result<Vec<QueuedItem>, AppError> {
    let placements = state.store.placements().await?;

    Ok(all_items(state)
        .await?
        .into_iter()
        .filter(|item| !placements.contains_key(&(item.kind.clone(), item.id)))
        .collect())
}

/// What each franchise already holds, for scoring a suggestion.
struct Facets {
    franchise: Franchise,
    collections: HashSet<u32>,
    directors: HashSet<u32>,
    keywords: HashSet<u32>,
}

async fn facets(state: &AppState, items: &[QueuedItem]) -> Result<Vec<Facets>, AppError> {
    let franchises = state.store.franchises().await?;
    let by_key: HashMap<(String, u32), &QueuedItem> = items
        .iter()
        .map(|item| ((item.kind.clone(), item.id), item))
        .collect();

    let mut out = Vec::with_capacity(franchises.len());

    for franchise in franchises {
        let assigned = state.store.franchise_titles(franchise.id).await?;
        let mut facets = Facets {
            franchise,
            collections: HashSet::new(),
            directors: HashSet::new(),
            keywords: HashSet::new(),
        };

        for title in assigned {
            // Only titles the stored lists know carry the facets; a franchise can hold a title
            // that no list mentions, and that title simply contributes nothing.
            if let Some(item) = by_key.get(&(title.kind, title.media_id)) {
                facets.collections.extend(item.collection_id);
                facets.directors.extend(item.director_ids.iter().copied());
                facets.keywords.extend(item.keyword_ids.iter().copied());
            }
        }

        out.push(facets);
    }

    Ok(out)
}

/// Reasons this title belongs in a franchise, strongest first.
///
/// A shared collection is the strongest signal TMDB gives — the same series by construction — so
/// it is offered before keyword overlap, and a shared director before that. Each is a *reason*:
/// the button says why, and the maintainer still presses it.
fn suggestions(item: &QueuedItem, facets: &[Facets]) -> Vec<Suggestion> {
    let mut scored: Vec<(u8, usize, &Facets, String)> = Vec::new();

    for facet in facets {
        let collection = item
            .collection_id
            .filter(|collection| facet.collections.contains(collection));
        let director = item
            .director_ids
            .iter()
            .find(|director| facet.directors.contains(director));
        let shared = item
            .keyword_ids
            .iter()
            .filter(|keyword| facet.keywords.contains(keyword))
            .count();

        let (rank, reason) = match (collection, director) {
            (Some(_), _) => (0, "same collection".to_owned()),
            (None, Some(_)) => (1, "same director".to_owned()),
            (None, None) if shared > 0 => (2, format!("{shared} shared keywords")),
            _ => continue,
        };

        scored.push((rank, shared, facet, reason));
    }

    scored.sort_by(|a, b| a.0.cmp(&b.0).then(b.1.cmp(&a.1)));

    scored
        .into_iter()
        .take(3)
        .map(|(_, _, facet, reason)| Suggestion {
            franchise_id: facet.franchise.id,
            slug: facet.franchise.slug.clone(),
            name: facet.franchise.name.clone(),
            reason,
            colour: crate::views::franchise::colour(facet.franchise.sort_order),
            kind: item.kind.clone(),
            media_id: item.id,
            title: item.name.clone(),
            poster: item.poster.clone().unwrap_or_default(),
        })
        .collect()
}

// ---------------------------------------------------------------- cameos

pub async fn cameos(
    State(state): State<AppState>,
    Path((slug, kind, id)): Path<(String, String, u32)>,
) -> Result<Response, AppError> {
    let Some(franchise) = state.store.franchise(slug.clone()).await? else {
        return Err(AppError::NotFound);
    };

    let media_kind = parse_kind(&kind)?;
    let selection = Selection {
        id: std::num::NonZeroU32::new(id).ok_or(AppError::NotFound)?,
        kind: media_kind,
    };

    let facts = match state.tmdb.title_facts(selection).await {
        Ok(facts) => facts,
        Err(error) => {
            tracing::warn!(%error, id, "could not read a cast for the cameo page");
            return Err(AppError::Upstream(
                "could not read that title's cast".into(),
            ));
        }
    };

    let overrides: HashMap<u32, bool> = state
        .store
        .cameos(kind.clone(), id)
        .await?
        .into_iter()
        .map(|cameo| (cameo.person_id, cameo.cameo))
        .collect();

    let mut rows: Vec<CastRow> = facts
        .cast
        .iter()
        .map(|claim| {
            let override_flag = overrides.get(&claim.id).copied();
            let cameo = override_flag.unwrap_or_else(|| is_cameo(claim.order));

            CastRow {
                person_id: claim.id,
                name: claim.name.clone(),
                image: claim.image.clone().unwrap_or_default(),
                order: claim
                    .order
                    .map(|order| order.to_string())
                    .unwrap_or_else(|| "—".to_owned()),
                cameo,
                overridden: override_flag.is_some(),
            }
        })
        .collect();

    rows.sort_by_key(|row| row.order.parse::<u32>().unwrap_or(u32::MAX));

    let page = CameosPage {
        head: page_head(&state, "Cameos — same.actor", "Cameo overrides."),
        slug,
        franchise_name: franchise.name,
        title_name: facts.meta.name,
        kind,
        media_id: id,
        cutoff: CAMEO_CUTOFF,
        rows,
        error: String::new(),
        message: String::new(),
    };

    Ok(render(&page)?.into_response())
}

/// The cutoff's verdict: anyone billed at or below it is cast, everyone else is a cameo.
fn is_cameo(order: Option<u32>) -> bool {
    match order {
        // TMDB did not say where they are billed, so there is no evidence of a cameo.
        None => false,
        Some(order) => order >= CAMEO_CUTOFF,
    }
}

#[derive(Debug, Deserialize)]
pub struct CameoForm {
    pub person_id: u32,
    /// `1` or `0` from the two buttons the template renders.
    pub cameo: String,
}

pub async fn set_cameo(
    State(state): State<AppState>,
    Path((slug, kind, id)): Path<(String, String, u32)>,
    Form(form): Form<CameoForm>,
) -> Result<Response, AppError> {
    state
        .store
        .set_cameo(Cameo {
            kind: kind.clone(),
            media_id: id,
            person_id: form.person_id,
            cameo: form.cameo.trim() == "1",
        })
        .await?;

    Ok(see_other(format!(
        "/admin/franchises/{slug}/cameos/{kind}/{id}"
    )))
}

pub async fn clear_cameo(
    State(state): State<AppState>,
    Path((slug, kind, id, person_id)): Path<(String, String, u32, u32)>,
) -> Result<Response, AppError> {
    state.store.clear_cameo(kind.clone(), id, person_id).await?;

    Ok(see_other(format!(
        "/admin/franchises/{slug}/cameos/{kind}/{id}"
    )))
}

fn parse_kind(raw: &str) -> Result<Kind, AppError> {
    match raw {
        "movie" => Ok(Kind::Movie),
        "tv" => Ok(Kind::Tv),
        other => Err(AppError::Validation(format!(
            "expected movie or tv, got {other}"
        ))),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn item(id: u32) -> QueuedItem {
        QueuedItem {
            kind: "movie".to_owned(),
            id,
            name: "The Matrix".to_owned(),
            year: Some(1999),
            poster: None,
            collection_id: Some(2344),
            director_ids: vec![9339],
            keyword_ids: vec![1721, 2343],
        }
    }

    fn facet(name: &str, sort_order: i64) -> Facets {
        Facets {
            franchise: Franchise {
                id: sort_order + 1,
                slug: name.to_lowercase(),
                name: name.to_owned(),
                sort_order,
                titles: 0,
            },
            collections: HashSet::new(),
            directors: HashSet::new(),
            keywords: HashSet::new(),
        }
    }

    #[test]
    fn a_shared_collection_outranks_everything_else() {
        let mut collection = facet("Marvel", 0);
        collection.collections.insert(2344);

        let mut keywords = facet("Sci-Fi", 1);
        keywords.keywords.extend([1721, 2343]);

        let ranked = suggestions(&item(603), &[keywords, collection]);

        assert_eq!(ranked.len(), 2);
        assert_eq!(ranked[0].name, "Marvel", "collection first");
        assert_eq!(ranked[0].reason, "same collection");
        assert_eq!(ranked[1].name, "Sci-Fi");
        assert_eq!(ranked[1].reason, "2 shared keywords");
    }

    #[test]
    fn a_title_with_nothing_in_common_gets_no_suggestion() {
        let unrelated = facet("Something Else", 0);
        assert!(suggestions(&item(603), &[unrelated]).is_empty());
    }

    #[test]
    fn only_the_three_best_suggestions_are_offered() {
        let facets: Vec<Facets> = (0..5)
            .map(|index| {
                let mut facet = facet(&format!("Franchise {index}"), index);
                facet.collections.insert(2344);
                facet
            })
            .collect();

        assert_eq!(suggestions(&item(603), &facets).len(), 3);
    }

    /// The cutoff is a guess; it must at least be a *documented* guess with a boundary.
    #[test]
    fn the_cutoff_starts_where_the_billing_does_not_say_cameo() {
        assert!(!is_cameo(None), "no billing is not evidence of a cameo");
        assert!(!is_cameo(Some(CAMEO_CUTOFF - 1)));
        assert!(is_cameo(Some(CAMEO_CUTOFF)));
        assert!(is_cameo(Some(CAMEO_CUTOFF + 40)));
    }

    #[test]
    fn collection_parts_default_to_films_and_respect_the_media_type() {
        let mut part: crate::tmdb::types::ListItem = Default::default();
        assert_eq!(part_kind(&part), Some(Kind::Movie));

        part.media_type = Some("tv".to_owned());
        assert_eq!(part_kind(&part), Some(Kind::Tv));
    }
}
