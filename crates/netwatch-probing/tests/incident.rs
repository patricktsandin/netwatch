//! Incidents: a role's degraded and failing periods (ADR-0031,
//! `specs/incidents.qnt`).

#![allow(clippy::unwrap_used)] // Tests may unwrap (ADR-0015), including in helpers.

use std::collections::BTreeMap;
use std::num::NonZeroU8;

use netwatch_probing::Health;
use netwatch_probing::incident::{Incident, IncidentTracker, Severity};
use netwatch_shared::{HopTtl, Role, TargetId, WallTime};

fn at(seconds: i64) -> WallTime {
    WallTime::from_unix_micros(seconds * 1_000_000)
}

fn hop1() -> Role {
    Role::Hop {
        target: TargetId::new(1),
        ttl: HopTtl::new(NonZeroU8::MIN),
    }
}

fn target() -> Role {
    Role::Target(TargetId::new(1))
}

fn graded(pairs: &[(Role, Health)]) -> BTreeMap<Role, Health> {
    pairs.iter().cloned().collect()
}

#[test]
fn a_lost_probe_opens_an_incident_that_starts_at_the_loss() {
    let mut tracker = IncidentTracker::new();
    tracker.grade(at(9), &graded(&[(target(), Health::Healthy)]));
    tracker.outcome(&target(), at(10), false);
    let updates = tracker.grade(at(11), &graded(&[(target(), Health::Degraded)]));
    assert_eq!(
        updates,
        vec![Incident {
            role: target(),
            started: at(10),
            ended: None,
            severity: Severity::Degradation,
            cause: None,
        }]
    );
}

#[test]
fn the_band_ends_at_the_first_answer_after_the_last_loss_not_when_the_grade_recovers() {
    let mut tracker = IncidentTracker::new();
    tracker.outcome(&target(), at(10), false);
    tracker.grade(at(11), &graded(&[(target(), Health::Degraded)]));
    tracker.outcome(&target(), at(15), false);
    tracker.outcome(&target(), at(20), true);
    tracker.outcome(&target(), at(25), true);
    assert!(
        tracker
            .grade(at(60), &graded(&[(target(), Health::Degraded)]))
            .is_empty()
    );
    let updates = tracker.grade(at(80), &graded(&[(target(), Health::Healthy)]));
    assert_eq!(updates.len(), 1);
    assert_eq!(updates[0].started, at(10));
    assert_eq!(updates[0].ended, Some(at(20)));
}

#[test]
fn a_layer_graded_without_probes_ends_at_the_grading_that_finds_it_calm() {
    let mut tracker = IncidentTracker::new();
    tracker.grade(at(5), &graded(&[(Role::Nic, Health::Degraded)]));
    let updates = tracker.grade(at(9), &graded(&[(Role::Nic, Health::Healthy)]));
    assert_eq!(updates[0].started, at(5));
    assert_eq!(updates[0].ended, Some(at(9)));
}

#[test]
fn severity_escalates_to_outage_and_never_back() {
    let mut tracker = IncidentTracker::new();
    tracker.grade(at(1), &graded(&[(target(), Health::Degraded)]));
    let escalated = tracker.grade(at(2), &graded(&[(target(), Health::Failing)]));
    assert_eq!(escalated[0].severity, Severity::Outage);
    assert!(
        tracker
            .grade(at(3), &graded(&[(target(), Health::Degraded)]))
            .is_empty()
    );
    let closed = tracker.grade(at(4), &graded(&[(target(), Health::Healthy)]));
    assert_eq!(closed[0].severity, Severity::Outage);
}

#[test]
fn the_probable_cause_is_the_most_upstream_role_with_an_open_incident() {
    let mut tracker = IncidentTracker::new();
    tracker.grade(at(1), &graded(&[(hop1(), Health::Failing)]));
    let updates = tracker.grade(
        at(2),
        &graded(&[
            (Role::Nic, Health::Failing),
            (hop1(), Health::Failing),
            (target(), Health::Failing),
        ]),
    );
    let of = |role: Role| updates.iter().find(|i| i.role == role).unwrap().clone();
    assert_eq!(of(target()).cause, Some(Role::Nic));
    assert_eq!(of(hop1()).cause, Some(Role::Nic));
    assert_eq!(of(Role::Nic).cause, None);
}

#[test]
fn rate_limiting_and_unknown_close_an_incident_like_health_does() {
    for calm in [Health::RateLimited, Health::Unknown, Health::Healthy] {
        let mut tracker = IncidentTracker::new();
        tracker.grade(at(1), &graded(&[(hop1(), Health::Degraded)]));
        let updates = tracker.grade(at(2), &graded(&[(hop1(), calm)]));
        assert_eq!(updates[0].ended, Some(at(2)), "{calm:?}");
    }
}

#[test]
fn a_role_that_stops_being_graded_is_calm() {
    let mut tracker = IncidentTracker::new();
    tracker.grade(at(1), &graded(&[(Role::Tunnel, Health::Failing)]));
    let updates = tracker.grade(at(2), &BTreeMap::new());
    assert_eq!(updates[0].ended, Some(at(2)));
}
