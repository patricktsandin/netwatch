//! Grading roles, and reading loss along a path (ADR-0004, ADR-0030).

use std::collections::BTreeMap;
use std::num::NonZeroU8;

use netwatch_probing::Health;
use netwatch_probing::health::{EgressWatch, Outcomes, along_paths, grade, without_egress};
use netwatch_shared::{HopTtl, IdentityEpoch, Instant, Role, TargetId};

fn hop(target: u32, ttl: u8) -> Role {
    Role::Hop {
        target: TargetId::new(target),
        ttl: HopTtl::new(NonZeroU8::new(ttl).unwrap_or(NonZeroU8::MIN)),
    }
}

#[test]
fn grades_recent_loss() {
    assert_eq!(grade(&[], true), Health::Unknown);
    assert_eq!(grade(&[true, true], false), Health::Unknown);
    assert_eq!(grade(&[true, true], true), Health::Healthy);
    assert_eq!(grade(&[true, true, false], true), Health::Degraded);
    assert_eq!(grade(&[true, false, false], true), Health::Failing);
}

#[test]
fn too_few_outcomes_are_never_failing() {
    // The first probe on a new path, lost: degraded, not an outage (ADR-0056).
    assert_eq!(grade(&[false], true), Health::Degraded);
    assert_eq!(grade(&[true, false], true), Health::Degraded);
    assert_eq!(grade(&[false, false, false], true), Health::Failing);
}

fn epoch(n: u64) -> IdentityEpoch {
    IdentityEpoch::new(std::num::NonZeroU64::new(n).unwrap_or(std::num::NonZeroU64::MIN))
}

fn at(s: u64) -> Instant {
    Instant::from_nanos(s * 1_000_000_000)
}

#[test]
fn outcomes_start_afresh_with_a_new_epoch_and_ignore_an_older_one() {
    let mut outcomes = Outcomes::default();
    for s in 0..5 {
        outcomes.observe(epoch(1), at(s), false);
    }
    assert_eq!(outcomes.grade(at(5)), Health::Failing);
    // A failover: the new path's first probe is answered...
    outcomes.observe(epoch(2), at(6), true);
    assert_eq!(outcomes.grade(at(6)), Health::Healthy);
    // ...and the retired stream's last probe on the old path times out late.
    outcomes.observe(epoch(1), at(7), false);
    assert_eq!(outcomes.grade(at(7)), Health::Healthy);
}

#[test]
fn outcomes_go_stale() {
    let mut outcomes = Outcomes::default();
    outcomes.observe(epoch(1), at(0), true);
    assert_eq!(outcomes.grade(at(30)), Health::Healthy);
    assert_eq!(outcomes.grade(at(31)), Health::Unknown);
}

#[test]
fn no_way_out_counts_once_it_has_lasted() {
    let mut watch = EgressWatch::default();
    assert!(!watch.observe(true, at(0)));
    // Gone at 1: not yet an outage, then one (ADR-0055).
    assert!(!watch.observe(false, at(1)));
    assert!(!watch.observe(false, at(5)));
    assert!(watch.observe(false, at(6)));
    // Back, then gone again: timed afresh.
    assert!(!watch.observe(true, at(7)));
    assert!(!watch.observe(false, at(8)));
}

#[test]
fn only_the_latest_window_counts() {
    let mut recent = vec![false; 20];
    recent.extend([true; 12]);
    assert_eq!(grade(&recent, true), Health::Healthy);
}

#[test]
fn a_lossy_hop_with_a_healthy_path_beyond_it_is_rate_limited() {
    let grades = BTreeMap::from([
        (hop(1, 1), Health::Failing),
        (hop(1, 2), Health::Healthy),
        (Role::Target(TargetId::new(1)), Health::Healthy),
    ]);
    assert_eq!(along_paths(&grades)[&hop(1, 1)], Health::RateLimited);
}

#[test]
fn loss_that_carries_through_to_the_target_stays_loss() {
    let grades = BTreeMap::from([
        (hop(1, 1), Health::Failing),
        (hop(1, 2), Health::Failing),
        (Role::Target(TargetId::new(1)), Health::Failing),
        // Another target's healthy path says nothing about this one.
        (Role::Target(TargetId::new(2)), Health::Healthy),
    ]);
    let read = along_paths(&grades);
    assert_eq!(read[&hop(1, 1)], Health::Failing);
    assert_eq!(read[&Role::Target(TargetId::new(1))], Health::Failing);
}

#[test]
fn a_target_s_own_loss_is_never_reinterpreted() {
    let grades = BTreeMap::from([(Role::Target(TargetId::new(1)), Health::Failing)]);
    assert_eq!(
        along_paths(&grades)[&Role::Target(TargetId::new(1))],
        Health::Failing
    );
}

#[test]
fn worse_prefers_the_more_alarming_grade() {
    assert_eq!(Health::Healthy.worse(Health::Degraded), Health::Degraded);
    assert_eq!(Health::Unknown.worse(Health::Healthy), Health::Healthy);
    assert_eq!(Health::Failing.worse(Health::RateLimited), Health::Failing);
}

#[test]
fn without_an_egress_path_the_whole_path_is_failing() {
    // Stale, so unknown from the samples alone: the daemon knows better (ADR-0053).
    let grades = BTreeMap::from([
        (Role::Kernel, Health::Healthy),
        (Role::Target(TargetId::new(1)), Health::Unknown),
        (hop(1, 2), Health::Unknown),
        (Role::Wifi, Health::Healthy),
    ]);
    let graded = without_egress(&grades);
    assert_eq!(graded[&Role::LocalInterface], Health::Failing);
    assert_eq!(graded[&Role::Target(TargetId::new(1))], Health::Failing);
    assert_eq!(graded[&hop(1, 2)], Health::Failing);
    // The host itself, and radios off the path, are what they were.
    assert_eq!(graded[&Role::Kernel], Health::Healthy);
    assert_eq!(graded[&Role::Wifi], Health::Healthy);
}
