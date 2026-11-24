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

/// TMDB's page size for list endpoints. A short page means the end of the list.
const PAGE_SIZE: usize = 20;

/// The floor between two upstream requests, across the whole process.
///
/// TMDB's published guidance is an upper limit "somewhere in the 40 requests per second range",
/// with the explicit request that a caller respects a 429 rather than probe for the edge. 45ms is
/// about 22 a second — half the stated ceiling, which leaves room for a retry burst and keeps a
/// hundred-title refresh from looking like bulk scraping.
const MIN_REQUEST_INTERVAL: Duration = Duration::from_millis(45);

/// How many times one request may be attempted before the caller hears about it.
const MAX_ATTEMPTS: u32 = 4;

/// What to do about a response.
#[derive(Debug, PartialEq, Eq)]
enum Next {
    /// Read the body.
    Use,
    /// Wait, then try again.
    Retry(Duration),
    /// Give up and tell the caller why.
    Fail(Status),
}

#[derive(Debug, PartialEq, Eq)]
enum Status {
    NotFound,
    RateLimited(u64),
    Upstream,
}

/// The retry decision, as a pure function of the status and the attempt number.
///
/// Split out so the policy can be tested without a network: a 429 must be retried after its
/// `Retry-After`, a 5xx after a backoff, and a 4xx that is not 429 must never be retried — asking
/// again for something TMDB says does not exist is how a polite client becomes a rude one.
fn on_status(status: StatusCode, retry_after: Option<u64>, attempt: u32) -> Next {
    if status.is_success() {
        return Next::Use;
    }

    let last = attempt >= MAX_ATTEMPTS;

    if status == StatusCode::TOO_MANY_REQUESTS {
        let wait = Duration::from_secs(retry_after.unwrap_or(2));
        return if last {
            Next::Fail(Status::RateLimited(wait.as_secs()))
        } else {
            Next::Retry(wait)
        };
    }

    if status.is_server_error() {
        return if last {
            Next::Fail(Status::Upstream)
        } else {
            Next::Retry(backoff(attempt))
        };
    }

    if status == StatusCode::NOT_FOUND {
        return Next::Fail(Status::NotFound);
    }

    Next::Fail(Status::Upstream)
}

/// Exponential backoff with jitter: 250ms, 500ms, 1s, capped at four seconds.
///
/// The jitter is not decoration. A hundred titles fail together when an upstream does, and
/// without it they all come back at the same instant and do it again.
fn backoff(attempt: u32) -> Duration {
    // 250ms, 500ms, 1s, 2s, then 4s for everything after that.
    let exponent = attempt.saturating_sub(1).min(4);
    let base = 250u64.saturating_mul(1u64 << exponent).min(4_000);

    let mut bytes = [0u8; 2];
    let jitter = match getrandom::fill(&mut bytes) {
        Ok(()) => u64::from(u16::from_le_bytes(bytes)) % 250,
        Err(_) => 0,
    };

    Duration::from_millis(base + jitter)
}

/// A process-wide floor between requests.
///
/// A permit is taken *before* the sleep, so concurrent callers queue rather than all waking at
/// once: twenty in flight become twenty starts spread 45ms apart, not twenty simultaneous ones.
struct Pace {
    next: tokio::sync::Mutex<tokio::time::Instant>,
    interval: Duration,
}

impl Pace {
    fn new(interval: Duration) -> Self {
        Self {
            next: tokio::sync::Mutex::new(tokio::time::Instant::now()),
            interval,
        }
    }

    async fn wait(&self) {
        let at = {
            let mut next = self.next.lock().await;
            let now = tokio::time::Instant::now();
            let at = (*next).max(now);
            *next = at + self.interval;
            at
        };

        tokio::time::sleep_until(at).await;
    }
}

