//! TMDB client: URL construction, auth, timeout, caching.
//!
//! Every URL is assembled with [`url::Url`] path segments and query pairs, so a value can
//! never escape its position in the path. Combined with the closed enums in [`crate::domain`],
//! the review's C1 finding is unrepresentable rather than merely guarded against.

pub mod types;

use std::sync::Arc;
use std::time::Duration;

use moka::future::Cache;
use reqwest::StatusCode;
use reqwest::Url;
use serde::de::DeserializeOwned;
use serde_json::Value;

use crate::config::{Config, TmdbAuth};
use crate::domain::{year_from_date, Kind, Mode, SearchResult, Selection, SelectionMeta};
use crate::error::AppError;
use crate::intersect::{PersonClaim, TitleClaim};

const LANGUAGE: &str = "en-US";

pub struct Tmdb {
    client: reqwest::Client,
    base: Url,
    image_base: String,
    token: String,
    auth: TmdbAuth,
    cache: Cache<String, Arc<Value>>,
    /// Raster images (posters, portraits) as fetched. Separate from `cache` because these
    /// are bytes from the image CDN, not JSON, and they are much bigger.
    image_cache: Cache<String, Arc<Vec<u8>>>,
}

/// Largest image worth keeping in memory. TMDB's `w185` thumbnails are tens of kilobytes;
/// anything past this is not a poster.
const MAX_IMAGE_BYTES: usize = 3 * 1024 * 1024;

impl Tmdb {
    pub fn new(config: &Config) -> Result<Self, AppError> {
        let base = Url::parse(&config.tmdb_base)
            .map_err(|e| AppError::Config(format!("TMDB_BASE is not a URL: {e}")))?;

        let client = reqwest::Client::builder()
            .connect_timeout(Duration::from_secs(5))
            .timeout(Duration::from_secs(10))
            .user_agent(concat!("same-actor/", env!("CARGO_PKG_VERSION")))
            .build()
            .map_err(|e| AppError::Config(format!("could not build the HTTP client: {e}")))?;

        let cache = Cache::builder()
            .max_capacity(config.cache_capacity)
            .time_to_live(config.cache_ttl)
            .build();

        let image_cache = Cache::builder()
            .max_capacity(512)
            .time_to_live(Duration::from_secs(6 * 60 * 60))
            .build();

        Ok(Self {
            client,
            base,
            image_base: config.image_base.clone(),
            token: config.tmdb_token.clone(),
            auth: config.tmdb_auth,
            cache,
            image_cache,
        })
    }

    /// Fetch an image from the CDN.
    ///
    /// Deliberately unauthenticated: the TMDB token belongs to the API host, not the image
    /// host, and sending it there would leak a credential to a third party for no reason.
    /// Any failure returns `None` — a missing poster must cost the card its picture, never
    /// the card itself.
    pub async fn fetch_image(&self, url: &str) -> Option<Arc<Vec<u8>>> {
        if let Some(hit) = self.image_cache.get(url).await {
            return Some(hit);
        }

        let response = self.client.get(url).send().await.ok()?;
        if !response.status().is_success() {
            tracing::debug!(%url, status = %response.status(), "image fetch failed");
            return None;
        }

        if let Some(length) = response.content_length() {
            if length as usize > MAX_IMAGE_BYTES {
                tracing::warn!(%url, length, "image too large; skipping");
                return None;
            }
        }

        let bytes = response.bytes().await.ok()?;
        if bytes.len() > MAX_IMAGE_BYTES {
            return None;
        }

        let bytes = Arc::new(bytes.to_vec());
        self.image_cache
            .insert(url.to_owned(), Arc::clone(&bytes))
            .await;
        Some(bytes)
    }

    fn image(&self, path: Option<&str>) -> Option<String> {
        let path = path?.trim();
        if path.is_empty() {
            return None;
        }
        // TMDB paths always start with '/'. Guard anyway; a missing slash is a broken URL.
        let path = if path.starts_with('/') {
            path.to_owned()
        } else {
            format!("/{path}")
        };
        Some(format!("{}{}", self.image_base.trim_end_matches('/'), path))
    }

