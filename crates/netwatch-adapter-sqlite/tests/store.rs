//! The store round-trips samples and events exactly, and its schema rules hold.

#![allow(clippy::unwrap_used)] // Tests may unwrap (ADR-0015), including in helpers.

use std::num::{NonZeroU32, NonZeroU64};
use std::path::PathBuf;

use netwatch_adapter_sqlite::{Store, StoredSample};
use netwatch_history::{Loss, Measurement, Sample};
use netwatch_shared::uom::si::f64::Time;
use netwatch_shared::uom::si::time::microsecond;
use netwatch_shared::{
    Identity, IdentityEpoch, InterfaceIndex, InterfaceName, Role, TargetId, WallTime,
};
use netwatch_topology::TopologyEvent;

fn fresh(name: &str) -> PathBuf {
    let path = PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join(format!("{name}.db"));
    for suffix in ["", "-wal", "-shm"] {
        let _ = std::fs::remove_file(format!("{}{suffix}", path.display()));
    }
    path
}

fn epoch(n: u64) -> IdentityEpoch {
    IdentityEpoch::new(NonZeroU64::new(n).unwrap())
}

fn sample(at: i64, measurement: Measurement) -> Sample {
    Sample {
        role: Role::Target(TargetId::new(1)),
        epoch: epoch(1),
        at: WallTime::from_unix_micros(at),
        measurement,
    }
}

#[test]
fn every_kind_of_measurement_round_trips() {
    let mut store = Store::open(&fresh("round-trip")).unwrap();
    let measurements = [
        Measurement::RoundTrip(Time::new::<microsecond>(3_688.0)),
        Measurement::Lost(Loss::TimedOut),
        Measurement::Lost(Loss::Unreachable(3)),
        Measurement::Lost(Loss::TimeExceeded),
        Measurement::Lost(Loss::Refused(2)),
        Measurement::Lost(Loss::UnexpectedResponse(302)),
        Measurement::Lost(Loss::ExchangeFailed(3)),
        Measurement::Lost(Loss::SendFailed),
    ];
    let samples: Vec<Sample> = (0..)
        .zip(measurements)
        .map(|(at, m)| sample(at, m))
        .collect();

    store.write_samples(&samples).unwrap();

    let read = store
        .samples(
            &Role::Target(TargetId::new(1)),
            WallTime::from_unix_micros(0),
        )
        .unwrap();
    let expected: Vec<StoredSample> = samples
        .iter()
        .map(|s| StoredSample {
            at: s.at,
            epoch: s.epoch,
            measurement: s.measurement,
        })
        .collect();
    assert_eq!(read, expected);
}

#[test]
fn epochs_are_tracked_from_their_events() {
    let mut store = Store::open(&fresh("epochs")).unwrap();
    let eth0 = Identity::Interface {
        index: InterfaceIndex::new(NonZeroU32::new(5).unwrap()),
        name: InterfaceName::parse(b"eth0").unwrap(),
    };

    store
        .record_event(
            &TopologyEvent::EpochStarted {
                epoch: epoch(7),
                role: Role::Nic,
                identity: eth0,
            },
            WallTime::from_unix_micros(10),
            false,
        )
        .unwrap();
    store
        .record_event(
            &TopologyEvent::EpochEnded {
                epoch: epoch(7),
                role: Role::Nic,
            },
            WallTime::from_unix_micros(20),
            false,
        )
        .unwrap();

    assert_eq!(store.last_epoch().unwrap(), Some(7));
    let events = store.events(WallTime::from_unix_micros(0)).unwrap();
    assert_eq!(
        events.iter().map(|e| e.kind.as_str()).collect::<Vec<_>>(),
        ["epoch-started", "epoch-ended"]
    );
    assert_eq!(events[0].detail.as_deref(), Some("eth0 (ifindex 5)"));
}

