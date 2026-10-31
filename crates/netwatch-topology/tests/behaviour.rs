//! The topology aggregate's behaviour, as examples written in glossary terms.
//!
//! The exhaustive check is trace conformance against `specs/topology.qnt`
//! (`conformance.rs`). These tests are the readable account of what that spec means,
//! including the two scenarios model checking found (ADR-0021).

#![allow(clippy::unwrap_used)] // Tests may unwrap (ADR-0015), including in helpers.

use std::collections::BTreeMap;
use std::num::{NonZeroU32, NonZeroU64};

use netwatch_shared::{Identity, IdentityEpoch, InterfaceIndex, InterfaceName, Role};
use netwatch_topology::{
    Binding, Command, Observation, Output, ResyncGeneration, Snapshot, Topology, TopologyEvent,
};

fn interface(index: u32, name: &str) -> Identity {
    Identity::Interface {
        index: InterfaceIndex::new(NonZeroU32::new(index).unwrap()),
        name: InterfaceName::parse(name.as_bytes()).unwrap(),
    }
}

fn eth0() -> Identity {
    interface(2, "eth0")
}

fn wlan0() -> Identity {
    interface(3, "wlp0s20f3")
}

fn epoch(n: u64) -> IdentityEpoch {
    IdentityEpoch::new(NonZeroU64::new(n).unwrap())
}

fn generation(n: u64) -> ResyncGeneration {
    ResyncGeneration::new(n)
}

fn observed(role: Role, identity: Identity) -> Observation {
    Observation::IdentityObserved { role, identity }
}

fn started(n: u64, role: Role, identity: Identity) -> Output {
    Output::Event(TopologyEvent::EpochStarted {
        epoch: epoch(n),
        role,
        identity,
    })
}

fn ended(n: u64, role: Role) -> Output {
    Output::Event(TopologyEvent::EpochEnded {
        epoch: epoch(n),
        role,
    })
}

fn snapshot(n: u64, bindings: impl IntoIterator<Item = (Role, Identity)>) -> Observation {
    Observation::SnapshotReceived(Snapshot {
        generation: generation(n),
        bindings: bindings.into_iter().collect::<BTreeMap<_, _>>(),
    })
}

#[test]
fn a_new_view_is_synchronized_and_has_no_bindings() {
    let topology = Topology::new();

    assert!(topology.is_synchronized());
    assert_eq!(topology.binding(&Role::Nic), None);
}

#[test]
fn first_identity_observed_for_a_role_starts_epoch_one() {
    let mut topology = Topology::new();

    let outputs = topology.observe(observed(Role::Nic, eth0()));

    assert_eq!(outputs, vec![started(1, Role::Nic, eth0())]);
    assert_eq!(
        topology.binding(&Role::Nic),
        Some(&Binding {
            identity: eth0(),
            epoch: epoch(1)
        })
    );
}

#[test]
fn repeated_notification_does_not_split_an_epoch() {
    let mut topology = Topology::new();
    topology.observe(observed(Role::Nic, eth0()));

    let outputs = topology.observe(observed(Role::Nic, eth0()));

    assert_eq!(outputs, vec![]);
    assert_eq!(
        topology.binding(&Role::Nic).map(|b| b.epoch),
        Some(epoch(1))
    );
}

#[test]
fn failover_ends_the_old_epoch_before_starting_the_new_one() {
    let mut topology = Topology::new();
    topology.observe(observed(Role::Nic, eth0()));

    let outputs = topology.observe(observed(Role::Nic, wlan0()));

    assert_eq!(
        outputs,
        vec![ended(1, Role::Nic), started(2, Role::Nic, wlan0())]
    );
}

#[test]
fn role_vanishing_ends_its_epoch_and_unbinds_it() {
    let mut topology = Topology::new();
    topology.observe(observed(Role::Tunnel, interface(7, "wg0")));

    let outputs = topology.observe(Observation::RoleVanished { role: Role::Tunnel });

    assert_eq!(outputs, vec![ended(1, Role::Tunnel)]);
    assert_eq!(topology.binding(&Role::Tunnel), None);
}

#[test]
fn vanishing_of_an_unbound_role_changes_nothing() {
    let mut topology = Topology::new();

    let outputs = topology.observe(Observation::RoleVanished { role: Role::Tunnel });

    assert_eq!(outputs, vec![]);
}

#[test]
fn epochs_are_unique_across_roles() {
    let mut topology = Topology::new();
    topology.observe(observed(Role::Nic, eth0()));

    let outputs = topology.observe(observed(Role::LocalInterface, eth0()));

    assert_eq!(outputs, vec![started(2, Role::LocalInterface, eth0())]);
}

#[test]
fn notification_loss_desynchronizes_and_requests_a_resync() {
    let mut topology = Topology::new();

    let outputs = topology.observe(Observation::NotificationsLost);

    assert_eq!(
        outputs,
        vec![
            Output::Event(TopologyEvent::Desynchronized {
                generation: generation(1)
            }),
            Output::Command(Command::Resync {
                generation: generation(1)
            }),
        ]
    );
    assert!(!topology.is_synchronized());
}

