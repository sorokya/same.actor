//! Same.Actor — API and SPA in one binary.
//!
//! Layer order, outermost first: tracing → panic containment → compression → security
//! headers → (static files, `/api` routes → rate limit).

#![forbid(unsafe_code)]

use std::path::PathBuf;

use axum::http::{header, HeaderValue};
use axum::routing::{get, post};
use axum::{middleware, Router};
use tower_http::catch_panic::CatchPanicLayer;
use tower_http::compression::CompressionLayer;
use tower_http::services::{ServeDir, ServeFile};
use tower_http::set_header::SetResponseHeaderLayer;
use tower_http::trace::TraceLayer;

pub mod admin;
pub mod assets;
pub mod bans;
pub mod clock;
pub mod compare;
pub mod config;
pub mod discovery;
pub mod domain;
pub mod error;
pub mod extract;
pub mod geo;
pub mod intersect;
pub mod maintenance;
pub mod og;
pub mod pageviews;
pub mod paths;
pub mod ratelimit;
pub mod routes;
pub mod session;
pub mod state;
pub mod store;
pub mod tmdb;
pub mod views;

use state::AppState;

/// Build the application router.
///
/// Kept separate from `main` so tests can drive it without binding a port.
pub fn build_app(state: AppState) -> Router {
    // The two endpoints that call TMDB and therefore *are* a visit. `require_session` sits
    // inside the rate limit so a caller with no cookie is still counted before being told to
    // come back with one — otherwise 428s would be free.
    // Trending is a cached aggregate of counts: there is no visit to attribute and the strip
    // renders on first paint, so it is deliberately outside `require_session`.
    let public_api = Router::new()
        .route("/trending", get(routes::trending::trending))
        .route("/discover", get(routes::discover::index))
        .route("/discover/{slug}", get(routes::discover::show))
        .layer(middleware::from_fn_with_state(
            state.clone(),
            routes::enforce_rate_limit,
        ))
        .with_state(state.clone());

    let api = Router::new()
        .route("/search", get(routes::search))
        .route("/findout", post(routes::findout))
        .layer(middleware::from_fn_with_state(
            state.clone(),
            routes::require_session,
        ))
        .layer(middleware::from_fn_with_state(
            state.clone(),
            routes::enforce_rate_limit,
        ))
        .with_state(state.clone());

    // Share cards are the most expensive thing here to produce (two upstream calls plus a
    // raster), so they get the same per-IP budget as the API. The pages themselves are never
    // rate limited: a human must be able to open a link.
    let share_card = Router::new()
        .route("/og/{mode}/{picks}", get(routes::og::image))
        .layer(middleware::from_fn_with_state(
            state.clone(),
            routes::enforce_rate_limit,
        ))
        .with_state(state.clone());

    // Server-rendered pages. These are real routes now, not paths handed to the SPA, so the
    // content is in the first response and the URL is the same for a person and a crawler.
    let pages = Router::new()
        .route("/discover/{section}", get(routes::discover::page))
        .with_state(state.clone());

    // The comparison pages: server-rendered, with their own metadata.
    let comparison_page = Router::new()
        .route("/", get(routes::compare::home))
        .route("/person", get(routes::compare::people))
        .route("/title/{picks}", get(routes::compare::titles))
        .route("/person/{picks}", get(routes::compare::persons))
        .with_state(state.clone());

    // The admin API is mounted unconditionally and guards itself: when ADMIN_TOKEN is unset it
    // answers 404, which is the same answer as a route that does not exist. Not mounting it
    // would let the SPA fallback answer `/api/admin/...` with an HTML page and a 200.
    // The login route is outside the guard; everything else is inside it. When ADMIN_TOKEN is
    // unset the guard answers 404 for the whole subtree, which is the same answer as a route
    // that does not exist — not mounting it at all would let the SPA fallback answer with an
    // HTML page and a 200.
    let admin = admin::open_router()
        .merge(
            admin::guarded_router().layer(middleware::from_fn_with_state(
                state.clone(),
                admin::authorize,
            )),
        )
        .with_state(state.clone());

    // The dashboard, as pages. The login form is unguarded; everything else redirects to it.
    let admin_pages = admin::pages::open_router()
        .merge(admin::pages::page_router(state.clone()))
        .with_state(state.clone());

    let mut app = Router::new()
        .route("/healthz", get(routes::healthz))
        .nest("/admin", admin_pages)
        .nest("/api/admin", admin)
        .nest("/api", api)
        .nest("/api", public_api)
        .merge(share_card)
        .merge(comparison_page)
        .merge(pages);

    let static_dir = PathBuf::from(&state.config.static_dir);
    if static_dir.is_dir() {
        // Single-page app: unknown paths return index.html so client-side routes survive a
        // refresh or a shared link.
        let serve = ServeDir::new(&static_dir)
            .append_index_html_on_directories(true)
            .fallback(ServeFile::new(static_dir.join("index.html")));
        app = app.fallback_service(serve);
        tracing::info!(dir = %static_dir.display(), "serving the SPA");
    } else {
        tracing::warn!(
            dir = %static_dir.display(),
            "no static directory; serving the API only"
        );
    }

    app.layer(SetResponseHeaderLayer::overriding(
        header::X_CONTENT_TYPE_OPTIONS,
        HeaderValue::from_static("nosniff"),
    ))
    .layer(SetResponseHeaderLayer::overriding(
        header::REFERRER_POLICY,
        HeaderValue::from_static("strict-origin-when-cross-origin"),
    ))
    .layer(SetResponseHeaderLayer::overriding(
        header::X_FRAME_OPTIONS,
        HeaderValue::from_static("DENY"),
    ))
    .layer(CompressionLayer::new())
    // A panic in one handler is a 500 for that request, not an outage. Requires unwinding,
    // which is why the release profile does not set `panic = "abort"`.
    .layer(CatchPanicLayer::new())
    // Outside the panic handler: a panicking handler has to produce a log row with a 500, and
    // that response only exists once `CatchPanicLayer` has made it.
    // Outside the rate limit and outside every route: a banned client does not get to reach
    // anything that costs upstream time.
    .layer(middleware::from_fn_with_state(
        state.clone(),
        routes::enforce_bans,
    ))
    .layer(middleware::from_fn_with_state(
        state.clone(),
        routes::observe,
    ))
    .layer(TraceLayer::new_for_http())
}
