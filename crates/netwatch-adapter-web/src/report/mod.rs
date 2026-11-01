//! Incident reports (ADR-0033): what happened in a time span, in words an ISP's
//! network operations center can act on.
//!
//! Everything here is pure. The handler in `lib.rs` loads incidents and history for
//! the span, and this module decides what they mean: which incidents are one
//! [`Episode`], where on the path each episode began ([`Place`]), and what every
//! role on the affected paths measured meanwhile ([`PathStats`]). [`render`] writes
//! the result as text and HTML.

pub mod analysis;
pub mod render;

use std::collections::BTreeMap;

use netwatch_history::{Histogram, Rollup};
use netwatch_probing::incident::{Incident, Severity};
use netwatch_shared::{IdentityEpoch, Role, WallTime};
use netwatch_verified::Summary;

/// Where an episode began (glossary: *episode*).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Place {
    /// Beyond the gateway: the ISP's network or further. Listed first.
    BeyondGateway,
    /// At or before hop 1: this host, its interfaces, or the gateway.
    HostOrLan,
}

/// Overlapping incidents, as one problem (glossary: *episode*).
#[derive(Debug, Clone, PartialEq)]
pub struct Episode {
    /// When its first incident started.
    pub started: WallTime,
    /// `None` while any of its incidents is still open.
    pub ended: Option<WallTime>,
    /// The worst of its incidents.
    pub severity: Severity,
    /// The most upstream role among its incidents and their probable causes.
    pub cause: Role,
    /// Every role with an incident in it, in path order.
    pub affected: Vec<Role>,
    /// Where it began, decided by its probable cause.
    pub place: Place,
}

/// Groups incidents into episodes, oldest first (ADR-0034). Two incidents are in one
/// episode when one is the other's probable cause and their bands overlap.
pub fn episodes(incidents: &[Incident]) -> Vec<Episode> {
    let mut sorted: Vec<&Incident> = incidents.iter().collect();
    sorted.sort_by_key(|i| (i.started, path_order(&i.role)));
    let overlap = |a: &Incident, b: &Incident| {
        a.ended.is_none_or(|end| b.started <= end) && b.ended.is_none_or(|end| a.started <= end)
    };
    // Union-find over the sorted incidents.
    let mut parent: Vec<usize> = (0..sorted.len()).collect();
    fn root(parent: &mut [usize], mut i: usize) -> usize {
        while parent[i] != i {
            parent[i] = parent[parent[i]];
            i = parent[i];
        }
        i
    }
    for (i, effect) in sorted.iter().enumerate() {
        let Some(cause) = &effect.cause else { continue };
        for (j, other) in sorted.iter().enumerate() {
            if other.role == *cause && overlap(effect, other) {
                let (a, b) = (root(&mut parent, i), root(&mut parent, j));
                // The earlier incident stays the root, so a group's first member is
                // its earliest.
                parent[a.max(b)] = a.min(b);
            }
        }
    }
    let mut groups: BTreeMap<usize, Vec<&Incident>> = BTreeMap::new();
    for (i, incident) in sorted.iter().enumerate() {
        let r = root(&mut parent, i);
        groups.entry(r).or_default().push(incident);
    }
    groups.into_values().map(|group| episode(&group)).collect()
}

/// One episode from its incidents, earliest first.
fn episode(group: &[&Incident]) -> Episode {
    let mut affected: Vec<Role> = group.iter().map(|i| i.role.clone()).collect();
    affected.sort_by_key(path_order);
    affected.dedup();
    let first = group[0];
    let cause = first.cause.clone().unwrap_or_else(|| first.role.clone());
    Episode {
        started: first.started,
        ended: group
            .iter()
            .map(|i| i.ended)
            .collect::<Option<Vec<_>>>()
            .and_then(|ends| ends.into_iter().max()),
        severity: group
            .iter()
            .map(|i| i.severity)
            .max()
            .unwrap_or(Severity::Degradation),
        place: place_of(&cause),
        cause,
        affected,
    }
}

fn place_of(cause: &Role) -> Place {
    match cause {
        Role::Kernel
        | Role::Nic
        | Role::Tunnel
        | Role::LocalInterface
        | Role::Wifi
        | Role::Cellular => Place::HostOrLan,
        Role::Hop { ttl, .. } if ttl.get().get() == 1 => Place::HostOrLan,
        Role::Hop { .. } | Role::Target(_) => Place::BeyondGateway,
    }
}

/// The order roles are listed in: the host's layers, then each target's hops and the
/// target itself, the way `mtr` lists a path.
pub fn path_order(role: &Role) -> (Option<u32>, u16) {
    match role {
        Role::Kernel => (None, 0),
        Role::Nic => (None, 1),
        Role::Wifi => (None, 2),
        Role::Cellular => (None, 3),
        Role::Tunnel => (None, 4),
        Role::LocalInterface => (None, 5),
        Role::Hop { target, ttl } => (Some(target.get()), u16::from(ttl.get().get())),
        Role::Target(target) => (Some(target.get()), u16::MAX),
    }
}

