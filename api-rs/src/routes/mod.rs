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

pub mod findout;
pub mod og;
pub mod search;

pub use findout::findout;
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
pub async fn enforce_rate_limit(
    State(state): State<AppState>,
    request: Request,
    next: Next,
) -> Result<Response, AppError> {
    let ip = client_ip(&request, state.config.trust_forwarded_for);

    state
        .limiter
        .check(ip)
        .map_err(|retry_after| AppError::RateLimited {
            // Round up, so `Retry-After: 0` never means "retry immediately".
            retry_after_secs: retry_after.as_secs().max(1),
        })?;

    Ok(next.run(request).await)
}
