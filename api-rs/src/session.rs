//! The visitor's session cookie.
//!
//! `sa_sid` is a random 16-byte id, base64url, stored `HttpOnly`, `SameSite=Lax`, 30 days. It
//! is not authentication and never gates data — it groups requests into a visit so the log can
//! answer "how many people" rather than "how many requests", and so a rate limit has something
//! to key on besides an address that a household or a phone network shares.
//!
//! Values are validated on the way in. A cookie is attacker-controlled text that ends up in a
//! `Set-Cookie` header and a database column, so anything outside the alphabet this program
//! mints is discarded rather than escaped.

use axum::http::{header, HeaderMap, HeaderValue};

pub const COOKIE: &str = "sa_sid";

/// 16 bytes of base64url without padding.
pub const LEN: usize = 22;

/// 30 days, in seconds.
pub const MAX_AGE: i64 = 2_592_000;

/// A session id, present on every request after the first: either the one the client sent or
/// one minted for this request.
#[derive(Debug, Clone)]
pub struct SessionId(pub String);

/// Present only when the request arrived without a cookie and one was minted for it. The
/// session-required middleware uses this as the "you had no session" signal, so the id in the
/// `Set-Cookie` it returns is the same one the log row carries.
#[derive(Debug, Clone)]
pub struct MintedSession(pub String);

/// A fresh id. Falls back to a time-derived value if the OS will not give us randomness —
/// a predictable session id is worse for grouping but never a security boundary here, and
/// refusing to serve the site because `/dev/urandom` is unavailable would be absurd.
pub fn mint() -> String {
    let mut bytes = [0u8; 16];

    if getrandom::fill(&mut bytes).is_err() {
        tracing::warn!("no system randomness available; session ids will not be unique");
        let now = crate::clock::now_unix() as u128;
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.subsec_nanos())
            .unwrap_or(0);
        bytes[..8].copy_from_slice(&now.to_le_bytes()[..8]);
        bytes[8..12].copy_from_slice(&nanos.to_le_bytes());
    }

    use base64::Engine;
    base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(bytes)
}

/// True when `value` is shaped exactly like something [`mint`] produced.
pub fn is_valid(value: &str) -> bool {
    value.len() == LEN
        && value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || byte == b'-' || byte == b'_')
}

/// Read a session id from the request's cookies. Invalid values are treated as absent.
pub fn from_headers(headers: &HeaderMap) -> Option<String> {
    let header = headers.get(header::COOKIE)?.to_str().ok()?;

    for cookie in header.split(';') {
        let cookie = cookie.trim();
        let Some((name, value)) = cookie.split_once('=') else {
            continue;
        };

        if name.trim() == COOKIE {
            let value = value.trim();
            return is_valid(value).then(|| value.to_owned());
        }
    }

    None
}

/// The `Set-Cookie` value for a session id.
pub fn cookie_header(id: &str, secure: bool) -> HeaderValue {
    let mut value = format!("{COOKIE}={id}; Path=/; HttpOnly; SameSite=Lax; Max-Age={MAX_AGE}");

    if secure {
        value.push_str("; Secure");
    }

    // HeaderValue::from_str fails only on control characters, which `is_valid` already
    // excluded. A fallback that omits the cookie beats a panic in a middleware.
    HeaderValue::from_str(&value).unwrap_or_else(|_| HeaderValue::from_static("sa_sid=; Max-Age=0"))
}

/// Whether cookies should carry `Secure`: true only when the site is actually served over
/// https, because a `Secure` cookie on a plain-http origin is silently dropped.
pub fn wants_secure(public_origin: &str) -> bool {
    public_origin.starts_with("https://")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn headers_with(value: &str) -> HeaderMap {
        let mut headers = HeaderMap::new();
        headers.insert(
            header::COOKIE,
            HeaderValue::from_str(value).expect("test header"),
        );
        headers
    }

    #[test]
    fn a_minted_id_is_valid() {
        let id = mint();
        assert!(is_valid(&id), "minted {id} does not pass its own validator");
        assert_eq!(id.len(), LEN);
    }

    #[test]
    fn minted_ids_do_not_repeat() {
        let mut seen = std::collections::HashSet::new();
        for _ in 0..1_000 {
            assert!(
                seen.insert(mint()),
                "a minted id repeated within 1000 draws"
            );
        }
    }

    #[test]
    fn the_cookie_is_found_among_others() {
        let headers = headers_with("theme=dark; sa_sid=aaaaaaaaaaaaaaaaaaaaaa; other=1");
        assert_eq!(
            from_headers(&headers),
            Some("aaaaaaaaaaaaaaaaaaaaaa".to_owned())
        );
    }

    #[test]
    fn a_missing_or_malformed_cookie_is_absent() {
        assert_eq!(from_headers(&HeaderMap::new()), None);
        assert_eq!(from_headers(&headers_with("theme=dark")), None);
        assert_eq!(from_headers(&headers_with("sa_sid=")), None);
        assert_eq!(from_headers(&headers_with("sa_sid=short")), None);
        // A value that would break out of a Set-Cookie header.
        assert_eq!(
            from_headers(&headers_with(
                "sa_sid=aaaaaaaaaaaaaaaaaaaaaa; Domain=evil.test"
            )),
            Some("aaaaaaaaaaaaaaaaaaaaaa".to_owned())
        );
        assert_eq!(
            from_headers(&headers_with("sa_sid=aaaaaaaaaa;aaa=bbbbbbbbbbbb")),
            None
        );
    }

    #[test]
    fn control_characters_are_never_accepted() {
        assert!(!is_valid("aaaaaaaaaaaaaaaaaaaaa\n"));
        assert!(!is_valid("aaaaaaaaaa\r\naaaaaaaa"));
        assert!(!is_valid("aaaaaaaaaaaaaaaaaaaaa;"));
    }

    #[test]
    fn secure_is_only_set_on_an_https_origin() {
        let secure = cookie_header("aaaaaaaaaaaaaaaaaaaaaa", true);
        assert!(secure.to_str().unwrap().contains("Secure"));
        assert!(secure.to_str().unwrap().contains("HttpOnly"));
        assert!(secure.to_str().unwrap().contains("SameSite=Lax"));

        let plain = cookie_header("aaaaaaaaaaaaaaaaaaaaaa", false);
        assert!(!plain.to_str().unwrap().contains("Secure"));
    }

    #[test]
    fn https_origins_ask_for_secure_cookies() {
        assert!(wants_secure("https://same.actor"));
        assert!(!wants_secure("http://localhost:3001"));
    }
}
