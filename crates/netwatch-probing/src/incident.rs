//! Incidents: the periods during which a role was degraded or failing (ADR-0031).
//!
//! Statuses say how a role is doing now. [`IncidentTracker`] turns the daemon's
//! gradings, once a second, into a history of incidents that survives the moment:
//! when each one started and ended, how bad it got, and which upstream role probably
//! caused it.
//!
//! The tracker is the design in `specs/incidents.qnt`, and is held to it by trace
//! conformance. What it hides is the difference between an incident's *lifetime*
//! (from the grading that opened it to the one that closed it) and its *band* (the
//! period it reports). Grades look at the last 12 probes, so they trail a loss by up
//! to 12 probe intervals. The band covers the loss instead: it starts at the first
//! lost probe and ends at the first answered probe after the last one.

use std::collections::BTreeMap;

use netwatch_shared::uom::si::f64::Time;
use netwatch_shared::uom::si::time::microsecond;
use netwatch_shared::{Role, WallTime};

use crate::Health;

/// How bad an incident got.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Severity {
    /// Graded degraded, never failing.
    Degradation,
    /// Graded failing at least once.
    Outage,
}

/// One of a role's degraded or failing periods (glossary: *incident*). An incident
/// is identified by its role and start, which never change once it has opened.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Incident {
    /// The role that was degraded or failing.
    pub role: Role,
    /// Where the band starts: the first lost probe, or the grading that opened the
    /// incident if it wasn't caused by loss.
    pub started: WallTime,
    /// Where the band ends: the first answered probe after the last loss, or the
    /// grading that closed the incident. `None` while the incident is still open.
    pub ended: Option<WallTime>,
    /// The worst it got.
    pub severity: Severity,
    /// The most upstream role that had an incident open at the same time
    /// (glossary: *probable cause*).
    pub cause: Option<Role>,
}

/// Turns gradings and probe outcomes into incidents. Feed it every outcome with
/// [`outcome`](Self::outcome), and every grading with [`grade`](Self::grade).
#[derive(Debug)]
pub struct IncidentTracker {
    roles: BTreeMap<Role, Track>,
    /// How long after an incident opened an upstream incident may open and still be
    /// its probable cause (ADR-0034).
    cause_grace_us: i64,
}

#[derive(Debug, Default)]
struct Track {
    last_loss: Option<WallTime>,
    /// The first answered probe after `last_loss`.
    recovered: Option<WallTime>,
    /// The latest grading that found the role calm (not degraded or failing).
    last_calm: Option<WallTime>,
    open: Option<Open>,
}

#[derive(Debug)]
struct Open {
    incident: Incident,
    /// The grading that opened it. Probable causes are judged by when incidents
    /// opened, not by where their bands start.
    opened: WallTime,
    /// Whether the incident has seen loss, which decides where its band ends.
    probed: bool,
}

fn is_bad(health: Health) -> bool {
    matches!(health, Health::Degraded | Health::Failing)
}

fn severity_of(health: Health) -> Severity {
    if health == Health::Failing {
        Severity::Outage
    } else {
        Severity::Degradation
    }
}

impl IncidentTracker {
    /// A tracker with no history: every role starts calm. An upstream incident is a
    /// probable cause only if it opened no later than `cause_grace` after the
    /// incident it explains (ADR-0034); the daemon uses 10 s.
    pub fn new(cause_grace: Time) -> Self {
        Self {
            roles: BTreeMap::new(),
            // `as` saturates, and no grace approaches i64::MAX µs.
            cause_grace_us: cause_grace.get::<microsecond>().round() as i64,
        }
    }

    /// Records one of `role`'s probe outcomes, resolved at `at`.
    pub fn outcome(&mut self, role: &Role, at: WallTime, answered: bool) {
        let track = self.roles.entry(role.clone()).or_default();
        if answered {
            if track.recovered.is_none() && track.last_loss.is_some() {
                track.recovered = Some(at);
            }
        } else {
            track.last_loss = Some(at);
            track.recovered = None;
            if let Some(open) = &mut track.open {
                open.probed = true;
            }
        }
    }

    /// Applies one grading of every role. A role missing from `grades` is calm.
    /// Returns every incident that opened, changed severity or probable cause, or
    /// closed, in role order.
    pub fn grade(&mut self, at: WallTime, grades: &BTreeMap<Role, Health>) -> Vec<Incident> {
        for role in grades.keys() {
            self.roles.entry(role.clone()).or_default();
        }
        let mut updated: BTreeMap<Role, Incident> = BTreeMap::new();
        for (role, track) in &mut self.roles {
            let health = grades.get(role).copied().unwrap_or(Health::Unknown);
            match (&mut track.open, is_bad(health)) {
                (None, true) => {
                    let since_loss = track
                        .last_loss
                        .filter(|&loss| track.last_calm.is_none_or(|calm| loss > calm));
                    let incident = Incident {
                        role: role.clone(),
                        started: since_loss.unwrap_or(at),
                        ended: None,
                        severity: severity_of(health),
                        cause: None,
                    };
                    updated.insert(role.clone(), incident.clone());
                    track.open = Some(Open {
                        incident,
                        opened: at,
                        probed: since_loss.is_some(),
                    });
                }
                (Some(open), true) => {
                    let severity = open.incident.severity.max(severity_of(health));
                    if severity != open.incident.severity {
                        open.incident.severity = severity;
                        updated.insert(role.clone(), open.incident.clone());
                    }
                }
                (open @ Some(_), false) => {
                    if let Some(Open {
                        mut incident,
                        probed,
                        ..
                    }) = open.take()
                    {
                        let recovered = track.recovered.filter(|_| probed);
                        incident.ended = Some(recovered.unwrap_or(at));
                        updated.insert(role.clone(), incident);
                    }
                    track.last_calm = Some(at);
                }
                (None, false) => track.last_calm = Some(at),
            }
        }

        let open: Vec<(Role, WallTime)> = self
            .roles
            .iter()
            .filter_map(|(role, track)| Some((role.clone(), track.open.as_ref()?.opened)))
            .collect();
        let grace = self.cause_grace_us;
        for (role, track) in &mut self.roles {
            let Some(open_here) = &mut track.open else {
                continue;
            };
            let latest = open_here.opened.as_unix_micros().saturating_add(grace);
            let eligible: Vec<&Role> = open
                .iter()
                .filter(|(u, opened)| u.is_upstream_of(role) && opened.as_unix_micros() <= latest)
                .map(|(u, _)| u)
                .collect();
            // Every candidate is on `role`'s path, so they are totally ordered.
            let candidate = eligible
                .iter()
                .find(|u| !eligible.iter().any(|v| v.is_upstream_of(u)))
                .copied();
            let better = candidate.filter(|c| {
                open_here
                    .incident
                    .cause
                    .as_ref()
                    .is_none_or(|cause| c.is_upstream_of(cause))
            });
            if let Some(cause) = better {
                open_here.incident.cause = Some(cause.clone());
                updated.insert(role.clone(), open_here.incident.clone());
            }
        }
        updated.into_values().collect()
    }
}
