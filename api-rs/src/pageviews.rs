//! Deciding whether a page view is a new one.
//!
//! The same visitor reloading a comparison, or opening it and then letting the page call
//! `/api/findout` for the results, is one view. Without this, a single person refreshing would
//! look like an audience.
//!
//! Deduplication is in memory and lasts 30 minutes. That is a deliberate trade: a database round
//! trip checked on every page render costs more than losing the counts on restart, and this is a
//! popularity signal, not an invoice.

use std::time::Duration;

use moka::future::Cache;

const TTL: Duration = Duration::from_secs(30 * 60);

/// Above this many tracked pairs, the cache evicts oldest-first. A spray of distinct
/// comparisons must not grow it without bound.
const CAPACITY: u64 = 50_000;

pub struct ViewDeduper {
    seen: Cache<String, ()>,
}

impl Default for ViewDeduper {
    fn default() -> Self {
        Self::new()
    }
}

impl ViewDeduper {
    pub fn new() -> Self {
        Self {
            seen: Cache::builder()
                .max_capacity(CAPACITY)
                .time_to_live(TTL)
                .build(),
        }
    }

    /// True the first time this visitor is seen looking at this comparison in the window.
    ///
    /// A visitor with no session still gets counted, keyed on the comparison alone — otherwise a
    /// client that refuses cookies would be invisible, and on a public page that is a real share
    /// of first-time traffic.
    pub async fn first_sighting(&self, session: Option<&str>, key: &str) -> bool {
        let entry = match session {
            Some(session) if !session.is_empty() => format!("{session}\u{1f}{key}"),
            _ => format!("anon\u{1f}{key}"),
        };

        if self.seen.get(&entry).await.is_some() {
            return false;
        }

        self.seen.insert(entry, ()).await;
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn the_same_visitor_and_comparison_is_counted_once() {
        let deduper = ViewDeduper::new();

        assert!(deduper.first_sighting(Some("s1"), "title:a,b").await);
        assert!(
            !deduper.first_sighting(Some("s1"), "title:a,b").await,
            "a refresh is not a new visitor"
        );
    }

    #[tokio::test]
    async fn a_different_visitor_or_comparison_is_counted() {
        let deduper = ViewDeduper::new();

        assert!(deduper.first_sighting(Some("s1"), "title:a,b").await);
        assert!(deduper.first_sighting(Some("s2"), "title:a,b").await);
        assert!(deduper.first_sighting(Some("s1"), "title:a,c").await);
    }

    /// The separator matters: without it, session "a" + key "b:c" and session "a:b" + key "c"
    /// would collide.
    #[tokio::test]
    async fn a_crafted_pair_does_not_collide_with_another() {
        let deduper = ViewDeduper::new();

        assert!(deduper.first_sighting(Some("a"), "b:c").await);
        assert!(deduper.first_sighting(Some("a:b"), "c").await);
    }

    #[tokio::test]
    async fn a_visitor_without_a_session_is_still_counted_once() {
        let deduper = ViewDeduper::new();

        assert!(deduper.first_sighting(None, "title:a,b").await);
        assert!(!deduper.first_sighting(None, "title:a,b").await);
        assert!(
            !deduper.first_sighting(Some(""), "title:a,b").await,
            "an empty session shares the anonymous bucket"
        );
    }
}
