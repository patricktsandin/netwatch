//! Behaviour of path traces and the trace schedule (ADR-0040).

#![allow(clippy::unwrap_used)] // Tests may unwrap (ADR-0015).

use std::net::IpAddr;
use std::num::NonZeroU8;

use netwatch_probing::incident::{Incident, Severity};
use netwatch_probing::trace::{
    Start, TraceKind, TracePath, TraceProbe, TraceQueue, TraceRequest, TraceSchedule, assemble,
};
use netwatch_shared::uom::si::f64::Time;
use netwatch_shared::uom::si::time::{millisecond, second};
use netwatch_shared::{HopTtl, Instant, Role, TargetId, WallTime};

fn at(seconds: u64) -> Instant {
    Instant::from_nanos(seconds * 1_000_000_000)
}

fn ttl(n: u8) -> HopTtl {
    HopTtl::new(NonZeroU8::new(n).unwrap())
}

fn schedule() -> TraceSchedule {
    TraceSchedule::new(
        [TargetId::new(1)],
        Time::new::<second>(600.0),
        Time::new::<second>(6.0 * 3600.0),
        at(0),
    )
}

fn incident(role: Role, open: bool) -> Incident {
    Incident {
        role,
        started: WallTime::from_unix_micros(0),
        ended: (!open).then(|| WallTime::from_unix_micros(1)),
        severity: Severity::Outage,
        cause: None,
    }
}

const T1: TargetId = TargetId::new(1);

fn episode() -> Option<TraceRequest> {
    Some(TraceRequest {
        target: T1,
        kind: TraceKind::Episode,
    })
}

#[test]
fn every_target_gets_a_routine_trace_at_start_and_again_after_the_interval() {
    let mut s = schedule();
    assert_eq!(
        s.tick(at(0)),
        vec![TraceRequest {
            target: T1,
            kind: TraceKind::Routine
        }]
    );
    assert_eq!(s.finished(T1, at(3)), None);
    assert!(s.tick(at(3600)).is_empty());
    assert_eq!(s.tick(at(6 * 3600)).len(), 1);
}

#[test]
fn an_incident_beyond_hop_1_gets_one_episode_trace_per_cooldown() {
    let mut s = schedule();
    let hop2 = Role::Hop {
        target: T1,
        ttl: ttl(2),
    };
    assert_eq!(s.incident(&incident(hop2.clone(), true), at(10)), episode());
    s.finished(T1, at(13));
    // Updates to an open incident, and reopening within the cooldown, start nothing.
    assert_eq!(s.incident(&incident(hop2.clone(), true), at(20)), None);
    assert_eq!(s.incident(&incident(hop2.clone(), false), at(30)), None);
    assert_eq!(s.incident(&incident(hop2.clone(), true), at(40)), None);
    s.incident(&incident(hop2.clone(), false), at(50));
    assert_eq!(
        s.incident(&incident(Role::Target(T1), true), at(611)),
        episode()
    );
}

#[test]
fn incidents_at_hop_1_or_on_the_host_start_no_trace() {
    let mut s = schedule();
    let hop1 = Role::Hop {
        target: T1,
        ttl: ttl(1),
    };
    assert_eq!(s.incident(&incident(hop1, true), at(10)), None);
    assert_eq!(s.incident(&incident(Role::Nic, true), at(10)), None);
    assert_eq!(
        s.incident(&incident(Role::Target(TargetId::new(9)), true), at(10)),
        None
    );
}

#[test]
fn an_episode_trace_waits_for_a_running_routine_trace() {
    let mut s = schedule();
    s.tick(at(0));
    assert_eq!(s.incident(&incident(Role::Target(T1), true), at(1)), None);
    assert_eq!(s.finished(T1, at(3)), episode());
    assert_eq!(s.finished(T1, at(6)), None);
}

fn probe(n: u8, from: Option<&str>, ms: Option<f64>, ends: bool) -> TraceProbe {
    TraceProbe {
        ttl: ttl(n),
        responder: from.map(|a| a.parse::<IpAddr>().unwrap()),
        rtt: ms.map(Time::new::<millisecond>),
        ends_path: ends,
    }
}

