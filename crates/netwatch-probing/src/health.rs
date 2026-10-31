//! Grading a role from its recent outcomes, and reading loss along a path
//! (ADR-0004, ADR-0030).

use std::collections::BTreeMap;

use netwatch_shared::Role;

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
    } else if lost * 2 < recent.len() {
        Health::Degraded
    } else {
        Health::Failing
    }
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
