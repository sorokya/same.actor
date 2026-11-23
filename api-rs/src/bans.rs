//! In-memory ban enforcement.
//!
//! The ban list is small enough to hold in memory and read on every request, which is the point:
//! a banned client must not cost a database query. The set is rebuilt from the store every 30
//! seconds and immediately after any change, so a ban created in the dashboard takes effect on
//! the next request rather than at the next refresh.
//!
//! Matching prefers the narrowest rule. An exact address beats any range, and among ranges the
//! longest prefix wins — so a `/24` ban does not shadow a specific-address unban attempt, and
//! the log records the rule that actually caught the request.

use std::collections::HashMap;
use std::net::IpAddr;
use std::sync::{Arc, Mutex, RwLock};
use std::time::{Duration, Instant};

use ipnet::IpNet;

use crate::error::AppError;
use crate::store::bans::{Ban, NewBan};
use crate::store::Store;

const REFRESH: Duration = Duration::from_secs(30);

enum Matcher {
    Exact(IpAddr),
    Range(IpNet, u8),
}

struct Compiled {
    ban: Ban,
    matcher: Option<Matcher>,
}

/// A snapshot of the ban list, pre-parsed.
pub struct BanSet {
    compiled: Vec<Compiled>,
}

impl BanSet {
    /// Compile a stored list, dropping bans that have expired and targets that do not parse.
    pub fn compile(bans: Vec<Ban>, now: i64) -> Self {
        let compiled = bans
            .into_iter()
            .filter(|ban| ban.expires_ts.is_none_or(|expires| expires > now))
            .map(|ban| {
                // A target that does not parse is kept in the admin list but never matches.
                // Dropping it here is the only safe reading: an unparseable "ban" that matched
                // everything would be far worse than one that matches nothing.
                let matcher = match ban.target.parse::<IpAddr>() {
                    Ok(ip) => Some(Matcher::Exact(ip)),
                    Err(_) => ban
                        .target
                        .parse::<IpNet>()
                        .ok()
                        .map(|net| Matcher::Range(net, net.prefix_len())),
                };

                if matcher.is_none() {
                    tracing::warn!(target = %ban.target, "a ban target is not an address or a range");
                }

                Compiled { ban, matcher }
            })
            .collect();

        Self { compiled }
    }

    pub fn is_empty(&self) -> bool {
        self.compiled.is_empty()
    }

    /// The ban that catches this address, if any. Exact first, then the longest matching range.
    pub fn find(&self, ip: IpAddr) -> Option<&Ban> {
        let mut best: Option<(&Ban, u8)> = None;

        for compiled in &self.compiled {
            match &compiled.matcher {
                Some(Matcher::Exact(exact)) if *exact == ip => return Some(&compiled.ban),
                Some(Matcher::Range(net, prefix))
                    if net.contains(&ip)
                        && best.is_none_or(|(_, best_prefix)| *prefix > best_prefix) =>
                {
                    best = Some((&compiled.ban, *prefix));
                }
                _ => {}
            }
        }

        best.map(|(ban, _)| ban)
    }
}

/// The live ban list.
pub struct BanCache {
    set: RwLock<Arc<BanSet>>,
}

impl Default for BanCache {
    fn default() -> Self {
        Self::new()
    }
}

impl BanCache {
    pub fn new() -> Self {
        Self {
            set: RwLock::new(Arc::new(BanSet::compile(Vec::new(), 0))),
        }
    }

    pub fn find(&self, ip: IpAddr) -> Option<Ban> {
        self.set
            .read()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .find(ip)
            .cloned()
    }

    pub fn len(&self) -> usize {
        let set = self
            .set
            .read()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        set.compiled.len()
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// Replace the set from the store. Called at startup, on a timer, and after a mutation.
    pub async fn reload(&self, store: &Store) -> Result<usize, AppError> {
        let bans = store.bans().await?;
        let set = BanSet::compile(bans, crate::clock::now_unix());

        let len = set.compiled.len();
        *self
            .set
            .write()
            .unwrap_or_else(|poisoned| poisoned.into_inner()) = Arc::new(set);

        Ok(len)
    }

    /// Refresh on a timer, for bans that expire or that another process wrote.
    pub fn spawn_refresher(cache: Arc<Self>, store: Arc<Store>) {
        tokio::spawn(async move {
            if let Err(error) = cache.reload(&store).await {
                tracing::warn!(%error, "could not load bans at startup");
            }

            loop {
                tokio::time::sleep(REFRESH).await;
                if let Err(error) = cache.reload(&store).await {
                    tracing::warn!(%error, "could not refresh bans");
                }
            }
        });
    }
}

/// Create a ban and make it live immediately.
///
/// Both halves matter: writing without reloading means the dashboard says "banned" while
/// requests keep flowing for up to 30 seconds, which is exactly when someone is watching.
pub async fn apply(store: &Store, cache: &BanCache, ban: NewBan) -> Result<i64, AppError> {
    let id = store.insert_ban(ban).await?;
    cache.reload(store).await?;
    Ok(id)
}

/// Counts violations per address and reports when a threshold is crossed.
///
/// Fires once per crossing rather than on every subsequent violation, so one bad client
/// produces one ban instead of a database write per request.
pub struct AutoBan {
    threshold: u32,
    window: Duration,
    counts: Mutex<HashMap<IpAddr, (Instant, u32)>>,
}

impl AutoBan {
    /// A threshold of 0 disables the rule.
    pub fn new(threshold: u32, window: Duration) -> Self {
        Self {
            threshold,
            window,
            counts: Mutex::new(HashMap::new()),
        }
    }

    pub fn enabled(&self) -> bool {
        self.threshold > 0
    }

