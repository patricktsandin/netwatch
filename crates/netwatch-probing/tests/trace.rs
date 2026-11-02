//! Behaviour of path traces and the trace schedule (ADR-0040).

#![allow(clippy::unwrap_used)] // Tests may unwrap (ADR-0015).

use std::net::IpAddr;
use std::num::NonZeroU8;

use netwatch_probing::incident::{Incident, Severity};
use netwatch_probing::trace::{TraceKind, TraceProbe, TraceRequest, TraceSchedule, assemble};
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