#[test]
fn a_trace_ends_where_the_destination_answered() {
    let trace = assemble(&[
        probe(5, Some("9.9.9.9"), Some(9.0), true),
        probe(4, Some("10.0.0.4"), Some(5.0), false),
        probe(4, None, None, false),
        probe(4, Some("10.0.0.5"), Some(7.0), false),
        probe(6, Some("9.9.9.9"), Some(9.0), true),
    ]);
    assert!(trace.reached);
    assert_eq!(trace.silent_from, None);
    assert_eq!(trace.hops.len(), 2);
    let four = &trace.hops[0];
    assert_eq!((four.sent, four.answered), (3, 2));
    assert_eq!(
        four.responders.len(),
        2,
        "load balancing shows both routers"
    );
    assert_eq!(four.best, Some(Time::new::<millisecond>(5.0)));
}

#[test]
fn silent_hops_after_the_last_answer_collapse_into_where_silence_began() {
    let trace = assemble(&[
        probe(4, Some("10.0.0.4"), Some(5.0), false),
        probe(5, None, None, false),
        probe(6, Some("10.0.0.6"), Some(6.0), false),
        probe(7, None, None, false),
        probe(8, None, None, false),
    ]);
    assert!(!trace.reached);
    assert_eq!(
        trace.hops.iter().map(|h| h.ttl).collect::<Vec<_>>(),
        vec![ttl(4), ttl(5), ttl(6)]
    );
    assert_eq!(trace.silent_from, Some(ttl(7)));
    let nothing = assemble(&[probe(4, None, None, false)]);
    assert!(nothing.hops.is_empty());
    assert_eq!(nothing.silent_from, Some(ttl(4)));
}

fn path(local: &str) -> TracePath {
    TracePath::new(
        &netwatch_shared::Identity::Interface {
            index: netwatch_shared::InterfaceIndex::new(std::num::NonZeroU32::new(3).unwrap()),
            name: netwatch_shared::InterfaceName::parse(b"wlan0").unwrap(),
        },
        local.parse().unwrap(),
    )
}

fn routine() -> TraceRequest {
    TraceRequest {
        target: T1,
        kind: TraceKind::Routine,
    }
}

fn hop(n: u8) -> Role {
    Role::Hop {
        target: T1,
        ttl: ttl(n),
    }
}

/// A queue over a schedule of targets 1 and 2, on a path known from 0 s: routine
/// traces of both come due at 0 s, and may start from 10 s.
fn queue() -> TraceQueue {
    let mut queue = TraceQueue::new(TraceSchedule::new(
        [T1, TargetId::new(2)],
        Time::new::<second>(600.0),
        Time::new::<second>(6.0 * 3600.0),
        at(0),
    ));
    queue.path(Some(&path("192.0.2.20")), at(0));
    queue
}

fn start(request: TraceRequest) -> Option<Start> {
    Some(Start {
        request,
        path: path("192.0.2.20"),
    })
}

fn of(target: u32, kind: TraceKind) -> TraceRequest {
    TraceRequest {
        target: TargetId::new(target),
        kind,
    }
}

#[test]
fn a_trace_waits_for_the_path_to_settle_and_runs_on_it() {
    let mut queue = queue();
    queue.tick(at(0));
    assert_eq!(queue.next(at(9)), None, "not settled");
    assert_eq!(queue.next(at(10)), start(routine()));
}

#[test]
fn nothing_starts_without_a_path_and_any_change_of_path_starts_the_settling_afresh() {
    let mut queue = queue();
    queue.tick(at(0));
    queue.path(None, at(5));
    assert_eq!(queue.next(at(20)), None, "no path");
    queue.path(Some(&path("192.0.2.20")), at(20));
    // Straight from one network to another between two looks (ADR-0059).
    queue.path(Some(&path("10.20.0.7")), at(25));
    assert_eq!(queue.next(at(34)), None);
    assert_eq!(queue.next(at(35)).map(|s| s.path), Some(path("10.20.0.7")));
}

