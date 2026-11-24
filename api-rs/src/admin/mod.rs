//! The admin API: authentication, and the routes it protects.
//!
//! Two ways in, because there are two callers. A script sends `Authorization: Bearer <token>`
//! and nothing else. The dashboard posts the token once to `/api/admin/session`, which sets an
//! `HttpOnly` cookie and never hands the token back — so an XSS in the dashboard cannot lift a
//! permanent credential out of `localStorage`.
//!
//! The cookie holds a random id, not the token, and the ids live in memory. Restarting the
//! process signs everyone out, which is the correct default for a single-operator service.

pub mod api;
pub mod pages;

use std::collections::HashMap;
use std::sync::Mutex;

use axum::extract::{Request, State};
use axum::http::{header, HeaderValue, StatusCode};
use axum::middleware::Next;
use axum::response::{IntoResponse, Response};
use axum::routing::{delete, get, post};
use axum::{Json, Router};
use serde_json::json;

use crate::state::AppState;

pub const COOKIE: &str = "sa_admin";
/// 12 hours: long enough for a working session, short enough that a stale browser tab does not
/// stay privileged for a week.
pub const SESSION_TTL: i64 = 12 * 60 * 60;

#[derive(Default)]
pub struct AdminAuth {
    /// `None` means the admin API is disabled entirely — a forgotten env var must not mean an
    /// open dashboard.
    token: Option<String>,
    sessions: Mutex<HashMap<String, i64>>,
}

impl AdminAuth {
    pub fn new(token: Option<String>) -> Self {
        let token = token
            .map(|token| token.trim().to_owned())
            .filter(|t| !t.is_empty());
        Self {
            token,
            sessions: Mutex::new(HashMap::new()),
        }
    }

    pub fn enabled(&self) -> bool {
        self.token.is_some()
    }

    /// Check a presented token. Compared without short-circuiting so the comparison does not
    /// leak how much of the token matched. Length is compared directly, which reveals only the
    /// length — not a secret worth protecting.
    pub fn token_matches(&self, presented: &str) -> bool {
        let Some(expected) = self.token.as_deref() else {
            return false;
        };

        let a = expected.as_bytes();
        let b = presented.as_bytes();

        if a.len() != b.len() {
            return false;
        }

        let mut difference = 0u8;
        for (x, y) in a.iter().zip(b) {
            difference |= x ^ y;
        }

        difference == 0
    }

    pub fn open_session(&self) -> String {
        let id = crate::session::mint();
        self.sessions
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .insert(id.clone(), crate::clock::now_unix() + SESSION_TTL);
        id
    }

    pub fn session_valid(&self, id: &str) -> bool {
        let now = crate::clock::now_unix();
        let mut sessions = self
            .sessions
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());

        // Sweep on read: this map has one entry per login and would otherwise only ever grow.
        sessions.retain(|_, expires| *expires > now);

        sessions.contains_key(id)
    }

    pub fn close_all(&self) {
        self.sessions
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .clear();
    }
}

fn bearer(request: &Request) -> Option<&str> {
    request
        .headers()
        .get(header::AUTHORIZATION)?
        .to_str()
        .ok()?
        .strip_prefix("Bearer ")
        .map(str::trim)
}

/// The session id from a request's cookies, for the page guard as well as the API.
pub fn cookie_session(headers: &axum::http::HeaderMap) -> Option<String> {
    let header = headers.get(header::COOKIE)?.to_str().ok()?;

    for cookie in header.split(';') {
        let Some((name, value)) = cookie.trim().split_once('=') else {
            continue;
        };
        if name.trim() == COOKIE {
            let value = value.trim();
            return crate::session::is_valid(value).then(|| value.to_owned());
        }
    }

    None
}

fn admin_cookie(request: &Request) -> Option<String> {
    let header = request.headers().get(header::COOKIE)?.to_str().ok()?;

    for cookie in header.split(';') {
        let Some((name, value)) = cookie.trim().split_once('=') else {
            continue;
        };
        if name.trim() == COOKIE {
            let value = value.trim();
            return crate::session::is_valid(value).then(|| value.to_owned());
        }
    }

    None
}

/// Refuse anything without a valid credential. When the API is disabled, answer `404` — the same
/// answer as a route that does not exist, because that is what it is.
pub async fn authorize(State(state): State<AppState>, request: Request, next: Next) -> Response {
    if !state.admin.enabled() {
        return (
            StatusCode::NOT_FOUND,
            Json(json!({
                "error": { "code": "not_found", "message": "Not found." }
            })),
        )
            .into_response();
    }

    let authorized = bearer(&request).is_some_and(|token| state.admin.token_matches(token))
        || admin_cookie(&request).is_some_and(|id| state.admin.session_valid(&id));

    if !authorized {
        return (
            StatusCode::UNAUTHORIZED,
            [(
                header::WWW_AUTHENTICATE,
                HeaderValue::from_static("Bearer realm=\"same.actor admin\""),
            )],
            Json(json!({
                "error": { "code": "unauthorized", "message": "A valid admin token is required." }
            })),
        )
            .into_response();
    }

    next.run(request).await
}