    /// Record one violation. Returns true when this is the request that crossed the threshold.
    pub fn note(&self, ip: IpAddr) -> bool {
        if !self.enabled() {
            return false;
        }

        let now = Instant::now();
        let mut counts = self
            .counts
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());

        // Bound the map: a spray of spoofed addresses must not grow it without limit.
        if counts.len() > 10_000 {
            counts.retain(|_, (started, _)| now.duration_since(*started) < self.window);
        }

        let entry = counts.entry(ip).or_insert((now, 0));
        if now.duration_since(entry.0) >= self.window {
            *entry = (now, 0);
        }

        entry.1 += 1;

        if entry.1 >= self.threshold {
            // Reset rather than remove: a client that keeps going should be re-banned once the
            // window refills, not once per request.
            *entry = (now, 0);
            return true;
        }

        false
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::net::Ipv4Addr;

    fn ip(last: u8) -> IpAddr {
        IpAddr::V4(Ipv4Addr::new(203, 0, 113, last))
    }

    fn ban(id: i64, target: &str, expires: Option<i64>) -> Ban {
        Ban {
            id,
            created_ts: 0,
            expires_ts: expires,
            target: target.into(),
            kind: if target.contains('/') { "cidr" } else { "ip" }.into(),
            reason: Some(format!("ban {id}")),
            created_by: "test".into(),
            hits: 0,
            last_hit: None,
        }
    }

    #[test]
    fn an_exact_address_matches_only_itself() {
        let set = BanSet::compile(vec![ban(1, "203.0.113.7", None)], 1_000);
        assert_eq!(set.find(ip(7)).map(|ban| ban.id), Some(1));
        assert_eq!(set.find(ip(8)), None);
    }

    #[test]
    fn a_range_catches_every_address_in_it() {
        let set = BanSet::compile(vec![ban(1, "203.0.113.0/24", None)], 1_000);
        assert_eq!(set.find(ip(1)).map(|ban| ban.id), Some(1));
        assert_eq!(set.find(ip(254)).map(|ban| ban.id), Some(1));
        assert_eq!(set.find("198.51.100.1".parse().unwrap()), None);
    }

    /// The narrowest rule is reported, so a /24 does not hide a specific ban inside it.
    #[test]
    fn the_longest_prefix_wins() {
        let set = BanSet::compile(
            vec![
                ban(1, "203.0.113.0/24", None),
                ban(2, "203.0.113.7/32", None),
            ],
            1_000,
        );
        assert_eq!(set.find(ip(7)).map(|ban| ban.id), Some(2));
        assert_eq!(set.find(ip(8)).map(|ban| ban.id), Some(1));
    }

    #[test]
    fn an_exact_match_beats_a_range_regardless_of_order() {
        let set = BanSet::compile(
            vec![
                ban(2, "203.0.113.7/32", None),
                ban(1, "203.0.113.0/24", None),
            ],
            1_000,
        );
        assert_eq!(set.find(ip(7)).map(|ban| ban.id), Some(2));
    }

    #[test]
    fn ipv6_ranges_work() {
        let set = BanSet::compile(vec![ban(1, "2001:db8::/32", None)], 1_000);
        assert_eq!(
            set.find("2001:db8::1".parse().unwrap()).map(|ban| ban.id),
            Some(1)
        );
        assert_eq!(set.find("2001:db9::1".parse().unwrap()), None);
    }

    #[test]
    fn an_expired_ban_does_not_match() {
        let set = BanSet::compile(vec![ban(1, "203.0.113.7", Some(1_000))], 2_000);
        assert!(
            set.is_empty(),
            "an expired ban should be dropped at compile time"
        );
        assert_eq!(set.find(ip(7)), None);
    }

    #[test]
    fn a_ban_expiring_later_still_matches() {
        let set = BanSet::compile(vec![ban(1, "203.0.113.7", Some(5_000))], 2_000);
        assert_eq!(set.find(ip(7)).map(|ban| ban.id), Some(1));
    }

    /// An unparseable target must match nothing. The alternative — treating it as a wildcard —
    /// would ban the world.
    #[test]
    fn a_garbage_target_matches_nothing() {
        let set = BanSet::compile(vec![ban(1, "everything", None)], 1_000);
        assert_eq!(set.find(ip(7)), None);
        assert_eq!(set.find("198.51.100.1".parse().unwrap()), None);
    }

    #[test]
    fn the_auto_ban_fires_once_at_the_threshold() {
        let auto = AutoBan::new(3, Duration::from_secs(60));

        assert!(!auto.note(ip(7)));
        assert!(!auto.note(ip(7)), "second violation is below the threshold");
        assert!(auto.note(ip(7)), "third crosses it");
        assert!(!auto.note(ip(7)), "and does not fire again immediately");
        assert!(!auto.note(ip(7)));
        assert!(auto.note(ip(7)), "the counter refills and fires again");
    }

    #[test]
    fn the_auto_ban_is_per_address() {
        let auto = AutoBan::new(2, Duration::from_secs(60));
        assert!(!auto.note(ip(7)));
        assert!(
            !auto.note(ip(8)),
            "one address's traffic must not ban another"
        );
        assert!(auto.note(ip(7)));
    }

    #[test]
    fn a_zero_threshold_disables_the_rule() {
        let auto = AutoBan::new(0, Duration::from_secs(60));
        assert!(!auto.enabled());
        for _ in 0..1_000 {
            assert!(!auto.note(ip(7)));
        }
    }

    #[test]
    fn the_window_resets_the_counter() {
        let auto = AutoBan::new(2, Duration::from_millis(20));
        assert!(!auto.note(ip(7)));
        std::thread::sleep(Duration::from_millis(30));
        assert!(!auto.note(ip(7)), "the window should have reset the count");
        assert!(auto.note(ip(7)));
    }
}
