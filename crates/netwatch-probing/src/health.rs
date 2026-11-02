//! Grading a role from its recent outcomes, and reading loss along a path
//! (ADR-0004, ADR-0030).

use std::collections::{BTreeMap, VecDeque};

use netwatch_shared::uom::si::time::nanosecond;
use netwatch_shared::{IdentityEpoch, Instant, Role};

/// How a role is doing right now (glossary: *status*).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Health {
    /// Nothing recent to judge by.
    Unknown,
    /// Every recent probe was answered.
    Healthy,
    /// Loss at an intermediate hop that doesn't carry through to the rest of the path:
    /// the router is deprioritizing ICMP, so this is not loss on the path
    /// (glossary: *suspected rate limiting*).
    RateLimited,
    /// Some recent probes were lost.
    Degraded,
    /// Half or more of the recent probes were lost.
    Failing,
}

/// How many recent outcomes a grade looks at.
pub const WINDOW: usize = 12;

/// How many outcomes a grade needs before it can be failing (ADR-0056). One lost
/// probe of one, such as the first on a new path, is degraded, not an outage.
pub const FAILING_NEEDS: usize = 3;

/// A role whose newest outcome is older than this many seconds is too stale to
/// grade, so it is unknown.
pub const STALE_AFTER_S: f64 = 30.0;

/// How long there must have been no egress path before the path is graded down
/// (ADR-0055).
pub const NO_EGRESS_AFTER_S: f64 = 5.0;

impl Health {
    /// The worse of two grades, for combining a role's probes with its layer
    /// report: failing, then degraded, then rate-limited, then healthy, then unknown.
    pub fn worse(self, other: Health) -> Health {
        self.max(other)
    }
}

/// Grades a role from its most recent outcomes (`true` = answered), oldest first,
/// at most [`WINDOW`] of them. `fresh` says whether the newest one is recent enough
/// to judge by at all.
pub fn grade(recent: &[bool], fresh: bool) -> Health {
    let recent = &recent[recent.len().saturating_sub(WINDOW)..];
    if recent.is_empty() || !fresh {
        return Health::Unknown;
    }
    let lost = recent.iter().filter(|answered| !**answered).count();
    if lost == 0 {
        Health::Healthy
    } else if lost * 2 < recent.len() || recent.len() < FAILING_NEEDS {
        Health::Degraded
    } else {
        Health::Failing
    }
}

/// A role's recent outcomes, from which it is graded: those of its newest epoch
/// only, so that a grade never mixes two identities (ADR-0055). An outcome of an
/// older epoch, such as a stream retired by a failover resolving a late probe, is
/// left out: it measured a path that is no longer graded.
#[derive(Debug, Clone, Default)]
pub struct Outcomes {
    epoch: Option<IdentityEpoch>,
    recent: VecDeque<(Instant, bool)>,
}

impl Outcomes {
    /// Adds an outcome (`answered` or not) of `epoch`, which arrived at `at`.
    pub fn observe(&mut self, epoch: IdentityEpoch, at: Instant, answered: bool) {
        match self.epoch {
            Some(current) if epoch < current => return,
            Some(current) if epoch == current => {}
            _ => {
                self.epoch = Some(epoch);
                self.recent.clear();
            }
        }
        self.recent.push_back((at, answered));
        if self.recent.len() > WINDOW {
            self.recent.pop_front();
        }
    }

    /// The grade at `now`: [`grade`] of the window, unknown once it is stale.
    pub fn grade(&self, now: Instant) -> Health {
        let answered: Vec<bool> = self.recent.iter().map(|&(_, answered)| answered).collect();
        let fresh = self
            .recent
            .back()
            .is_some_and(|&(at, _)| seconds(now.since(at)) <= STALE_AFTER_S);
        grade(&answered, fresh)
    }
}

/// A duration in seconds, through whole nanoseconds, so that a bound such as 30 s
/// compares exactly.
fn seconds(duration: netwatch_shared::uom::si::f64::Time) -> f64 {
    duration.get::<nanosecond>().round() / 1e9
}

/// Whether this host has had no way out for long enough to grade the path down
/// (ADR-0053, ADR-0055). At a start, and between the observations of one change,
/// there is briefly no egress path, and that is not an outage.
#[derive(Debug, Clone, Default)]
pub struct EgressWatch {
    /// When the egress path went.
    gone: Option<Instant>,
}

impl EgressWatch {
    /// Observes at `now` whether there is an egress path. Returns whether there has
    /// been none for at least [`NO_EGRESS_AFTER_S`].
    pub fn observe(&mut self, egress: bool, now: Instant) -> bool {
        if egress {
            self.gone = None;
            return false;
        }
        let since = *self.gone.get_or_insert(now);
        seconds(now.since(since)) >= NO_EGRESS_AFTER_S
    }
}

/// The grades while this host has no egress path at all (ADR-0053): nothing can be
/// sent, so the local interface and every target and hop are failing, whatever
/// their stale samples say. The kernel and the radios, which aren't on the path, keep
/// their grades.
pub fn without_egress(grades: &BTreeMap<Role, Health>) -> BTreeMap<Role, Health> {
    let mut graded: BTreeMap<Role, Health> = grades
        .iter()
        .map(|(role, &health)| match role {
            Role::Target(_) | Role::Hop { .. } => (role.clone(), Health::Failing),
            _ => (role.clone(), health),
        })
        .collect();
    graded.insert(Role::LocalInterface, Health::Failing);
    graded
}

/// Applies ADR-0004 across every path: a degraded or failing hop with a healthy role
/// further along the same path (a later hop toward the same target, or the target
/// itself) is `RateLimited`, never alarmed on. Every other grade is unchanged.
pub fn along_paths(grades: &BTreeMap<Role, Health>) -> BTreeMap<Role, Health> {
    grades
        .iter()
        .map(|(role, &health)| {
            let lossy = matches!(health, Health::Degraded | Health::Failing);
            let healthy_beyond = match role {
                Role::Hop { target, ttl } if lossy => grades.iter().any(|(other, &h)| {
                    h == Health::Healthy
                        && match other {
                            Role::Hop { target: t, ttl: n } => t == target && n > ttl,
                            Role::Target(t) => t == target,
                            _ => false,
                        }
                }),
                _ => false,
            };
            (
                role.clone(),
                if healthy_beyond {
                    Health::RateLimited
                } else {
                    health
                },
            )
        })
        .collect()
}