/// The routes that require a credential. `/session` is deliberately not among them: presenting
/// the token is what it does.
/// The login route. Deliberately reachable without a credential — presenting the token is what
/// it does. It performs the same constant-time comparison as [`authorize`].
pub fn open_router() -> Router<AppState> {
    Router::new().route("/session", post(api::session))
}

/// Everything that requires a credential. `/api/admin/session` is not among these; nesting it
/// behind the guard would make logging in impossible.
pub fn guarded_router() -> Router<AppState> {
    Router::new()
        .route("/overview", get(api::overview))
        .route("/logs", get(api::logs))
        .route("/logs/{id}", get(api::log))
        .route("/stats/timeseries", get(api::timeseries))
        .route("/stats/geo", get(api::geo))
        .route("/stats/searches", get(api::searches))
        .route("/stats/comparisons", get(api::comparisons))
        .route("/suspicious", get(api::suspicious))
        .route("/bans", get(api::bans).post(api::create_ban))
        .route("/bans/{id}", delete(api::delete_ban))
        .route("/health", get(api::health))
        // Everything here is live data and must never be cached by a proxy or a browser.
        .layer(axum::middleware::from_fn(no_store))
}

pub fn admin_cookie_header(id: &str, secure: bool) -> HeaderValue {
    let mut value =
        format!("{COOKIE}={id}; Path=/; HttpOnly; SameSite=Strict; Max-Age={SESSION_TTL}");
    if secure {
        value.push_str("; Secure");
    }
    HeaderValue::from_str(&value)
        .unwrap_or_else(|_| HeaderValue::from_static("sa_admin=; Max-Age=0"))
}

pub fn router(state: AppState) -> Router<AppState> {
    Router::new()
        .route("/overview", get(api::overview))
        .route("/logs", get(api::logs))
        .route("/logs/{id}", get(api::log))
        .route("/stats/timeseries", get(api::timeseries))
        .route("/stats/geo", get(api::geo))
        .route("/stats/searches", get(api::searches))
        .route("/stats/comparisons", get(api::comparisons))
        .route("/suspicious", get(api::suspicious))
        .route("/bans", get(api::bans).post(api::create_ban))
        .route("/bans/{id}", delete(api::delete_ban))
        .route("/health", get(api::health))
        // Everything here is live data and must never be cached by a proxy or a browser.
        .layer(axum::middleware::from_fn_with_state(state, no_store))
}

pub async fn no_store(request: Request, next: Next) -> Response {
    let mut response = next.run(request).await;
    response
        .headers_mut()
        .insert(header::CACHE_CONTROL, HeaderValue::from_static("no-store"));
    response
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_correct_token_matches_and_a_wrong_one_does_not() {
        let auth = AdminAuth::new(Some("correct-horse-battery-staple".into()));
        assert!(auth.token_matches("correct-horse-battery-staple"));
        assert!(!auth.token_matches("correct-horse-battery-stapl"));
        assert!(!auth.token_matches("correct-horse-battery-staple "));
        assert!(!auth.token_matches(""));
    }

    #[test]
    fn an_unset_or_blank_token_disables_the_api() {
        assert!(!AdminAuth::new(None).enabled());
        assert!(!AdminAuth::new(Some("   ".into())).enabled());
        assert!(!AdminAuth::new(Some(String::new())).enabled());
    }

    #[test]
    fn a_disabled_api_never_matches_anything() {
        let auth = AdminAuth::new(None);
        assert!(!auth.token_matches(""));
        assert!(!auth.token_matches("anything"));
    }

    #[test]
    fn a_session_is_valid_until_it_expires() {
        let auth = AdminAuth::new(Some("t".into()));
        let id = auth.open_session();
        assert!(auth.session_valid(&id));
        assert!(!auth.session_valid("not-a-session"));
    }

    #[test]
    fn sessions_can_be_closed_all_at_once() {
        let auth = AdminAuth::new(Some("t".into()));
        let id = auth.open_session();
        auth.close_all();
        assert!(!auth.session_valid(&id));
    }

    #[test]
    fn the_cookie_is_http_only_strict_and_short_lived() {
        let cookie = admin_cookie_header("aaaaaaaaaaaaaaaaaaaaaa", true)
            .to_str()
            .unwrap()
            .to_owned();
        assert!(cookie.contains("HttpOnly"));
        assert!(
            cookie.contains("SameSite=Strict"),
            "an admin cookie must not ride cross-site requests"
        );
        assert!(cookie.contains("Secure"));
        assert!(cookie.contains("Max-Age=43200"));
    }
}