#[test]
fn traces_run_one_at_a_time() {
    let mut queue = queue();
    queue.tick(at(0));
    assert_eq!(queue.next(at(10)), start(routine()));
    assert_eq!(queue.next(at(11)), None, "one is running");
    queue.finished(true, at(15));
    assert_eq!(queue.next(at(15)), start(of(2, TraceKind::Routine)));
}

#[test]
fn an_episode_trace_goes_ahead_of_waiting_routine_traces() {
    let mut queue = queue();
    queue.tick(at(0));
    assert_eq!(queue.next(at(10)), start(routine()));
    queue.finished(true, at(14));
    // Target 2's routine trace waits; target 1 gets an incident beyond hop 1.
    queue.incident(&incident(hop(3), true), false, at(20));
    assert_eq!(queue.next(at(20)), start(of(1, TraceKind::Episode)));
}

#[test]
fn an_episode_trace_whose_incident_is_over_when_it_could_start_is_dropped() {
    let mut queue = queue();
    queue.tick(at(0));
    assert_eq!(queue.next(at(10)), start(routine()));
    queue.finished(true, at(14));
    // The incident opens and ends while there is no path to trace on.
    queue.path(None, at(15));
    queue.incident(&incident(hop(3), true), false, at(16));
    queue.incident(&incident(hop(3), false), false, at(18));
    queue.path(Some(&path("192.0.2.20")), at(20));
    assert_eq!(queue.next(at(30)), start(of(2, TraceKind::Routine)));
    queue.finished(true, at(34));
    assert_eq!(queue.next(at(34)), None);
}

#[test]
fn a_routine_trace_waits_while_its_target_has_an_incident_open() {
    // Even in the cooldown, when no episode trace is coming (ADR-0062).
    let mut queue = queue();
    queue.tick(at(0));
    queue.incident(&incident(hop(3), true), false, at(1));
    // Target 1's routine trace became its episode trace (ADR-0061); it runs, and
    // the incident goes on.
    assert_eq!(queue.next(at(10)), start(of(1, TraceKind::Episode)));
    queue.finished(true, at(14));
    assert_eq!(queue.next(at(14)), start(of(2, TraceKind::Routine)));
    queue.finished(true, at(18));
    assert_eq!(
        queue.next(at(18)),
        None,
        "the handed-on duplicate was dropped"
    );
}

#[test]
fn an_upgraded_trace_that_stored_nothing_leaves_the_episode_trace_to_run() {
    let mut queue = queue();
    queue.tick(at(0));
    queue.incident(&incident(hop(3), true), false, at(1));
    assert_eq!(queue.next(at(10)), start(of(1, TraceKind::Episode)));
    // The path changed under it, and it was dropped (ADR-0060).
    queue.finished(false, at(14));
    assert_eq!(queue.next(at(14)), start(of(1, TraceKind::Episode)));
}

#[test]
fn a_shared_hops_copy_keeps_its_targets_routine_trace_waiting_without_a_trace_of_its_own() {
    // Target 2 shares target 1's hop 3 stream (ADR-0043): its copy's incident asks
    // for no trace, but target 2 is in an outage all the same.
    let mut queue = queue();
    queue.tick(at(0));
    let copy = Role::Hop {
        target: TargetId::new(2),
        ttl: ttl(3),
    };
    queue.incident(&incident(copy.clone(), true), true, at(1));
    assert_eq!(queue.next(at(10)), start(routine()));
    queue.finished(true, at(14));
    assert_eq!(queue.next(at(14)), None, "target 2 waits");
    queue.incident(&incident(copy, false), true, at(20));
    assert_eq!(queue.next(at(20)), start(of(2, TraceKind::Routine)));
}

#[test]
fn the_schedule_says_whether_a_target_has_an_incident_open_or_a_trace_held_back() {
    let mut schedule = schedule();
    schedule.incident(&incident(hop(1), true), at(0));
    assert!(
        !schedule.has_open(T1),
        "hop 1 is before what a trace measures"
    );
    schedule.tick(at(0));
    schedule.incident(&incident(hop(3), true), at(0));
    assert!(schedule.has_open(T1));
    assert!(
        schedule.has_pending(T1),
        "held back behind the routine trace"
    );
    schedule.incident(&incident(hop(3), false), at(5));
    assert!(!schedule.has_open(T1));
}
