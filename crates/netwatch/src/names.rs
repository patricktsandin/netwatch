//! Router names (ADR-0040): PTR names and origin ASes for the addresses on this
//! host's paths, looked up from the system resolver and cached.

use std::collections::HashMap;
use std::net::IpAddr;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use netwatch_adapter_sockets::lookup::{self, RecordType};
use netwatch_host_health::facts::{RouterName, is_public, parse_as_name, parse_origin};

/// How long a name is kept, and how long a lookup that found nothing is.
const KEEP: Duration = Duration::from_secs(3600);
const KEEP_MISSING: Duration = Duration::from_secs(600);

/// Each address's name (or that it has none), and when that stops being fresh.
type Cache = HashMap<IpAddr, (Option<String>, Instant)>;

/// Looks up router names, at most once per address per [`KEEP`]. Cheap to clone:
/// clones share one cache.
#[derive(Clone)]
pub struct Names {
    resolver: Option<IpAddr>,
    asn_lookups: bool,
    cache: Arc<Mutex<Cache>>,
}

impl Names {
    /// Names from `resolver` (none: every name is missing), with origin ASes if
    /// `asn_lookups`.
    pub fn new(resolver: Option<IpAddr>, asn_lookups: bool) -> Self {
        Self {
            resolver,
            asn_lookups,
            cache: Arc::default(),
        }
    }

    /// `address`'s name in words, if it has one, from the cache when it's fresh.
    pub async fn name(&self, address: IpAddr) -> Option<String> {
        if let Some((name, expires)) = self.cache().get(&address)
            && Instant::now() < *expires
        {
            return name.clone();
        }
        let name = self.look_up(address).await.text();
        let keep = if name.is_some() { KEEP } else { KEEP_MISSING };
        self.cache()
            .insert(address, (name.clone(), Instant::now() + keep));
        name
    }

    /// Every name already known, without looking anything up.
    pub fn known(&self) -> HashMap<IpAddr, String> {
        self.cache()
            .iter()
            .filter_map(|(address, (name, _))| Some((*address, name.clone()?)))
            .collect()
    }

    fn cache(&self) -> std::sync::MutexGuard<'_, Cache> {
        // A panic while holding the lock leaves only a cache, which is still valid.
        self.cache
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
    }

    async fn look_up(&self, address: IpAddr) -> RouterName {
        let Some(server) = self.resolver else {
            return RouterName::default();
        };
        let first = |texts: Option<Vec<String>>| texts.and_then(|t| t.into_iter().next());
        let ptr =
            first(lookup::lookup(server, &lookup::reverse_name(address), RecordType::Ptr).await)
                .map(|name| sanitize(name.trim_end_matches('.')))
                .filter(|name| !name.is_empty());
        // Private and other unannounced addresses have no origin.
        let (asn, as_name) = if self.asn_lookups && is_public(address) {
            let asn =
                first(lookup::lookup(server, &lookup::origin_name(address), RecordType::Txt).await)
                    .and_then(|text| parse_origin(&text));
            let as_name = match asn {
                Some(asn) => {
                    first(lookup::lookup(server, &lookup::as_name_name(asn), RecordType::Txt).await)
                        .and_then(|text| parse_as_name(&text))
                        .map(|name| sanitize(&name))
                }
                None => None,
            };
            (asn, as_name)
        } else {
            (None, None)
        };
        RouterName { ptr, asn, as_name }
    }
}

/// A name from the network, made safe to show and store: printable, on one line,
/// and not absurdly long.
fn sanitize(text: &str) -> String {
    text.chars().filter(|c| !c.is_control()).take(128).collect()
}

/// How often hop identities are checked for names not yet looked up.
const FOLLOW_EVERY: Duration = Duration::from_secs(30);

/// Keeps every hop identity named: every [`FOLLOW_EVERY`], looks up the names of
/// the addresses the hops currently hold. The cache makes repeated lookups free.
pub fn follow_hops(
    names: Names,
    current: tokio::sync::watch::Receiver<netwatch_adapter_web::Current>,
) {
    tokio::spawn(async move {
        loop {
            let addresses: std::collections::BTreeSet<IpAddr> = current
                .borrow()
                .bindings
                .iter()
                .filter(|(key, _)| key.starts_with("hop:"))
                .filter_map(|(_, b)| b.identity.parse().ok())
                .collect();
            for address in addresses {
                names.name(address).await;
            }
            tokio::time::sleep(FOLLOW_EVERY).await;
        }
    });
}
