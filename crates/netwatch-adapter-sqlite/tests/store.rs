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
        )
        .unwrap();
    store
        .record_event(
            &TopologyEvent::EpochEnded {
                epoch: epoch(7),
                role: Role::Nic,
            },
            WallTime::from_unix_micros(20),
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
