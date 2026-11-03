//! Behaviour of path traces and the trace schedule (ADR-0040).

#![allow(clippy::unwrap_used)] // Tests may unwrap (ADR-0015).

use std::net::IpAddr;
use std::num::NonZeroU8;

use netwatch_probing::incident::{Incident, Severity};
use netwatch_probing::trace::{
    Next, TraceKind, TracePath, TraceProbe, TraceQueue, TraceRequest, TraceSchedule, assemble,
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

#[test]
fn a_queued_trace_waits_for_the_path_to_settle_and_runs_on_it() {
    let mut queue = TraceQueue::default();
    queue.push(routine());
    assert_eq!(queue.next(at(0), |_| false), None, "no path yet");
    queue.path(Some(&path("192.0.2.20")), at(1));
    assert_eq!(queue.next(at(10), |_| false), None, "not settled");
    assert_eq!(
        queue.next(at(11), |_| false),
        Some(Next::Run(routine(), path("192.0.2.20")))
    );
}

#[test]
fn any_change_of_path_starts_the_settling_afresh() {
    // Straight from one network to another between two looks (ADR-0059).
    let mut queue = TraceQueue::default();
    queue.path(Some(&path("192.0.2.20")), at(0));
    queue.path(Some(&path("192.0.2.20")), at(30));
    queue.path(Some(&path("10.20.0.7")), at(31));
    queue.push(routine());
    assert_eq!(queue.next(at(40), |_| false), None);
    assert!(queue.next(at(41), |_| false).is_some());
}

#[test]
fn traces_run_one_at_a_time_and_each_is_checked_when_it_starts() {
    let mut queue = TraceQueue::default();
    queue.path(Some(&path("192.0.2.20")), at(0));
    queue.push(episode().unwrap());
    queue.push(episode().unwrap());
    assert!(matches!(queue.next(at(10), |_| true), Some(Next::Run(..))));
    assert_eq!(queue.next(at(11), |_| true), None, "one is running");
    queue.finished(true);
    // By the time the second could start, the incident it was for has ended.
    assert_eq!(
        queue.next(at(20), |_| false),
        Some(Next::Drop(episode().unwrap()))
    );
}

#[test]
fn the_schedule_says_whether_a_target_has_an_incident_open() {
    let mut schedule = schedule();
    let hop = |n| Role::Hop {
        target: T1,
        ttl: ttl(n),
    };
    schedule.incident(&incident(hop(1), true), at(0));
    assert!(
        !schedule.has_open(T1),
        "hop 1 is before what a trace measures"
    );
    schedule.incident(&incident(hop(3), true), at(0));
    assert!(schedule.has_open(T1));
    schedule.incident(&incident(hop(3), false), at(5));
    assert!(!schedule.has_open(T1));
}

#[test]
fn an_episode_trace_goes_ahead_of_waiting_routine_traces() {
    // Every target's routine trace comes due at once; an incident's trace shouldn't
    // wait behind them all (ADR-0060).
    let other = |n| TraceRequest {
        target: TargetId::new(n),
        kind: TraceKind::Routine,
    };
    let mut queue = TraceQueue::default();
    queue.path(Some(&path("192.0.2.20")), at(0));
    queue.push_all([other(2), other(3)]);
    queue.push(episode().unwrap());
    let first = queue.next(at(10), |_| true);
    assert_eq!(
        first,
        Some(Next::Run(episode().unwrap(), path("192.0.2.20")))
    );
}

#[test]
fn an_incident_on_a_target_whose_routine_trace_waits_makes_it_the_episode_trace() {
    // The schedule counts the waiting routine trace as running, so the incident's
    // trace would come only after it, and the routine trace would measure the outage
    // as the normal path (ADR-0061).
    let mut schedule = schedule();
    let mut queue = TraceQueue::default();
    queue.path(Some(&path("192.0.2.20")), at(0));
    queue.push_all(schedule.tick(at(0)));
    let hop3 = Role::Hop {
        target: T1,
        ttl: ttl(3),
    };
    assert_eq!(schedule.incident(&incident(hop3, true), at(1)), None);
    assert!(schedule.has_pending(T1));
    queue.upgrade(T1);
    let first = queue.next(at(10), |t| schedule.has_open(t));
    assert_eq!(
        first,
        Some(Next::Run(episode().unwrap(), path("192.0.2.20")))
    );
    queue.finished(true);
    // The schedule hands on the pending episode trace: it already ran.
    queue.push_all(schedule.finished(T1, at(20)));
    assert_eq!(
        queue.next(at(20), |t| schedule.has_open(t)),
        Some(Next::Drop(episode().unwrap()))
    );
}

#[test]
fn a_routine_trace_waits_while_its_target_has_an_incident_open() {
    // In its cooldown, an incident gets no episode trace; the routine trace mustn't
    // run during it and be taken for the normal path (ADR-0062). Others go ahead.
    let mut queue = TraceQueue::default();
    queue.path(Some(&path("192.0.2.20")), at(0));
    let two = TraceRequest {
        target: TargetId::new(2),
        kind: TraceKind::Routine,
    };
    queue.push_all([routine(), two]);
    let target_one_open = |t: TargetId| t == T1;
    assert_eq!(
        queue.next(at(10), target_one_open),
        Some(Next::Run(two, path("192.0.2.20")))
    );
    queue.finished(true);
    assert_eq!(queue.next(at(11), target_one_open), None, "target 1 waits");
    assert_eq!(
        queue.next(at(12), |_| false),
        Some(Next::Run(routine(), path("192.0.2.20")))
    );
}

#[test]
fn an_upgraded_trace_that_stored_nothing_leaves_the_episode_trace_to_run() {
    let mut schedule = schedule();
    let mut queue = TraceQueue::default();
    queue.path(Some(&path("192.0.2.20")), at(0));
    queue.push_all(schedule.tick(at(0)));
    let hop3 = Role::Hop {
        target: T1,
        ttl: ttl(3),
    };
    schedule.incident(&incident(hop3, true), at(1));
    queue.upgrade(T1);
    assert!(matches!(
        queue.next(at(10), |t| schedule.has_open(t)),
        Some(Next::Run(..))
    ));
    // The path changed under it, and it was dropped (ADR-0060).
    queue.finished(false);
    queue.push_all(schedule.finished(T1, at(20)));
    assert_eq!(
        queue.next(at(20), |t| schedule.has_open(t)),
        Some(Next::Run(episode().unwrap(), path("192.0.2.20")))
    );
}