#[test]
fn a_version_one_database_migrates_without_losing_samples() {
    let path = fresh("migrate-from-1");
    {
        let connection = rusqlite::Connection::open(&path).unwrap();
        connection
            .execute_batch(include_str!("../migrations/0001-initial.sql"))
            .unwrap();
        connection
            .execute_batch(
                "INSERT INTO samples (role, epoch, at_us, rtt_us, loss, unreachable_code)
                 VALUES ('target:1', 1, 5, NULL, 'unreachable', 3);
                 PRAGMA user_version = 1;",
            )
            .unwrap();
    }

    let store = Store::open(&path).unwrap();

    let read = store
        .samples(
            &Role::Target(TargetId::new(1)),
            WallTime::from_unix_micros(0),
        )
        .unwrap();
    assert_eq!(read.len(), 1);
    assert_eq!(read[0].measurement, Measurement::Lost(Loss::Unreachable(3)));
}

#[test]
fn reopening_does_not_reapply_migrations() {
    let path = fresh("reopen");
    Store::open(&path)
        .unwrap()
        .write_samples(&[sample(1, Measurement::Lost(Loss::TimedOut))])
        .unwrap();

    let store = Store::open(&path).unwrap();

    assert_eq!(store.roles().unwrap().len(), 1);
}

#[test]
fn prune_removes_only_samples_past_retention() {
    let mut store = Store::open(&fresh("prune")).unwrap();
    let day = 24 * 3600 * 1_000_000;
    store
        .write_samples(&[
            sample(0, Measurement::Lost(Loss::TimedOut)),
            sample(6 * day, Measurement::Lost(Loss::TimedOut)),
        ])
        .unwrap();

    let removed = store.prune(WallTime::from_unix_micros(8 * day)).unwrap();

    assert_eq!(removed, 1);
}

#[test]
fn rollups_round_trip_with_their_histograms() {
    use netwatch_history::{Tier, roll_up};
    let mut store = Store::open(&fresh("rollups")).unwrap();
    let samples: Vec<Sample> = (0..90)
        .map(|i| {
            sample(
                i * 1_000_000,
                if i % 7 == 0 {
                    Measurement::Lost(Loss::TimedOut)
                } else {
                    Measurement::RoundTrip(Time::new::<microsecond>(1_000.0 + i as f64))
                },
            )
        })
        .collect();
    let rollups = roll_up(&samples, Tier::Minute);

    store.write_rollups(&rollups).unwrap();

    let read = store
        .rollups(
            &Role::Target(TargetId::new(1)),
            Tier::Minute,
            WallTime::from_unix_micros(0),
        )
        .unwrap();
    assert_eq!(read, rollups);
    assert_eq!(read.len(), 2);
}

mod incidents {
    use super::*;
    use netwatch_probing::incident::{Incident, Severity};

    fn incident(role: Role, started: i64, ended: Option<i64>) -> Incident {
        Incident {
            role,
            started: WallTime::from_unix_micros(started),
            ended: ended.map(WallTime::from_unix_micros),
            severity: Severity::Degradation,
            cause: None,
        }
    }

    #[test]
    fn an_incident_is_rewritten_in_place_as_it_changes_and_closes() {
        let mut store = Store::open(&fresh("incident-upsert")).unwrap();
        let target = Role::Target(TargetId::new(1));
        let mut open = incident(target.clone(), 100, None);
        store.write_incident(&open).unwrap();
        open.severity = Severity::Outage;
        open.cause = Some(Role::Nic);
        store.write_incident(&open).unwrap();
        open.ended = Some(WallTime::from_unix_micros(300));
        store.write_incident(&open).unwrap();
        let all = store
            .incidents(
                WallTime::from_unix_micros(0),
                WallTime::from_unix_micros(1000),
            )
            .unwrap();
        assert_eq!(all, vec![open]);
    }

    #[test]
    fn a_query_returns_every_incident_overlapping_its_span_including_open_ones() {
        let mut store = Store::open(&fresh("incident-overlap")).unwrap();
        let nic = Role::Nic;
        let before = incident(nic.clone(), 10, Some(20));
        let straddles = incident(nic.clone(), 90, Some(110));
        let inside = incident(Role::Kernel, 120, Some(130));
        let still_open = incident(Role::Tunnel, 150, None);
        let after = incident(nic, 250, Some(260));
        for i in [&before, &straddles, &inside, &still_open, &after] {
            store.write_incident(i).unwrap();
        }
        let found = store
            .incidents(
                WallTime::from_unix_micros(100),
                WallTime::from_unix_micros(200),
            )
            .unwrap();
        assert_eq!(found, vec![straddles, inside, still_open]);
    }