/// What one title contributes to a discovery refresh.
///
/// The cast is what the synergy table is built from; the other three fields exist only to make
/// the admin's franchise suggestions one click instead of a lookup. None of them *decides* a
/// franchise: TMDB's collections are narrower than franchises and its keywords are noisy, so
/// they are offered as suggestions and the maintainer chooses.
#[derive(Debug, Clone)]
pub struct TitleFacts {
    pub meta: SelectionMeta,
    pub cast: Vec<PersonClaim>,
    pub collection_id: Option<u32>,
    pub director_ids: Vec<u32>,
    pub keyword_ids: Vec<u32>,
}

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
    /// One floor shared by every request this process makes, refreshes included.
    pace: Pace,
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
            pace: Pace::new(MIN_REQUEST_INTERVAL),
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

    /// Public form of [`Self::image`] for callers outside this module — the discovery lists
    /// build their own items from `/movie/popular`, which returns bare paths.
    pub fn image_url(&self, path: &str) -> String {
        self.image(Some(path)).unwrap_or_default()
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
            let response = self.send(&url, &key).await?;

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

    /// One upstream request, paced and retried.
    ///
    /// Every attempt passes the process-wide pacer first, so a hundred concurrent callers become
    /// a steady stream rather than a burst. A 429 is retried after its own `Retry-After`; a 5xx
    /// and a dead connection are retried with backoff; anything else fails at once, because
    /// asking again for a title TMDB says does not exist is how a polite client becomes a rude
    /// one.
    async fn send(&self, url: &Url, key: &str) -> Result<reqwest::Response, AppError> {
        let mut attempt = 1;

        loop {
            let mut request = self.client.get(url.clone());
            if self.auth == TmdbAuth::Bearer {
                request = request.bearer_auth(&self.token);
            }

            self.pace.wait().await;

            let outcome = request.send().await;

            let problem = match outcome {
                Ok(response) => {
                    let status = response.status();
                    match on_status(status, retry_after(&response), attempt) {
                        Next::Use => return Ok(response),
                        Next::Fail(Status::NotFound) => return Err(AppError::NotFound),
                        Next::Fail(Status::RateLimited(retry_after_secs)) => {
                            return Err(AppError::RateLimited { retry_after_secs })
                        }
                        Next::Fail(Status::Upstream) => {
                            return Err(AppError::Upstream(format!("{key}: status {status}")))
                        }
                        Next::Retry(wait) => {
                            tracing::warn!(%key, %status, ?wait, attempt, "waiting before retry");
                            wait
                        }
                    }
                }
                // A timeout or a refused connection is worth another go; a malformed URL is not.
                Err(error)
                    if (error.is_timeout() || error.is_connect()) && attempt < MAX_ATTEMPTS =>
                {
                    let wait = backoff(attempt);
                    tracing::warn!(%key, %error, ?wait, attempt, "retrying after a transport error");
                    wait
                }
                Err(error) => return Err(AppError::Upstream(format!("{key}: {error}"))),
            };

            tokio::time::sleep(problem).await;
            attempt += 1;

            if attempt > MAX_ATTEMPTS {
                return Err(AppError::Upstream(format!(
                    "{key}: gave up after {MAX_ATTEMPTS} attempts"
                )));
            }
        }
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

    /// A discovery list — `/movie/popular`, `/movie/upcoming`, `/tv/on_the_air`, and so on.
    ///
    /// `region` is only sent for the lists that need one: TMDB rejects a region on endpoints that
    /// do not take it, and ignores one on the rest.
    /// One TMDB list endpoint.
    ///
    /// `params` are extra query pairs — `sort_by`, release-type filters, date windows. They belong
    /// to the caller because "top" and "new" are different questions to the same endpoint, and
    /// spelling them at the call site keeps the query visible in the code rather than assembled
    /// somewhere else.
    /// Several pages of one list, concatenated and deduplicated.
    ///
    /// A page is twenty titles, and twenty is not enough for a matrix: two titles out of twenty
    /// usually share nothing, so the interesting rows never appear. Five pages gives the synergy
    /// table something to find, at the cost of five list calls per list per refresh.
    ///
    /// Stops early when a page comes back short, which is how TMDB says there is no more.
    pub async fn list(
        &self,
        path: &str,
        params: &[(&str, &str)],
        region: Option<&str>,
        pages: u8,
    ) -> Result<Vec<types::ListItem>, AppError> {
        // Split into segments: `get` percent-encodes each one, so passing "movie/popular" as a
        // single segment sends `/3/movie%2Fpopular` — a 404 that looks like a permissions
        // problem in the log.
        let segments: Vec<&str> = path.split('/').filter(|part| !part.is_empty()).collect();
        let mut out: Vec<types::ListItem> = Vec::new();
        let mut seen: std::collections::HashSet<u32> = std::collections::HashSet::new();

        for page in 1..=pages.max(1) {
            let number = page.to_string();
            let mut query = vec![("language", LANGUAGE), ("page", number.as_str())];
            query.extend(params.iter().copied());
            if let Some(region) = region {
                query.push(("region", region));
            }

            // A page that fails after the first keeps what came before it: a list of eighty
            // titles with a gap beats no list at all, and the refresh runs again later.
            let response: types::ListResponse = match self.get(&segments, &query).await {
                Ok(response) => response,
                Err(error) => {
                    if out.is_empty() {
                        return Err(error);
                    }
                    tracing::warn!(%path, %page, %error, "stopping at this page, keeping {} titles", out.len());
                    break;
                }
            };

            let short = response.results.len() < PAGE_SIZE;

            for item in response.results {
                // The same title can sit on two pages when a list shifts between requests.
                if seen.insert(item.id) {
                    out.push(item);
                }
            }

            if short {
                break;
            }
        }

        Ok(out)
    }

    /// Cast of one selected title. Used when comparing titles.
    ///
    /// Returns the title's own metadata as well as its cast, from a single upstream call.
    pub async fn title_selection(
        &self,
        selection: Selection,
    ) -> Result<(SelectionMeta, Vec<PersonClaim>), AppError> {
        let facts = self.title_facts(selection).await?;
        Ok((facts.meta, facts.cast))
    }

    /// Everything one title contributes to a discovery refresh.
    ///
    /// The collection, the directors and the keywords all arrive in the same
    /// `append_to_response` as the cast, so asking for them costs no extra request — only a
    /// slightly larger response, cached under the same key the comparison flow already uses.
    pub async fn title_facts(&self, selection: Selection) -> Result<TitleFacts, AppError> {
        let id = selection.id.get().to_string();

        let detail: types::MediaDetail = match selection.kind {
            Kind::Movie => {
                self.get(
                    &["movie", id.as_str()],
                    &[
                        ("append_to_response", "credits,keywords"),
                        ("language", LANGUAGE),
                    ],
                )
                .await?
            }
            Kind::Tv => {
                self.get(
                    &["tv", id.as_str()],
                    &[
                        ("append_to_response", "aggregate_credits,keywords"),
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
                    order: c.order,
                })
            })
            .collect();

        Ok(TitleFacts {
            meta,
            cast: claims,
            collection_id: detail.collection_id(),
            director_ids: detail.director_ids(),
            keyword_ids: detail.keyword_ids(),
        })
    }

    /// Every part of a TMDB collection.
    ///
    /// One call, and only when the maintainer asks for it: collections are suggestions for a
    /// franchise, never the franchise itself, so nothing here runs on a normal page render.
    pub async fn collection(&self, id: u32) -> Result<types::CollectionDetail, AppError> {
        let id = id.to_string();

        self.get(&["collection", id.as_str()], &[("language", LANGUAGE)])
            .await
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

    /// TMDB asks to be respected rather than probed: a 429 is retried after the wait it names,
    /// and two seconds when it does not name one.
    #[test]
    fn a_429_is_retried_after_its_retry_after() {
        assert_eq!(
            on_status(StatusCode::TOO_MANY_REQUESTS, Some(7), 1),
            Next::Retry(Duration::from_secs(7))
        );
        assert_eq!(
            on_status(StatusCode::TOO_MANY_REQUESTS, None, 2),
            Next::Retry(Duration::from_secs(2))
        );
    }

    /// Eventually it is the caller's problem, and it says how long to wait.
    #[test]
    fn a_429_that_never_clears_is_reported_with_its_wait() {
        assert_eq!(
            on_status(StatusCode::TOO_MANY_REQUESTS, Some(3), MAX_ATTEMPTS),
            Next::Fail(Status::RateLimited(3))
        );
    }

    /// A 403 or a 404 is an answer, not a hiccup: retrying it is how a client becomes a nuisance.
    #[test]
    fn a_4xx_that_is_not_429_is_never_retried() {
        assert_eq!(
            on_status(StatusCode::FORBIDDEN, None, 1),
            Next::Fail(Status::Upstream)
        );
        assert_eq!(
            on_status(StatusCode::NOT_FOUND, None, 1),
            Next::Fail(Status::NotFound)
        );
    }

    /// A 5xx is worth another go, until the attempts are spent.
    #[test]
    fn a_5xx_is_retried_with_backoff_until_the_attempts_run_out() {
        match on_status(StatusCode::INTERNAL_SERVER_ERROR, None, 1) {
            Next::Retry(wait) => assert!(wait >= Duration::from_millis(250), "{wait:?}"),
            other => panic!("expected a retry, got {other:?}"),
        }

        assert_eq!(
            on_status(StatusCode::BAD_GATEWAY, None, MAX_ATTEMPTS),
            Next::Fail(Status::Upstream)
        );
    }

    /// Backoff has to grow, has to stop growing, and has to be jittered — a hundred titles fail
    /// together, and without jitter they all come back at the same instant and fail together.
    #[test]
    fn backoff_grows_then_stops() {
        let first = backoff(1);
        let second = backoff(2);
        let last = backoff(9);

        assert!(first >= Duration::from_millis(250) && first < Duration::from_millis(500));
        assert!(second >= Duration::from_millis(500) && second < Duration::from_millis(750));
        assert!(last >= Duration::from_millis(4_000) && last < Duration::from_millis(4_250));
    }

    /// The pacer must space requests out rather than let a batch through at once.
    #[tokio::test]
    async fn the_pacer_spaces_requests_apart() {
        let pace = Pace::new(Duration::from_millis(20));
        let start = tokio::time::Instant::now();

        for _ in 0..5 {
            pace.wait().await;
        }

        assert!(
            start.elapsed() >= Duration::from_millis(80),
            "five requests at 20ms each cannot happen in {:?}",
            start.elapsed()
        );
    }

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
            tmdb_region: "US".into(),
            admin_token: None,
            autoban_requests: 0,
            autoban_errors: 0,
            autoban_window: Duration::from_secs(60),
            autoban_ban_secs: 3600,
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
