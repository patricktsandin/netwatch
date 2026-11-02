//! Incident reports (ADR-0033): episodes, their placement, and path evidence.

#![allow(clippy::unwrap_used)] // Tests may unwrap (ADR-0015), including in helpers.

use std::collections::BTreeMap;
use std::num::NonZeroU64;

use netwatch_adapter_web::report::{
    Place, episodes, format_duration, path_stats, roles_on_paths, utc,
};
use netwatch_history::{Loss, Measurement, Sample, Tier, roll_up};
use netwatch_probing::incident::{Incident, Severity};
use netwatch_shared::uom::si::f64::Time;
use netwatch_shared::uom::si::time::millisecond;
use netwatch_shared::{IdentityEpoch, Role, WallTime};

fn role(key: &str) -> Role {
    Role::from_key(key).unwrap()
}

fn s(seconds: i64) -> WallTime {
    WallTime::from_unix_micros(seconds * 1_000_000)
}

fn incident(
    key: &str,
    started: i64,
    ended: Option<i64>,
    severity: Severity,
    cause: Option<&str>,
) -> Incident {
    Incident {
        role: role(key),
        started: s(started),
        ended: ended.map(s),
        severity,
        cause: cause.map(role),
    }
}

#[test]
fn incidents_linked_by_probable_cause_are_one_episode_with_the_root_as_its_cause() {
    let found = episodes(&[
        incident(
            "target:1",
            105,
            Some(150),
            Severity::Degradation,
            Some("hop:1:2"),
        ),
        incident("hop:1:2", 100, Some(140), Severity::Outage, None),
        incident("hop:1:3", 102, Some(170), Severity::Outage, Some("hop:1:2")),
        incident("target:1", 300, Some(310), Severity::Degradation, None),
    ]);
    assert_eq!(found.len(), 2);
    let first = &found[0];
    assert_eq!((first.started, first.ended), (s(100), Some(s(170))));
    assert_eq!(first.severity, Severity::Outage);
    assert_eq!(first.cause, role("hop:1:2"));
    assert_eq!(
        first.affected,
        [role("hop:1:2"), role("hop:1:3"), role("target:1")]
    );
    assert_eq!(first.place, Place::BeyondGateway);
    assert_eq!(found[1].cause, role("target:1"));
}

#[test]
fn overlapping_incidents_without_a_cause_between_them_are_separate_episodes() {
    let found = episodes(&[
        incident("target:9", 0, None, Severity::Outage, None),
        incident("nic", 100, Some(200), Severity::Degradation, None),
        incident("target:1", 101, Some(190), Severity::Outage, Some("nic")),
    ]);
    assert_eq!(found.len(), 2);
    assert_eq!(found[0].affected, [role("target:9")]);
    assert_eq!(found[0].ended, None);
    assert_eq!(found[1].cause, Role::Nic);
    assert_eq!(found[1].affected, [Role::Nic, role("target:1")]);
}

#[test]
fn an_episode_caused_at_or_before_the_gateway_is_on_this_host_or_lan() {
    for cause in ["kernel", "nic", "local-interface", "hop:3:1"] {
        let found = episodes(&[
            incident("target:3", 10, Some(20), Severity::Outage, Some(cause)),
            incident(cause, 9, Some(21), Severity::Outage, None),
        ]);
        assert_eq!(found[0].place, Place::HostOrLan, "{cause}");
    }
}

#[test]
fn the_roles_on_a_path_run_from_the_host_out_to_the_target() {
    let known: Vec<Role> = [
        "target:2", "hop:2:1", "kernel", "nic", "hop:1:1", "hop:1:2", "target:1",
    ]
    .iter()
    .map(|k| role(k))
    .collect();
    assert_eq!(
        roles_on_paths(&[role("target:1")], &known),
        [
            role("kernel"),
            role("nic"),
            role("hop:1:1"),
            role("hop:1:2"),
            role("target:1")
        ]
    );
}