    #[test]
    fn an_incident_left_open_by_a_stopped_daemon_closes_at_the_last_thing_known() {
        let mut store = Store::open(&fresh("incident-left-open")).unwrap();
        store
            .write_samples(&[sample(400, Measurement::Lost(Loss::TimedOut))])
            .unwrap();
        let target = Role::Target(TargetId::new(1));
        store
            .write_incident(&incident(target.clone(), 300, None))
            .unwrap();
        store
            .write_incident(&incident(Role::Nic, 350, None))
            .unwrap();
        store
            .write_incident(&incident(Role::Kernel, 360, None))
            .unwrap();
        // The NIC has readings, not samples; the kernel neither. The newest reading
        // of any role is the last moment the run was alive.
        let reading = |role: Role, at| netwatch_history::Reading {
            role,
            epoch: epoch(3),
            at: WallTime::from_unix_micros(at),
            gauge: netwatch_shared::Gauge::RxThroughput,
            value: 1.0,
        };
        store
            .write_readings(&[reading(Role::Nic, 500), reading(Role::Wifi, 700)])
            .unwrap();
        assert_eq!(store.close_incidents_left_open().unwrap(), 3);
        let found = store
            .incidents(
                WallTime::from_unix_micros(0),
                WallTime::from_unix_micros(1000),
            )
            .unwrap();
        let ended = |role: &Role| found.iter().find(|i| &i.role == role).and_then(|i| i.ended);
        assert_eq!(ended(&target), Some(WallTime::from_unix_micros(400)));
        assert_eq!(ended(&Role::Nic), Some(WallTime::from_unix_micros(500)));
        assert_eq!(ended(&Role::Kernel), Some(WallTime::from_unix_micros(700)));
    }
}

mod readings {
    use super::*;
    use netwatch_history::{Reading, Tier, roll_up_readings};
    use netwatch_shared::Gauge;

    fn reading(at: i64, gauge: Gauge, value: f64) -> Reading {
        Reading {
            role: Role::Nic,
            epoch: epoch(3),
            at: WallTime::from_unix_micros(at),
            gauge,
            value,
        }
    }

    #[test]
    fn readings_and_their_rollups_round_trip_and_make_the_role_known() {
        let mut store = Store::open(&fresh("readings")).unwrap();
        let written = vec![
            reading(1_000_000, Gauge::RxThroughput, 1.5e6),
            reading(1_000_000, Gauge::TxThroughput, 2.0e5),
            reading(2_000_000, Gauge::RxThroughput, 3.5e6),
        ];
        store.write_readings(&written).unwrap();
        assert_eq!(
            store
                .readings(&Role::Nic, WallTime::from_unix_micros(0))
                .unwrap(),
            written
        );
        let rollups = roll_up_readings(&written, Tier::Minute);
        store.write_reading_rollups(&rollups).unwrap();
        assert_eq!(
            store
                .reading_rollups(&Role::Nic, Tier::Minute, WallTime::from_unix_micros(0))
                .unwrap(),
            rollups
        );
        let roles: Vec<Role> = store.roles().unwrap().into_iter().map(|(r, _)| r).collect();
        assert_eq!(roles, vec![Role::Nic]);
    }

    #[test]
    fn raw_readings_expire_with_raw_samples() {
        let mut store = Store::open(&fresh("readings-prune")).unwrap();
        let day = 24 * 3600 * 1_000_000_i64;
        store
            .write_readings(&[reading(0, Gauge::RxThroughput, 1.0)])
            .unwrap();
        store.prune(WallTime::from_unix_micros(8 * day)).unwrap();
        assert!(
            store
                .readings(&Role::Nic, WallTime::from_unix_micros(0))
                .unwrap()
                .is_empty()
        );
    }
}

mod flush {
    use super::*;
    use netwatch_history::Reading;
    use netwatch_shared::Gauge;