    /// Build the request URL and the cache key.
    ///
    /// The cache key deliberately excludes the credential, so the token never sits in a map
    /// key where a heap dump or a debug log could find it.
    fn build(&self, segments: &[&str], query: &[(&str, &str)]) -> Result<(Url, String), AppError> {
        let mut url = self.base.clone();

        {
            let mut path = url
                .path_segments_mut()
                .map_err(|_| AppError::Config("TMDB_BASE cannot be a base URL".into()))?;
            for segment in segments {
                path.push(segment);
            }
        }

        let mut key = url.path().to_owned();
        {
            let mut pairs = url.query_pairs_mut();
            for (k, v) in query {
                pairs.append_pair(k, v);
            }
        }
        key.push('?');
        key.push_str(url.query().unwrap_or_default());

        if self.auth == TmdbAuth::Query {
            url.query_pairs_mut().append_pair("api_key", &self.token);
        }

        Ok((url, key))
    }

    async fn get<T: DeserializeOwned>(
        &self,
        segments: &[&str],
        query: &[(&str, &str)],
    ) -> Result<T, AppError> {
        let (url, key) = self.build(segments, query)?;

        let value = if let Some(hit) = self.cache.get(&key).await {
            tracing::debug!(key = %key, "cache hit");
            (*hit).clone()
        } else {
            let mut request = self.client.get(url);
            if self.auth == TmdbAuth::Bearer {
                request = request.bearer_auth(&self.token);
            }

            let response = request
                .send()
                .await
                .map_err(|e| AppError::Upstream(format!("{key}: {e}")))?;

            let status = response.status();
            match status {
                s if s.is_success() => {}
                StatusCode::NOT_FOUND => return Err(AppError::NotFound),
                StatusCode::TOO_MANY_REQUESTS => {
                    let retry_after = retry_after(&response).unwrap_or(2);
                    return Err(AppError::RateLimited {
                        retry_after_secs: retry_after,
                    });
                }
                _ => return Err(AppError::Upstream(format!("{key}: status {status}"))),
            }

            let value: Value = response
                .json()
                .await
                .map_err(|e| AppError::Upstream(format!("{key}: malformed JSON: {e}")))?;

            self.cache
                .insert(key.clone(), Arc::new(value.clone()))
                .await;
            value
        };

        serde_json::from_value(value)
            .map_err(|e| AppError::Upstream(format!("{key}: unexpected schema: {e}")))
    }

    pub async fn search(&self, mode: Mode, term: &str) -> Result<Vec<SearchResult>, AppError> {
        match mode {
            Mode::Title => {
                let data: types::SearchMulti = self
                    .get(
                        &["search", "multi"],
                        &[
                            ("query", term),
                            ("include_adult", "false"),
                            ("language", LANGUAGE),
                            ("page", "1"),
                        ],
                    )
                    .await?;

                Ok(data
                    .results
                    .into_iter()
                    .filter_map(|r| {
                        let kind = match r.media_type.as_deref() {
                            Some("movie") => Kind::Movie,
                            Some("tv") => Kind::Tv,
                            _ => return None,
                        };
                        let name = r.title.clone().or_else(|| r.name.clone())?;
                        let year = year_from_date(
                            r.release_date.as_deref().or(r.first_air_date.as_deref()),
                        );

                        Some(SearchResult {
                            id: r.id,
                            kind,
                            name,
                            year,
                            overview: r.overview.unwrap_or_default(),
                            image: self.image(r.poster_path.as_deref()),
                        })
                    })
                    .collect())
            }
            Mode::Person => {
                let data: types::SearchMulti = self
                    .get(
                        &["search", "person"],
                        &[
                            ("query", term),
                            ("include_adult", "false"),
                            ("language", LANGUAGE),
                            ("page", "1"),
                        ],
                    )
                    .await?;

                Ok(data
                    .results
                    .into_iter()
                    .filter_map(|r| {
                        let name = r.name.clone()?;
                        Some(SearchResult {
                            id: r.id,
                            kind: Kind::Person,
                            name,
                            year: None,
                            overview: known_for(&r.known_for),
                            image: self.image(r.profile_path.as_deref()),
                        })
                    })
                    .collect())
            }
        }
    }

