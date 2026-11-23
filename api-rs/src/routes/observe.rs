//! Request logging, and the session cookie that makes the log add up to visits.
//!
//! `observe` is the outermost layer under tracing, and deliberately sits *outside* the panic
//! handler: a panicking handler must still produce a log row with a 500, which only works if
//! this layer sees the response the panic handler generated.
//!
//! Nothing here can fail a request. A record that cannot be queued is dropped and counted, and
//! a cookie that cannot be minted simply does not appear.

use std::time::Instant;

use axum::extract::{Request, State};
use axum::http::header;
use axum::middleware::Next;
use axum::response::{IntoResponse, Response};

use crate::error::AppError;
use crate::session::{self, MintedSession, SessionId};
use crate::state::AppState;
use crate::store::{Kind, RequestLog};

/// Longest user agent kept. Some are enormous, and a log column is not a place to store a
/// kilobyte of attacker-chosen text.
const MAX_UA: usize = 300;
const MAX_PATH: usize = 512;
const MAX_MESSAGE: usize = 1000;

/// Truncate on a character boundary, so multi-byte input cannot panic the middleware.
fn truncate(value: &str, limit: usize) -> String {
    if value.len() <= limit {
        return value.to_owned();
    }

    let mut end = limit;
    while end > 0 && !value.is_char_boundary(end) {
        end -= 1;
    }

    value[..end].to_owned()
}

pub async fn observe(State(state): State<AppState>, mut request: Request, next: Next) -> Response {
    let started = Instant::now();

    let ip = super::client_ip(&request, state.config.trust_forwarded_for);
    let incoming = session::from_headers(request.headers());
    let minted = incoming.is_none();
    let id = incoming.unwrap_or_else(session::mint);

    // Downstream middleware and handlers read the session from here, so the id in the log row,
    // the id in a `Set-Cookie`, and the id in a 428 are always the same one.
    request.extensions_mut().insert(SessionId(id.clone()));
    if minted {
        request.extensions_mut().insert(MintedSession(id.clone()));
    }

    let user_agent = request
        .headers()
        .get(header::USER_AGENT)
        .and_then(|value| value.to_str().ok())
        .map(|value| truncate(value, MAX_UA));
    let method = request.method().as_str().to_owned();
    let path = request.uri().path().to_owned();
    let kind = Kind::of_path(&path);

    let mut response = next.run(request).await;

    let status = response.status();
    let detail = response
        .extensions()
        .get::<crate::error::ErrorDetail>()
        .cloned();

    // Automatic bans need evidence from more than the rate limiter: a scanner walking ids
    // produces 404s, not 429s.
    if status.is_client_error() && status != axum::http::StatusCode::TOO_MANY_REQUESTS {
        super::bans::note_violation(&state, ip, "errors");
    }

    if should_log(kind, state.config.log_assets) {
        state.store.record(RequestLog {
            ts: crate::clock::now_unix(),
            duration_ms: started.elapsed().as_millis().min(u32::MAX as u128) as u32,
            ip: Some(ip.to_string()),
            session: Some(id.clone()),
            ua: user_agent,
            method,
            path: truncate(&path, MAX_PATH),
            status: status.as_u16(),
            code: detail.as_ref().map(|detail| detail.code),
            message: detail.map(|detail| truncate(&detail.message, MAX_MESSAGE)),
            kind,
            // Resolved by the writer, off the request path.
            country: None,
        });
    }

    if minted && wants_a_cookie(kind) && !response.headers().contains_key(header::SET_COOKIE) {
        response.headers_mut().insert(
            header::SET_COOKIE,
            session::cookie_header(&id, session::wants_secure(&state.config.public_origin)),
        );
    }

    response
}

/// Assets and health checks are cheap to skip and expensive to store: one page load would
/// otherwise be five rows. `LOG_ASSETS=1` when that trade is wrong.
fn should_log(kind: Kind, log_assets: bool) -> bool {
    match kind {
        Kind::Health => false,
        Kind::Asset => log_assets,
        _ => true,
    }
}

/// Share cards are fetched by unfurlers and assets by browsers. Neither needs a cookie, and
/// adding `Set-Cookie` to a card response is noise on an image.
fn wants_a_cookie(kind: Kind) -> bool {
    matches!(kind, Kind::Page | Kind::Api | Kind::Admin)
}

/// Require a session on the endpoints that cost an upstream call.
///
/// A refusal carries the cookie, so the caller can retry with it — `curl -c jar -b jar` works
/// on the second call, and the SPA never notices because it loaded a page first.
pub async fn require_session(
    State(state): State<AppState>,
    request: Request,
    next: Next,
) -> Response {
    let minted = request.extensions().get::<MintedSession>().cloned();

    let Some(MintedSession(id)) = minted else {
        return next.run(request).await;
    };

    let mut response = AppError::SessionRequired.into_response();
    response.headers_mut().insert(
        header::SET_COOKIE,
        session::cookie_header(&id, session::wants_secure(&state.config.public_origin)),
    );

    response
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn truncation_never_splits_a_character() {
        // A four-byte emoji straddling the limit must not produce a broken slice.
        let value = "aaaaaaaaaa🦀bbbb";
        let cut = truncate(value, 12);
        assert!(cut.len() <= 12);
        assert!(value.starts_with(&cut));
    }

    #[test]
    fn short_values_are_untouched() {
        assert_eq!(truncate("hello", 10), "hello");
        assert_eq!(truncate("hello", 5), "hello");
    }

    #[test]
    fn assets_and_health_are_not_logged_by_default() {
        assert!(!should_log(Kind::Asset, false));
        assert!(!should_log(Kind::Health, false));
        assert!(should_log(Kind::Asset, true));
        assert!(should_log(Kind::Api, false));
        assert!(should_log(Kind::Page, false));
        assert!(should_log(Kind::Card, false));
    }

    #[test]
    fn never_set_a_cookie_on_an_image_or_a_probe() {
        assert!(!wants_a_cookie(Kind::Card));
        assert!(!wants_a_cookie(Kind::Asset));
        assert!(!wants_a_cookie(Kind::Health));
        assert!(wants_a_cookie(Kind::Page));
        assert!(wants_a_cookie(Kind::Api));
    }
}