    #[test]
    fn a_flush_empties_history_but_keeps_epochs_unique() {
        let mut store = Store::open(&fresh("flush")).unwrap();
        store
            .write_samples(&[sample(1_000_000, Measurement::Lost(Loss::TimedOut))])
            .unwrap();
        store
            .write_readings(&[Reading {
                role: Role::Nic,
                epoch: epoch(1),
                at: WallTime::from_unix_micros(1_000_000),
                gauge: Gauge::RxThroughput,
                value: 1.0,
            }])
            .unwrap();
        let nic = Identity::Interface {
            index: InterfaceIndex::new(NonZeroU32::new(2).unwrap()),
            name: InterfaceName::parse(b"eth0").unwrap(),
        };
        for (n, ended) in [(5, true), (9, true), (7, false)] {
            store
                .record_event(
                    &TopologyEvent::EpochStarted {
                        epoch: epoch(n),
                        role: Role::Nic,
                        identity: nic.clone(),
                    },
                    WallTime::from_unix_micros(0),
                    false,
                )
                .unwrap();
            if ended {
                store
                    .record_event(
                        &TopologyEvent::EpochEnded {
                            epoch: epoch(n),
                            role: Role::Nic,
                        },
                        WallTime::from_unix_micros(1),
                        false,
                    )
                    .unwrap();
            }
        }
        store.flush_history().unwrap();
        assert!(store.roles().unwrap().is_empty());
        assert!(
            store
                .events(WallTime::from_unix_micros(0))
                .unwrap()
                .is_empty()
        );
        assert_eq!(
            store.last_epoch().unwrap(),
            Some(9),
            "the highest epoch is kept"
        );
        let kept = store
            .epoch_identities(
                WallTime::from_unix_micros(0),
                WallTime::from_unix_micros(i64::MAX),
            )
            .unwrap();
        let kept: Vec<u64> = kept.keys().map(|e| e.get().get()).collect();
        assert_eq!(
            kept,
            [7, 9],
            "open epochs and the highest survive; closed ones go"
        );
    }
}

#[test]
fn path_traces_round_trip_with_their_router_names_and_flush_with_history() {
    use netwatch_adapter_sqlite::StoredTrace;
    use netwatch_probing::trace::{PathTrace, TraceHop, TraceKind};
    use netwatch_shared::{HopTtl, Role, TargetId, WallTime};
    use std::num::NonZeroU8;
    let mut store = Store::open(&fresh("traces.db")).unwrap();
    let router: std::net::IpAddr = "81.2.69.142".parse().unwrap();
    let hop = |n: u8, responders: Vec<std::net::IpAddr>| TraceHop {
        ttl: HopTtl::new(NonZeroU8::new(n).unwrap()),
        responders,
        sent: 3,
        answered: 2,
        best: Some(Time::new::<microsecond>(4100.0)),
        median: Some(Time::new::<microsecond>(4800.0)),
    };
    let role = Role::Target(TargetId::new(1));
    let trace = |at: i64, kind| StoredTrace {
        role: role.clone(),
        at: WallTime::from_unix_micros(at),
        kind,
        destination: "1.1.1.1".parse().unwrap(),
        trace: PathTrace {
            hops: vec![hop(4, vec![router]), hop(5, vec![])],
            reached: false,
            silent_from: Some(HopTtl::new(NonZeroU8::new(6).unwrap())),
        },
        names: [(router, "ae1.example.net · AS64500".to_owned())].into(),
    };
    store.write_trace(&trace(10, TraceKind::Routine)).unwrap();
    store.write_trace(&trace(20, TraceKind::Episode)).unwrap();
    let all = store
        .traces(
            WallTime::from_unix_micros(0),
            WallTime::from_unix_micros(100),
        )
        .unwrap();
    assert_eq!(
        all,
        vec![trace(10, TraceKind::Routine), trace(20, TraceKind::Episode)]
    );
    assert_eq!(
        store
            .latest_trace(&role, TraceKind::Routine, WallTime::from_unix_micros(30))
            .unwrap(),
        Some(trace(10, TraceKind::Routine))
    );
    store.flush_history().unwrap();
    assert!(
        store
            .traces(
                WallTime::from_unix_micros(0),
                WallTime::from_unix_micros(100)
            )
            .unwrap()
            .is_empty()
    );
}

fn eth(name: &[u8], index: u32) -> Identity {
    Identity::Interface {
        index: InterfaceIndex::new(NonZeroU32::new(index).unwrap()),
        name: InterfaceName::parse(name).unwrap(),
    }
}

