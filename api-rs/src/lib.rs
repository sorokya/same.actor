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

pub mod config;
pub mod domain;
pub mod error;
pub mod extract;
pub mod intersect;
pub mod ratelimit;
pub mod routes;
pub mod state;
pub mod tmdb;

use state::AppState;

/// Build the application router.
///
/// Kept separate from `main` so tests can drive it without binding a port.
pub fn build_app(state: AppState) -> Router {
    let api = Router::new()
        .route("/search", get(routes::search))
        .route("/findout", post(routes::findout))
        .layer(middleware::from_fn_with_state(
            state.clone(),
            routes::enforce_rate_limit,
        ))
        .with_state(state.clone());

    let mut app = Router::new()
        .route("/healthz", get(routes::healthz))
        .nest("/api", api);

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
    .layer(TraceLayer::new_for_http())
}