#[test]
fn path_stats_count_sent_and_lost_with_median_rtt_for_each_epoch_apart() {
    let epoch = |n| IdentityEpoch::new(NonZeroU64::new(n).unwrap());
    let sample = |at, e, m| Sample {
        role: role("hop:1:2"),
        epoch: epoch(e),
        at: s(at),
        measurement: m,
    };
    let rtt = |ms| Measurement::RoundTrip(Time::new::<millisecond>(ms));
    let rollups = roll_up(
        &[
            sample(1, 7, rtt(10.0)),
            sample(2, 7, rtt(10.0)),
            sample(3, 7, Measurement::Lost(Loss::TimedOut)),
            sample(4, 8, rtt(30.0)),
        ],
        Tier::Minute,
    );
    let identities = BTreeMap::from([
        (epoch(7), "100.65.192.1".to_owned()),
        (epoch(8), "100.65.192.9".to_owned()),
    ]);
    let stats = path_stats(&rollups, &identities);
    // Two routers at hop 2 are two rows: no median spans them (ADR-0003).
    assert_eq!(stats.len(), 2);
    let (first, second) = (&stats[0], &stats[1]);
    assert_eq!((first.sent, first.lost), (3, 1));
    assert_eq!(first.identity.as_deref(), Some("100.65.192.1"));
    let median = first.median_ms.unwrap();
    assert!((9.0..11.0).contains(&median), "{median}");
    assert_eq!((second.sent, second.lost), (1, 0));
    assert_eq!(second.identity.as_deref(), Some("100.65.192.9"));
    let median = second.median_ms.unwrap();
    assert!((27.0..33.0).contains(&median), "{median}");
}

#[test]
fn times_are_written_in_utc_and_durations_in_two_units() {
    assert_eq!(utc(s(1_793_545_384)), "2026-11-01 15:03:04 UTC");
    assert_eq!(utc(s(0)), "1970-01-01 00:00:00 UTC");
    assert_eq!(format_duration(126_000_000), "2 min 6 s");
    assert_eq!(format_duration(3_600_000_000), "1 h");
    assert_eq!(format_duration(500_000), "0 s");
}

#[test]
fn the_first_public_hop_comes_from_the_hops_or_else_from_a_trace_beyond_them() {
    use netwatch_adapter_sqlite::StoredTrace;
    use netwatch_adapter_web::report::first_public_hop;
    use netwatch_probing::trace::{PathTrace, TraceHop, TraceKind};
    use netwatch_shared::{HopTtl, TargetId};
    let ip = |a: &str| a.parse::<std::net::IpAddr>().unwrap();
    let cgnat = [
        (1, ip("192.168.1.1")),
        (3, ip("100.127.3.193")),
        (2, ip("100.65.192.1")),
    ];
    let trace = StoredTrace {
        role: Role::Target(TargetId::new(1)),
        at: s(0),
        kind: TraceKind::Routine,
        destination: ip("1.1.1.1"),
        trace: PathTrace {
            hops: [(4, "100.127.0.9"), (5, "81.2.69.142")]
                .map(|(ttl, a)| TraceHop {
                    ttl: HopTtl::new(std::num::NonZeroU8::new(ttl).unwrap()),
                    responders: vec![ip(a)],
                    sent: 3,
                    answered: 3,
                    best: None,
                    median: None,
                })
                .to_vec(),
            reached: false,
            silent_from: None,
        },
        names: Default::default(),
    };
    assert_eq!(first_public_hop(&cgnat, &[]), None);
    assert_eq!(
        first_public_hop(&cgnat, std::slice::from_ref(&trace)),
        Some((5, ip("81.2.69.142")))
    );
    let direct = [(2, ip("81.2.69.1")), (1, ip("192.168.1.1"))];
    assert_eq!(
        first_public_hop(&direct, &[trace]),
        Some((2, ip("81.2.69.1")))
    );
}