    /// Cast of one selected title. Used when comparing titles.
    ///
    /// Returns the title's own metadata as well as its cast, from a single upstream call.
    pub async fn title_selection(
        &self,
        selection: Selection,
    ) -> Result<(SelectionMeta, Vec<PersonClaim>), AppError> {
        let id = selection.id.get().to_string();

        let detail: types::MediaDetail = match selection.kind {
            Kind::Movie => {
                self.get(
                    &["movie", id.as_str()],
                    &[("append_to_response", "credits"), ("language", LANGUAGE)],
                )
                .await?
            }
            Kind::Tv => {
                self.get(
                    &["tv", id.as_str()],
                    &[
                        ("append_to_response", "aggregate_credits"),
                        ("language", LANGUAGE),
                    ],
                )
                .await?
            }
            // Unreachable: selection kinds are validated against the mode before we get here.
            Kind::Person => {
                return Err(AppError::Validation(
                    "a person cannot be compared as a title".into(),
                ))
            }
        };

        let meta = SelectionMeta {
            id: detail.id,
            kind: selection.kind,
            year: detail.year(),
            image: self.image(detail.poster_path.as_deref()),
            name: detail.display_name(),
        };

        let claims = detail
            .cast()
            .iter()
            .filter_map(|c| {
                let name = c.name.clone()?;
                Some(PersonClaim {
                    id: c.id,
                    name,
                    image: self.image(c.profile_path.as_deref()),
                    characters: c.characters(),
                })
            })
            .collect();

        Ok((meta, claims))
    }

    /// Filmography of one selected person. Used when comparing people.
    pub async fn person_selection(
        &self,
        selection: Selection,
    ) -> Result<(SelectionMeta, Vec<TitleClaim>), AppError> {
        let id = selection.id.get().to_string();

        let detail: types::PersonDetail = self
            .get(
                &["person", id.as_str()],
                &[
                    ("append_to_response", "combined_credits"),
                    ("language", LANGUAGE),
                ],
            )
            .await?;

        let meta = SelectionMeta {
            id: detail.id,
            kind: Kind::Person,
            year: None,
            image: self.image(detail.profile_path.as_deref()),
            name: detail
                .name
                .clone()
                .filter(|n| !n.trim().is_empty())
                .unwrap_or_else(|| format!("#{}", detail.id)),
        };

        let claims: Vec<TitleClaim> = detail
            .combined_credits
            .map(|c| c.cast)
            .unwrap_or_default()
            .into_iter()
            .filter_map(|c| {
                let kind = match c.media_type.as_deref() {
                    Some("movie") => Kind::Movie,
                    Some("tv") => Kind::Tv,
                    _ => return None,
                };
                let name = c.title.clone().or_else(|| c.name.clone())?;
                let year =
                    year_from_date(c.release_date.as_deref().or(c.first_air_date.as_deref()));

                Some(TitleClaim {
                    id: c.id,
                    kind,
                    name,
                    year,
                    image: self.image(c.poster_path.as_deref()),
                    character: c.character.filter(|s| !s.trim().is_empty()),
                })
            })
            .collect();

        Ok((meta, claims))
    }
}

fn retry_after(response: &reqwest::Response) -> Option<u64> {
    response
        .headers()
        .get(reqwest::header::RETRY_AFTER)?
        .to_str()
        .ok()?
        .trim()
        .parse()
        .ok()
}