/// A role's name, for people, matching the UI's row titles. `names` maps target
/// role keys (`target:1`) to their configured names.
pub fn label(role: &Role, names: &BTreeMap<String, String>) -> String {
    let target = |id: u32| {
        names
            .get(&format!("target:{id}"))
            .cloned()
            .unwrap_or_else(|| format!("target {id}"))
    };
    match role {
        Role::Kernel => "Kernel (loopback)".to_owned(),
        Role::Nic => "NIC".to_owned(),
        Role::Tunnel => "Tunnel".to_owned(),
        Role::LocalInterface => "Local interface".to_owned(),
        Role::Wifi => "Wi-Fi".to_owned(),
        Role::Cellular => "Cellular".to_owned(),
        Role::Hop { target: t, ttl } => format!("Hop {} → {}", ttl.get(), target(t.get())),
        Role::Target(t) => target(t.get()),
    }
}

/// Every known role on the paths of `affected` (each affected role and everything
/// upstream of it), in path order.
pub fn roles_on_paths(affected: &[Role], known: &[Role]) -> Vec<Role> {
    let mut on: Vec<Role> = known
        .iter()
        .filter(|r| affected.iter().any(|a| *r == a || r.is_upstream_of(a)))
        .cloned()
        .collect();
    on.sort_by_key(path_order);
    on.dedup();
    on
}

/// What one role measured over a stretch of time.
#[derive(Debug, Clone, PartialEq)]
pub struct PathStats {
    /// Whose measurements.
    pub role: Role,
    /// The identity it measured most: for a hop, the router's address.
    pub identity: Option<String>,
    /// Probes resolved.
    pub sent: u64,
    /// Of those, how many were lost.
    pub lost: u64,
    /// The median RTT of the answered ones.
    pub median_ms: Option<f64>,
    /// The 95th-percentile RTT of the answered ones.
    pub p95_ms: Option<f64>,
}

impl PathStats {
    /// Loss as a percentage of probes sent.
    pub fn loss_percent(&self) -> f64 {
        if self.sent == 0 {
            0.0
        } else {
            self.lost as f64 * 100.0 / self.sent as f64
        }
    }
}

/// Merges rollups (of any tier) into one [`PathStats`] per role, in path order.
/// `identities` names each epoch's identity.
pub fn path_stats(
    rollups: &[Rollup],
    identities: &BTreeMap<IdentityEpoch, String>,
) -> Vec<PathStats> {
    struct Acc {
        summary: Summary,
        histogram: Histogram,
        by_epoch: BTreeMap<IdentityEpoch, u64>,
    }
    let mut roles: BTreeMap<Role, Acc> = BTreeMap::new();
    for r in rollups {
        let acc = roles.entry(r.role.clone()).or_insert_with(|| Acc {
            summary: Summary::empty(),
            histogram: Histogram::default(),
            by_epoch: BTreeMap::new(),
        });
        acc.summary = acc
            .summary
            .merge(&r.summary)
            .expect("u64 sample counts cannot be exhausted");
        acc.histogram.merge(&r.histogram);
        *acc.by_epoch.entry(r.epoch).or_default() += r.summary.count;
    }
    let mut stats: Vec<PathStats> = roles
        .into_iter()
        .map(|(role, acc)| {
            let most = acc
                .by_epoch
                .iter()
                .max_by_key(|&(_, &count)| count)
                .map(|(epoch, _)| *epoch);
            // A target's epoch is its pin's (ADR-0020): the interface it left by, which
            // says nothing about the target, so targets show none.
            let identity = most
                .filter(|_| !matches!(role, Role::Target(_)))
                .and_then(|e| identities.get(&e).cloned());
            PathStats {
                role,
                identity,
                sent: acc.summary.count,
                lost: acc.summary.lost,
                median_ms: acc.histogram.quantile_us(0.5).map(|us| us as f64 / 1000.0),
                p95_ms: acc.histogram.quantile_us(0.95).map(|us| us as f64 / 1000.0),
            }
        })
        .collect();
    stats.sort_by_key(|s| path_order(&s.role));
    stats
}

/// A wall time in UTC, to the second: `2026-11-01 15:03:04 UTC`. A NOC works in UTC.
pub fn utc(at: WallTime) -> String {
    let seconds = at.as_unix_micros().div_euclid(1_000_000);
    let (days, rest) = (seconds.div_euclid(86_400), seconds.rem_euclid(86_400));
    // Howard Hinnant's days-to-civil algorithm.
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let day = doy - (153 * mp + 2) / 5 + 1;
    let month = if mp < 10 { mp + 3 } else { mp - 9 };
    let year = yoe + era * 400 + i64::from(month <= 2);
    format!(
        "{year:04}-{month:02}-{day:02} {:02}:{:02}:{:02} UTC",
        rest / 3600,
        rest % 3600 / 60,
        rest % 60
    )
}

/// A duration in its two largest units: `2 min 6 s`, `1 h`.
pub fn format_duration(micros: i64) -> String {
    const UNITS: [(&str, i64); 4] = [("d", 86_400), ("h", 3600), ("min", 60), ("s", 1)];
    let mut rest = micros.max(0) / 1_000_000;
    let mut parts = Vec::new();
    for (unit, size) in UNITS {
        let n = rest / size;
        rest -= n * size;
        if n > 0 || !parts.is_empty() {
            parts.push((n, unit));
        }
        if parts.len() == 2 {
            break;
        }
    }
    let shown: Vec<String> = parts
        .iter()
        .enumerate()
        .filter(|&(i, &(n, _))| i == 0 || n > 0)
        .map(|(_, (n, unit))| format!("{n} {unit}"))
        .collect();
    if shown.is_empty() {
        "0 s".to_owned()
    } else {
        shown.join(" ")
    }
}
