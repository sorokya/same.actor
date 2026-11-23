//! Ban enforcement, and the automatic bans that come from abuse rules.

use axum::extract::{Request, State};
use axum::http::header;
use axum::http::header::HeaderValue;
use axum::middleware::Next;
use axum::response::{IntoResponse, Response};

use crate::error::AppError;
use crate::state::AppState;

/// Refuse a banned client before it reaches anything else.
///
/// Before the rate limit on purpose: a banned client should be told it is banned, not told to
/// come back in a second. `/healthz` is exempt — a ban must never be able to make the container
/// look unhealthy.
pub async fn enforce_bans(State(state): State<AppState>, request: Request, next: Next) -> Response {
    let path = request.uri().path();

    // Two exemptions, for different reasons. `/healthz` because a ban must never be able to make
    // the container look unhealthy. `/api/admin` because an operator who bans their own address
    // would otherwise lock themselves out of the only screen that can undo it — the admin
    // credential is the gate there, and it is a stronger one than an address.
    if path == "/healthz" || path.starts_with("/api/admin") {
        return next.run(request).await;
    }

    let ip = super::client_ip(&request, state.config.trust_forwarded_for);

    let Some(ban) = state.bans.find(ip) else {
        return next.run(request).await;
    };

    // Fire and forget: counting the hit must not delay the refusal.
    state.store.ban_hit(ban.id);

    let mut response = AppError::Banned(ban.reason.unwrap_or_default()).into_response();

    // A temporary ban tells the client when to try again, which is the difference between a
    // puzzling 403 and a comprehensible one.
    if let Some(expires_ts) = ban.expires_ts {
        let remaining = (expires_ts - crate::clock::now_unix()).max(0);
        if let Ok(value) = HeaderValue::from_str(&remaining.to_string()) {
            response.headers_mut().insert(header::RETRY_AFTER, value);
        }
    }

    response
}

/// Give an automatic rule the chance to ban an address that keeps violating it.
///
/// Called from the rate limiter (for 429s) and from the logger (for other client errors). The
/// ban is created in the background so the response is not delayed by a write.
pub fn note_violation(state: &AppState, ip: std::net::IpAddr, rule: &'static str) {
    let tracker = match rule {
        "rate" => &state.autoban_rate,
        _ => &state.autoban_errors,
    };

    if !state.bans.find(ip).is_none() {
        // Already banned: re-banning on every violation would be a write per request.
        return;
    }

    if !tracker.note(ip) {
        return;
    }

    let store = std::sync::Arc::clone(&state.store);
    let cache = std::sync::Arc::clone(&state.bans);
    let expires_ts =
        crate::clock::now_unix() + state.config.autoban_ban_secs.min(i64::MAX as u64) as i64;

    tokio::spawn(async move {
        let ban = crate::store::bans::NewBan {
            target: ip.to_string(),
            kind: "ip".into(),
            reason: Some(format!("automatic: {rule} limit exceeded")),
            created_by: format!("auto:{rule}"),
            expires_ts: Some(expires_ts),
        };

        match crate::bans::apply(&store, &cache, ban).await {
            Ok(id) => tracing::info!(%ip, rule, id, "automatic ban applied"),
            Err(error) => tracing::warn!(%ip, rule, %error, "could not apply an automatic ban"),
        }
    });
}