/// The old code produced `"Known for: "` with a dangling label when `known_for` was empty,
/// and `"Known for: undefined (NaN)"` when an entry had no title.
fn known_for(items: &[types::MultiResult]) -> String {
    let entries: Vec<String> = items
        .iter()
        .filter_map(|i| {
            let name = i.title.as_deref().or(i.name.as_deref())?.trim();
            if name.is_empty() {
                return None;
            }
            Some(
                match year_from_date(i.release_date.as_deref().or(i.first_air_date.as_deref())) {
                    Some(year) => format!("{name} ({year})"),
                    None => name.to_owned(),
                },
            )
        })
        .collect();

    if entries.is_empty() {
        String::new()
    } else {
        format!("Known for: {}", entries.join(", "))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tmdb() -> Tmdb {
        let config = Config {
            bind: "127.0.0.1:0".parse().unwrap(),
            tmdb_token: "secret-token-value".into(),
            tmdb_auth: TmdbAuth::Bearer,
            tmdb_base: "https://api.themoviedb.org/3".into(),
            image_base: "https://image.tmdb.org/t/p/w342".into(),
            static_dir: "dist".into(),
            db_path: ":memory:".into(),
            log_retention_days: 90,
            log_assets: false,
            geo_enabled: false,
            geo_db_path: "unused.mmdb".into(),
            geo_url: "https://example.invalid/{month}.mmdb.gz".into(),
            public_origin: "http://localhost:3001".into(),
            tmdb_logo_url: "https://www.themoviedb.org/assets/v4/logos/v2/blue_short.svg".into(),
            cache_ttl: Duration::from_secs(60),
            cache_capacity: 10,
            rate_limit: 10,
            rate_window: Duration::from_secs(1),
            trust_forwarded_for: false,
        };
        Tmdb::new(&config).unwrap()
    }

    #[test]
    fn path_segments_cannot_escape_their_position() {
        let tmdb = tmdb();
        let selection = Selection {
            id: std::num::NonZeroU32::new(603).unwrap(),
            kind: Kind::Movie,
        };
        let (url, _) = tmdb
            .build(
                &["movie", &selection.id.get().to_string(), "credits"],
                &[("language", LANGUAGE)],
            )
            .unwrap();
        assert_eq!(url.path(), "/3/movie/603/credits");
    }

    #[test]
    fn the_credential_is_not_in_the_cache_key() {
        let mut config_tmdb = tmdb();
        config_tmdb.auth = TmdbAuth::Query;
        let (url, key) = config_tmdb
            .build(&["search", "multi"], &[("query", "the matrix")])
            .unwrap();

        assert!(
            url.as_str().contains("api_key="),
            "the request must carry the key"
        );
        assert!(
            !key.contains("secret-token-value"),
            "the cache key must not: {key}"
        );
    }

    #[test]
    fn query_values_are_percent_encoded() {
        let tmdb = tmdb();
        let (url, _) = tmdb
            .build(&["search", "multi"], &[("query", "a&b=c/d")])
            .unwrap();
        assert!(url.as_str().contains("query=a%26b%3Dc%2Fd"));
    }

    #[test]
    fn images_are_only_built_from_real_paths() {
        let tmdb = tmdb();
        assert_eq!(
            tmdb.image(Some("/abc.jpg")).as_deref(),
            Some("https://image.tmdb.org/t/p/w342/abc.jpg")
        );
        assert_eq!(tmdb.image(Some("")), None);
        assert_eq!(tmdb.image(None), None);
    }

    #[test]
    fn known_for_is_empty_rather_than_a_dangling_label() {
        assert_eq!(known_for(&[]), "");

        let parsed: types::MultiResult =
            serde_json::from_str(r#"{"id":1,"title":"The Matrix","release_date":"1999-03-31"}"#)
                .unwrap();
        assert_eq!(known_for(&[parsed]), "Known for: The Matrix (1999)");

        let undated: types::MultiResult =
            serde_json::from_str(r#"{"id":2,"name":"Some Show"}"#).unwrap();
        assert_eq!(known_for(&[undated]), "Known for: Some Show");
    }
}
