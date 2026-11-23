//! HTTP handlers.

use axum::extract::{Request, State};
use axum::http::header;
use axum::middleware::Next;
use axum::response::{IntoResponse, Response};
use axum::Json;
use serde_json::json;
use std::net::{IpAddr, Ipv4Addr, SocketAddr};

use crate::error::AppError;
use crate::state::AppState;

pub mod bans;
pub mod findout;
pub mod observe;
pub mod og;
pub mod search;
pub mod trending;

pub use bans::enforce_bans;
pub use findout::findout;
pub use observe::{observe, require_session};
pub use search::search;

/// Liveness. Deliberately does not touch TMDB: a liveness probe that depends on a third
/// party will restart a healthy process during someone else's outage.
pub async fn healthz() -> impl IntoResponse {
    (
        [(header::CACHE_CONTROL, "no-store")],
        Json(json!({ "status": "ok" })),
    )
}

/// Attach a cache lifetime to a successful response.
pub fn cacheable(seconds: u32, body: Json<impl serde::Serialize>) -> Response {
    let value = format!("public, max-age={seconds}");
    ([(header::CACHE_CONTROL, value)], body).into_response()
}

/// Resolve the visitor's address for rate limiting.
///
/// `X-Forwarded-For` is only honoured when `TRUST_FORWARDED_FOR` is set, because an
/// unvalidated `X-Forwarded-For` lets any client pick its own bucket and the limiter becomes
/// decorative. When connections arrive directly (no proxy), the socket peer is authoritative.
pub fn client_ip(request: &Request, trust_forwarded_for: bool) -> IpAddr {
    if trust_forwarded_for {
        if let Some(value) = request.headers().get("x-forwarded-for") {
            if let Ok(value) = value.to_str() {
                // The left-most entry is the originating client.
                if let Some(first) = value.split(',').next() {
                    if let Ok(ip) = first.trim().parse() {
                        return ip;
                    }
                }
            }
        }
    }

    request
        .extensions()
        .get::<axum::extract::ConnectInfo<SocketAddr>>()
        .map(|info| info.0.ip())
        .unwrap_or(IpAddr::V4(Ipv4Addr::UNSPECIFIED))
}

/// Middleware: one bucket per visitor, `429` with `Retry-After` when spent.
///
/// Returns a `Response` rather than a `Result` so a refusal can be examined on the way out —
/// that is where an automatic ban gets its evidence.
pub async fn enforce_rate_limit(
    State(state): State<AppState>,
    request: Request,
    next: Next,
) -> Response {
    let ip = client_ip(&request, state.config.trust_forwarded_for);

    match state.limiter.check(ip) {
        Ok(()) => next.run(request).await,
        Err(retry_after) => {
            bans::note_violation(&state, ip, "rate");

            // Round up, so `Retry-After: 0` never means "retry immediately".
            AppError::RateLimited {
                retry_after_secs: retry_after.as_secs().max(1),
            }
            .into_response()
        }
    }
}