fn started(store: &mut Store, n: u64, role: Role, identity: Identity, at: i64, resumed: bool) {
    store
        .record_event(
            &TopologyEvent::EpochStarted {
                epoch: epoch(n),
                role,
                identity,
            },
            WallTime::from_unix_micros(at),
            resumed,
        )
        .unwrap();
}

fn ended(store: &mut Store, n: u64, role: Role, at: i64) {
    store
        .record_event(
            &TopologyEvent::EpochEnded {
                epoch: epoch(n),
                role,
            },
            WallTime::from_unix_micros(at),
            false,
        )
        .unwrap();
}

fn public(store: &mut Store, at: i64, detail: &str) {
    store
        .record_note(&netwatch_adapter_sqlite::StoredEvent {
            at: WallTime::from_unix_micros(at),
            kind: "public-address".to_owned(),
            role: Some("local-interface".to_owned()),
            epoch: None,
            detail: Some(detail.to_owned()),
        })
        .unwrap();
}

fn kinds(store: &Store) -> Vec<(String, Option<String>)> {
    store
        .events(WallTime::from_unix_micros(0))
        .unwrap()
        .into_iter()
        .map(|e| (e.kind, e.detail))
        .collect()
}

#[test]
fn a_resumed_epoch_is_recorded_as_resumed_and_still_starts_an_epoch() {
    let mut store = Store::open(&fresh("resumed")).unwrap();
    started(&mut store, 4, Role::Nic, eth(b"eth0", 5), 10, true);
    assert_eq!(
        kinds(&store),
        [(
            "epoch-resumed".to_owned(),
            Some("eth0 (ifindex 5)".to_owned())
        )]
    );
    assert_eq!(store.last_epoch().unwrap(), Some(4));
}

#[test]
fn the_identities_before_a_start_are_each_roles_latest() {
    let mut store = Store::open(&fresh("last-identities")).unwrap();
    started(&mut store, 1, Role::Nic, eth(b"eth0", 5), 10, false);
    ended(&mut store, 1, Role::Nic, 20);
    started(&mut store, 2, Role::Nic, eth(b"wlan0", 3), 20, false);
    started(&mut store, 3, Role::Kernel, eth(b"lo", 1), 10, false);
    let last = store.last_identities().unwrap();
    assert_eq!(
        last.get(&Role::Nic).map(String::as_str),
        Some("wlan0 (ifindex 3)")
    );
    assert_eq!(
        last.get(&Role::Kernel).map(String::as_str),
        Some("lo (ifindex 1)")
    );
    assert_eq!(last.len(), 2);
}

#[test]
fn a_role_whose_latest_epoch_ended_had_no_identity_before_a_start() {
    let mut store = Store::open(&fresh("last-identities-ended")).unwrap();
    started(&mut store, 1, Role::Nic, eth(b"eth0", 5), 10, false);
    ended(&mut store, 1, Role::Nic, 20);
    started(&mut store, 2, Role::Kernel, eth(b"lo", 1), 10, false);
    let last = store.last_identities().unwrap();
    assert_eq!(last.get(&Role::Nic), None);
    assert_eq!(
        last.get(&Role::Kernel).map(String::as_str),
        Some("lo (ifindex 1)")
    );
}

#[test]
fn an_epoch_the_run_ended_is_not_resumed_after_a_flush_either() {
    // The flush keeps the highest epoch but deletes its epoch-ended event.
    let mut store = Store::open(&fresh("last-identities-flushed")).unwrap();
    started(&mut store, 1, Role::Nic, eth(b"eth0", 5), 10, false);
    ended(&mut store, 1, Role::Nic, 20);
    store.flush_history().unwrap();
    assert_eq!(store.last_identities().unwrap().get(&Role::Nic), None);
}

#[test]
fn the_public_address_before_a_start_is_the_last_one_noted() {
    let mut store = Store::open(&fresh("last-public")).unwrap();
    assert_eq!(store.last_public_address().unwrap(), None);
    public(&mut store, 10, "192.0.2.1 (enters Cloudflare at LHR, GB)");
    public(
        &mut store,
        20,
        "192.0.2.1 (enters Cloudflare at LHR, GB) → 198.51.100.2 (enters Cloudflare at MAN, GB)",
    );
    assert_eq!(
        store.last_public_address().unwrap().as_deref(),
        Some("198.51.100.2 (enters Cloudflare at MAN, GB)")
    );
}

