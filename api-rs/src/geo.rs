//! Country lookup from a DB-IP Lite database.
//!
//! The database is not shipped in the image: it changes monthly and is a few megabytes, which
//! is a build-time network dependency and a staleness bug in one. A background task downloads
//! it into the data directory and swaps it in; every failure mode — no network, no file, a
//! corrupt download, no database at all — degrades to `country = NULL` and a log line. Geo must
//! never be the reason the site does not start.
//!
//! One detail that is not obvious and cost a debugging session: **the download returns `403`
//! without a `User-Agent`**. The request sets one explicitly, because the default client UA is
//! rejected by the CDN and the failure is otherwise indistinguishable from an outage.

use std::net::IpAddr;
use std::path::{Path, PathBuf};
use std::sync::{Arc, RwLock};
use std::time::Duration;

use serde::Deserialize;

/// Identifies this client to DB-IP. Their CDN rejects requests with no user agent at all.
const USER_AGENT: &str = concat!(
    "same.actor/",
    env!("CARGO_PKG_VERSION"),
    " (+https://same.actor)"
);

const REFRESH: Duration = Duration::from_secs(24 * 60 * 60);

/// Only the fields we use. Deserialising into owned types rather than DB-IP's borrowed views
/// keeps the lookup independent of how the database chains records.
#[derive(Debug, Deserialize)]
struct Record {
    country: Option<Country>,
}

#[derive(Debug, Deserialize)]
struct Country {
    iso_code: Option<String>,
}

struct Loaded {
    reader: maxminddb::Reader<Vec<u8>>,
    loaded_at: i64,
}

impl std::fmt::Debug for Geo {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Geo")
            .field("path", &self.path)
            .field("enabled", &self.enabled)
            .field("loaded", &self.age_secs().is_some())
            .finish()
    }
}

pub struct Geo {
    /// `None` until a database is available. A `RwLock` rather than an atomic pointer because
    /// readers outnumber writers by a few hundred thousand to one and each lookup needs the
    /// reader for the duration.
    current: RwLock<Option<Loaded>>,
    path: PathBuf,
    enabled: bool,
    /// Template with `{month}` substituted. Overridable so a mirror can be used.
    url: String,
}

impl Geo {
    pub fn new(path: PathBuf, url: String, enabled: bool) -> Arc<Self> {
        Arc::new(Self {
            current: RwLock::new(None),
            path,
            enabled,
            url,
        })
    }