#[test]
fn pins_stay_available_while_desynchronized() {
    let mut topology = Topology::new();
    topology.observe(observed(Role::Nic, eth0()));

    topology.observe(Observation::NotificationsLost);

    assert_eq!(
        topology.binding(&Role::Nic).map(|b| b.epoch),
        Some(epoch(1))
    );
}

#[test]
fn notifications_while_desynchronized_wait_for_the_snapshot() {
    let mut topology = Topology::new();
    topology.observe(observed(Role::Nic, eth0()));
    topology.observe(Observation::NotificationsLost);

    let outputs = topology.observe(observed(Role::Nic, wlan0()));

    assert_eq!(outputs, vec![]);
    assert_eq!(
        topology.binding(&Role::Nic).map(|b| &b.identity),
        Some(&eth0())
    );
}

#[test]
fn resync_reports_only_the_roles_that_changed() {
    let mut topology = Topology::new();
    topology.observe(observed(Role::Nic, eth0()));
    topology.observe(observed(Role::LocalInterface, eth0()));
    topology.observe(Observation::NotificationsLost);

    let outputs = topology.observe(snapshot(
        1,
        [(Role::Nic, wlan0()), (Role::LocalInterface, eth0())],
    ));

    assert_eq!(
        outputs,
        vec![
            ended(1, Role::Nic),
            started(3, Role::Nic, wlan0()),
            Output::Event(TopologyEvent::Resynchronized {
                generation: generation(1)
            }),
        ]
    );
    assert!(topology.is_synchronized());
}

#[test]
fn resync_ends_epochs_of_roles_missing_from_the_snapshot() {
    let mut topology = Topology::new();
    topology.observe(observed(Role::Tunnel, interface(7, "wg0")));
    topology.observe(Observation::NotificationsLost);

    let outputs = topology.observe(snapshot(1, []));

    assert_eq!(
        outputs,
        vec![
            ended(1, Role::Tunnel),
            Output::Event(TopologyEvent::Resynchronized {
                generation: generation(1)
            }),
        ]
    );
}

/// ADR-0021, counterexample 1: the snapshot was taken before the failover, but the
/// failover's notification arrived first. The newer identity must survive.
#[test]
fn notification_newer_than_the_snapshot_survives_the_resync() {
    let mut topology = Topology::new();
    topology.observe(observed(Role::Nic, eth0()));
    topology.observe(Observation::NotificationsLost);
    topology.observe(observed(Role::Nic, wlan0()));

    topology.observe(snapshot(1, [(Role::Nic, eth0())]));

    assert_eq!(
        topology.binding(&Role::Nic).map(|b| &b.identity),
        Some(&wlan0())
    );
}

#[test]
fn snapshot_from_an_earlier_generation_is_ignored() {
    let mut topology = Topology::new();
    topology.observe(Observation::NotificationsLost);
    topology.observe(Observation::NotificationsLost);

    let outputs = topology.observe(snapshot(1, [(Role::Nic, eth0())]));

    assert_eq!(outputs, vec![]);
    assert!(!topology.is_synchronized());
    assert_eq!(topology.binding(&Role::Nic), None);
}

/// ADR-0021: everything buffered before a further loss predates the snapshot that
/// loss will request, so it must not be replayed on top of it.
#[test]
fn a_further_loss_discards_notifications_buffered_before_it() {
    let mut topology = Topology::new();
    topology.observe(Observation::NotificationsLost);
    topology.observe(observed(Role::Nic, eth0()));
    topology.observe(Observation::NotificationsLost);

    topology.observe(snapshot(2, []));

    assert_eq!(topology.binding(&Role::Nic), None);
}

#[test]
fn snapshot_while_synchronized_is_ignored() {
    let mut topology = Topology::new();

    let outputs = topology.observe(snapshot(0, [(Role::Nic, eth0())]));

    assert_eq!(outputs, vec![]);
    assert_eq!(topology.binding(&Role::Nic), None);
}

#[test]
fn a_resumed_view_continues_after_the_last_stored_epoch() {
    let mut topology = Topology::resuming_after(epoch(41));

    let outputs = topology.observe(observed(Role::Nic, eth0()));

    assert_eq!(outputs, vec![started(42, Role::Nic, eth0())]);
}

fn hop() -> Role {
    Role::Hop {
        target: netwatch_shared::TargetId::new(1),
        ttl: netwatch_shared::HopTtl::new(std::num::NonZeroU8::new(1).unwrap()),
    }
}

fn router() -> Identity {
    Identity::Address("192.168.1.1".parse().unwrap())
}

#[test]
fn hop_identified_by_probing_takes_effect_even_while_desynchronized() {
    let mut topology = Topology::new();
    topology.observe(Observation::NotificationsLost);

    let outputs = topology.observe(Observation::ProbeIdentified {
        role: hop(),
        identity: router(),
    });

    assert_eq!(outputs, vec![started(1, hop(), router())]);
}

#[test]
fn resync_leaves_roles_identified_by_probing_alone() {
    let mut topology = Topology::new();
    topology.observe(Observation::ProbeIdentified {
        role: hop(),
        identity: router(),
    });
    topology.observe(Observation::NotificationsLost);

    topology.observe(snapshot(1, []));

    assert_eq!(topology.binding(&hop()).map(|b| b.epoch), Some(epoch(1)));
}