#[test]
fn migration_8_marks_restarts_resumed_and_drops_repeated_public_addresses() {
    let path = fresh("migrate-to-8");
    {
        let mut store = Store::open(&path).unwrap();
        // A run with eth0, a restart with eth0 again, and within that run eth0
        // going away and coming back. Then a restart with a different NIC. Events
        // are written in time order, as the daemon writes them.
        started(&mut store, 1, Role::Nic, eth(b"eth0", 5), 10, false);
        public(&mut store, 11, "A");
        started(&mut store, 2, Role::Nic, eth(b"eth0", 5), 20, false);
        public(&mut store, 21, "A");
        ended(&mut store, 2, Role::Nic, 30);
        started(&mut store, 3, Role::Nic, eth(b"eth0", 5), 40, false);
        public(&mut store, 41, "A → B");
        started(&mut store, 4, Role::Nic, eth(b"wlan0", 3), 50, false);
        public(&mut store, 51, "B");
        public(&mut store, 61, "C");
        rusqlite::Connection::open(&path)
            .unwrap()
            .pragma_update(None, "user_version", 7)
            .unwrap();
    }

    let store = Store::open(&path).unwrap();

    let s = |k: &str, d: &str| (k.to_owned(), Some(d.to_owned()));
    assert_eq!(
        kinds(&store),
        [
            s("epoch-started", "eth0 (ifindex 5)"),
            s("public-address", "A"),
            s("epoch-resumed", "eth0 (ifindex 5)"),
            ("epoch-ended".to_owned(), None),
            s("epoch-started", "eth0 (ifindex 5)"),
            s("public-address", "A → B"),
            s("epoch-started", "wlan0 (ifindex 3)"),
            s("public-address", "C"),
        ]
    );
}

#[test]
fn migration_9_makes_an_identity_back_long_after_a_start_a_change() {
    let path = fresh("migrate-to-9");
    {
        let mut store = Store::open(&path).unwrap();
        // eth0 and lo before a stop; the next start, at 100 s, closes both.
        started(&mut store, 1, Role::Nic, eth(b"eth0", 5), 10, false);
        started(&mut store, 2, Role::Kernel, eth(b"lo", 1), 10, false);
        store
            .close_epochs_left_open(WallTime::from_unix_micros(100_000_000))
            .unwrap();
        // lo is back at once; eth0 only after an outage of two minutes.
        started(
            &mut store,
            3,
            Role::Kernel,
            eth(b"lo", 1),
            101_000_000,
            true,
        );
        started(&mut store, 4, Role::Nic, eth(b"eth0", 5), 220_000_000, true);
        rusqlite::Connection::open(&path)
            .unwrap()
            .pragma_update(None, "user_version", 8)
            .unwrap();
    }

    let store = Store::open(&path).unwrap();

    let s = |k: &str, d: &str| (k.to_owned(), Some(d.to_owned()));
    assert_eq!(
        kinds(&store),
        [
            s("epoch-started", "eth0 (ifindex 5)"),
            s("epoch-started", "lo (ifindex 1)"),
            s("epoch-resumed", "lo (ifindex 1)"),
            s("epoch-started", "eth0 (ifindex 5)"),
        ]
    );
}

#[test]
fn the_highest_target_id_outlives_the_raw_samples() {
    // Target 30's raw samples are pruned after 7 days, but its epochs, rollups, and
    // incidents stay, so its id is still taken (ADR-0009).
    let mut store = Store::open(&fresh("highest-target")).unwrap();
    assert_eq!(store.highest_target_id().unwrap(), None);
    store
        .record_event(
            &TopologyEvent::EpochStarted {
                epoch: epoch(3),
                role: Role::Hop {
                    target: TargetId::new(30),
                    ttl: netwatch_shared::HopTtl::new(std::num::NonZeroU8::MIN),
                },
                identity: Identity::Address("192.0.2.1".parse().unwrap()),
            },
            WallTime::from_unix_micros(10),
            false,
        )
        .unwrap();
    store
        .write_samples(&[sample(20, Measurement::Lost(Loss::TimedOut))])
        .unwrap();
    assert_eq!(store.highest_target_id().unwrap(), Some(TargetId::new(30)));
}