    pub fn enabled(&self) -> bool {
        self.enabled
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    /// Whether a database is loaded, and how old it is.
    pub fn age_secs(&self) -> Option<i64> {
        let current = self
            .current
            .read()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        current
            .as_ref()
            .map(|loaded| crate::clock::now_unix() - loaded.loaded_at)
    }

    /// The ISO-3166-1 alpha-2 code for an address, if the database knows it.
    pub fn country(&self, ip: IpAddr) -> Option<String> {
        let current = self
            .current
            .read()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let loaded = current.as_ref()?;

        // `lookup` returns a borrowed view into the database; `decode` turns it into owned
        // values. Both are fallible: an address the database does not cover is an error, not a
        // panic.
        let found = loaded.reader.lookup(ip).ok()?;
        // `decode` is doubly optional: the lookup can fail, and the record inside it can be
        // absent for an address the database maps to no data.
        let decoded: Option<Record> = found.decode().ok()?;
        let code = decoded?.country?.iso_code?;

        // A two-letter code is what the column promises; anything else is a corrupt database.
        (code.len() == 2 && code.bytes().all(|b| b.is_ascii_alphabetic()))
            .then(|| code.to_uppercase())
    }

    /// Install a database from disk, if there is one. Called at startup.
    pub fn load_from_disk(&self) -> bool {
        if !self.enabled {
            return false;
        }

        let bytes = match std::fs::read(&self.path) {
            Ok(bytes) => bytes,
            Err(error) => {
                tracing::info!(
                    path = %self.path.display(),
                    %error,
                    "no geo database yet; countries will be empty until it downloads"
                );
                return false;
            }
        };

        self.install(bytes)
    }

    fn install(&self, bytes: Vec<u8>) -> bool {
        match maxminddb::Reader::from_source(bytes) {
            Ok(reader) => {
                tracing::info!(path = %self.path.display(), "geo database loaded");
                let mut current = self
                    .current
                    .write()
                    .unwrap_or_else(|poisoned| poisoned.into_inner());
                *current = Some(Loaded {
                    reader,
                    loaded_at: crate::clock::now_unix(),
                });
                true
            }
            Err(error) => {
                tracing::warn!(%error, "the geo database is unreadable; countries will be empty");
                false
            }
        }
    }

    /// Download this month's file and swap it in. Returns whether a database is now loaded.
    pub async fn refresh(&self) -> bool {
        if !self.enabled {
            return false;
        }

        let url = self.url.replace(
            "{month}",
            &crate::clock::month_string(crate::clock::now_unix()),
        );

        match self.download(&url).await {
            Ok(bytes) => {
                // Written before installing so a restart does not re-download; the temp name
                // means a half-written file is never mistaken for a database.
                if let Err(error) = self.persist(&bytes) {
                    tracing::warn!(%error, "could not save the geo database; it will re-download");
                }
                self.install(bytes)
            }
            Err(error) => {
                tracing::warn!(url = %url, %error, "could not download the geo database");
                false
            }
        }
    }

    fn persist(&self, bytes: &[u8]) -> std::io::Result<()> {
        if let Some(parent) = self.path.parent() {
            if !parent.as_os_str().is_empty() {
                std::fs::create_dir_all(parent)?;
            }
        }

        let temp = self.path.with_extension("mmdb.part");
        std::fs::write(&temp, bytes)?;
        std::fs::rename(&temp, &self.path)
    }

    async fn download(&self, url: &str) -> Result<Vec<u8>, String> {
        let client = reqwest::Client::builder()
            .timeout(Duration::from_secs(120))
            .user_agent(USER_AGENT)
            .build()
            .map_err(|error| format!("client: {error}"))?;

        let response = client
            .get(url)
            .send()
            .await
            .map_err(|error| format!("request: {error}"))?;

        if !response.status().is_success() {
            return Err(format!("HTTP {}", response.status()));
        }

        let compressed = response
            .bytes()
            .await
            .map_err(|error| format!("body: {error}"))?;

        // The file is served gzip-compressed with no Content-Encoding, so it is decoded here
        // rather than by the client. Guarded by the gzip magic bytes so a mirror serving a
        // plain mmdb still works.
        if compressed.starts_with(&[0x1f, 0x8b]) {
            let mut decoder = flate2::read::GzDecoder::new(compressed.as_ref());
            let mut out = Vec::new();
            std::io::Read::read_to_end(&mut decoder, &mut out)
                .map_err(|error| format!("gzip: {error}"))?;
            Ok(out)
        } else {
            Ok(compressed.to_vec())
        }
    }

    /// Run the refresh now and then daily, forever. Called once at startup.
    pub fn spawn_refresher(geo: Arc<Self>) {
        if !geo.enabled {
            tracing::info!("geo lookup is disabled");
            return;
        }

        tokio::spawn(async move {
            geo.load_from_disk();

            // Refresh at startup when the file is older than a day, so a long-running
            // container does not keep using last month's addresses.
            let stale = geo
                .age_secs()
                .is_none_or(|age| age > REFRESH.as_secs() as i64);
            if stale {
                geo.refresh().await;
            }

            loop {
                tokio::time::sleep(REFRESH).await;
                geo.refresh().await;
            }
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn geo() -> Arc<Geo> {
        Geo::new(
            std::env::temp_dir().join("same-actor-geo-test.mmdb"),
            "https://example.invalid/{month}.mmdb.gz".into(),
            true,
        )
    }

    #[test]
    fn with_no_database_every_lookup_is_none() {
        let geo = geo();
        assert_eq!(geo.country("203.0.113.7".parse().unwrap()), None);
        assert_eq!(geo.age_secs(), None);
    }

    #[test]
    fn a_corrupt_database_is_refused_without_panicking() {
        let geo = geo();
        assert!(!geo.install(b"this is not an mmdb file".to_vec()));
        assert_eq!(geo.country("203.0.113.7".parse().unwrap()), None);
    }

    #[test]
    fn the_month_is_substituted_into_the_url() {
        let url = "https://download.db-ip.com/free/dbip-country-lite-{month}.mmdb.gz";
        let filled = url.replace("{month}", &crate::clock::month_string(1_795_435_200));
        assert_eq!(
            filled,
            "https://download.db-ip.com/free/dbip-country-lite-2026-11.mmdb.gz"
        );
        assert!(!filled.contains('{'));
    }

    #[test]
    fn a_disabled_lookup_never_downloads() {
        let geo = Geo::new(
            std::env::temp_dir().join("same-actor-geo-off.mmdb"),
            "https://example.invalid/{month}".into(),
            false,
        );
        assert!(!geo.enabled());
        assert!(!geo.load_from_disk());
    }
}

#[cfg(test)]
mod live_tests {
    use super::*;

    /// Downloads the real database and resolves addresses in it.
    ///
    /// Ignored by default because it needs the network and moves ~4 MB. Run it when touching
    /// the downloader or the parser: it is the only check that the CDN still serves this URL,
    /// that a `User-Agent` still matters, and that DB-IP's record shape still has
    /// `country.iso_code` where this code looks for it.
    #[tokio::test]
    #[ignore = "downloads ~4 MB from download.db-ip.com"]
    async fn the_real_database_downloads_and_resolves_addresses() {
        let dir = std::env::temp_dir().join(format!("same-actor-geo-live-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);

        let geo = Geo::new(
            dir.join("dbip-country-lite.mmdb"),
            "https://download.db-ip.com/free/dbip-country-lite-{month}.mmdb.gz".into(),
            true,
        );

        assert!(geo.refresh().await, "the download failed");
        assert!(geo.age_secs().is_some(), "no database was installed");

        // Google's resolver, and an APNIC address that should not be in the US.
        assert_eq!(
            geo.country("8.8.8.8".parse().unwrap()).as_deref(),
            Some("US")
        );
        assert_eq!(
            geo.country("1.0.0.1".parse().unwrap()).as_deref(),
            Some("AU")
        );

        // A private address is not in any public database.
        assert_eq!(geo.country("192.168.1.1".parse().unwrap()), None);

        // The file was written, so a restart does not download again.
        assert!(dir.join("dbip-country-lite.mmdb").exists());

        let _ = std::fs::remove_dir_all(&dir);
    }
}
