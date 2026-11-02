//! Behaviour of the network map (ADR-0042).

#![allow(clippy::unwrap_used)] // Tests may unwrap (ADR-0015).

use std::collections::{BTreeMap, BTreeSet, VecDeque};
use std::net::IpAddr;
use std::num::NonZeroU8;

use netwatch_probing::Health;
use netwatch_probing::map::{EdgeKind, LatestTrace, MapEdge, MapNode, NetworkMap, Place, build};
use netwatch_probing::trace::{PathTrace, TraceHop};
use netwatch_shared::uom::si::f64::Time;
use netwatch_shared::uom::si::time::millisecond;
use netwatch_shared::{HopTtl, Role, TargetId};
use proptest::prelude::*;

fn ttl(n: u8) -> HopTtl {
    HopTtl::new(NonZeroU8::new(n).unwrap())
}

fn hop(target: u32, n: u8) -> Role {
    Role::Hop {
        target: TargetId::new(target),
        ttl: ttl(n),
    }
}

fn target(id: u32) -> Role {
    Role::Target(TargetId::new(id))
}

fn ip(text: &str) -> IpAddr {
    text.parse().unwrap()
}

fn router(address: &str, n: u8) -> Place {
    Place::Router {
        address: ip(address),
        ttl: ttl(n),
    }
}

fn node<'a>(map: &'a NetworkMap, place: &Place) -> &'a MapNode {
    map.nodes
        .iter()
        .find(|n| &n.place == place)
        .unwrap_or_else(|| panic!("no node at {place:?} in {map:#?}"))
}

fn edge<'a>(map: &'a NetworkMap, from: &Place, to: &Place) -> &'a MapEdge {
    map.edges
        .iter()
        .find(|e| &e.from == from && &e.to == to)
        .unwrap_or_else(|| panic!("no edge {from:?} → {to:?} in {map:#?}"))
}

fn trace_hop(n: u8, responders: &[&str]) -> TraceHop {
    TraceHop {
        ttl: ttl(n),
        responders: responders.iter().map(|r| ip(r)).collect(),
        sent: 3,
        answered: if responders.is_empty() { 0 } else { 3 },
        best: None,
        median: (!responders.is_empty()).then(|| Time::new::<millisecond>(8.0)),
    }
}

fn latest(destination: &str, hops: Vec<TraceHop>, silent_from: Option<u8>) -> LatestTrace {
    let reached = hops.iter().any(|h| h.responders.contains(&ip(destination)));
    LatestTrace {
        destination: ip(destination),
        trace: PathTrace {
            hops,
            reached,
            silent_from: silent_from.map(ttl),
        },
    }
}

/// The host's layers, healthy, plus `extra`.
fn statuses(extra: &[(Role, Health)]) -> BTreeMap<Role, Health> {
    [Role::Kernel, Role::Nic, Role::LocalInterface]
        .into_iter()
        .map(|r| (r, Health::Healthy))
        .chain(extra.iter().cloned())
        .collect()
}

fn targets(ids: &[u32]) -> BTreeSet<TargetId> {
    ids.iter().copied().map(TargetId::new).collect()
}

#[test]
fn the_hosts_layers_form_a_probed_trunk_in_path_order() {
    let map = build(&statuses(&[]), &BTreeMap::new(), &BTreeMap::new());
    let kernel = Place::Role(Role::Kernel);
    let nic = Place::Role(Role::Nic);
    let local = Place::Role(Role::LocalInterface);
    assert_eq!(node(&map, &kernel).depth, 0);
    assert_eq!(node(&map, &nic).depth, 1);
    assert_eq!(node(&map, &local).depth, 3);
    assert_eq!(edge(&map, &kernel, &nic).kind, EdgeKind::Probed);
    assert_eq!(edge(&map, &nic, &local).kind, EdgeKind::Probed);
    assert_eq!(map.edges.len(), 2, "no tunnel is graded, so none is drawn");
}

#[test]
fn a_tunnel_sits_between_the_nic_and_the_local_interface() {
    let map = build(
        &statuses(&[(Role::Tunnel, Health::Healthy)]),
        &BTreeMap::new(),
        &BTreeMap::new(),
    );
    let tunnel = Place::Role(Role::Tunnel);
    assert_eq!(node(&map, &tunnel).depth, 2);
    edge(&map, &Place::Role(Role::Nic), &tunnel);
    edge(&map, &tunnel, &Place::Role(Role::LocalInterface));
}

#[test]
fn hops_with_the_same_address_and_ttl_are_one_router_node_shared_by_both_paths() {
    let all = statuses(&[
        (hop(1, 1), Health::Healthy),
        (hop(2, 1), Health::Healthy),
        (target(1), Health::Healthy),
        (target(2), Health::Healthy),
    ]);
    let hops = BTreeMap::from([
        (hop(1, 1), ip("192.168.1.1")),
        (hop(2, 1), ip("192.168.1.1")),
    ]);
    let map = build(&all, &hops, &BTreeMap::new());
    let gateway = router("192.168.1.1", 1);
    let shared = node(&map, &gateway);
    assert_eq!(shared.depth, 4);
    assert_eq!(
        shared.roles.keys().cloned().collect::<Vec<_>>(),
        vec![hop(1, 1), hop(2, 1)]
    );
    let into = edge(&map, &Place::Role(Role::LocalInterface), &gateway);
    assert_eq!(into.kind, EdgeKind::Probed);
    assert_eq!(into.targets, targets(&[1, 2]));
    assert_eq!(
        map.nodes
            .iter()
            .filter(|n| matches!(n.place, Place::Router { .. }))
            .count(),
        1
    );
}

#[test]
fn a_router_nodes_status_is_the_worst_of_its_roles_and_it_lists_each() {
    let all = statuses(&[
        (hop(1, 2), Health::RateLimited),
        (hop(2, 2), Health::Degraded),
    ]);
    let hops = BTreeMap::from([(hop(1, 2), ip("10.0.0.1")), (hop(2, 2), ip("10.0.0.1"))]);
    let map = build(&all, &hops, &BTreeMap::new());
    let shared = node(&map, &router("10.0.0.1", 2));
    assert_eq!(shared.health, Some(Health::Degraded));
    assert_eq!(shared.roles[&hop(1, 2)], Health::RateLimited);
    assert_eq!(shared.roles[&hop(2, 2)], Health::Degraded);
}

#[test]
fn a_hop_with_no_responding_address_is_a_node_of_its_own() {
    let all = statuses(&[(hop(1, 1), Health::Unknown), (hop(2, 1), Health::Unknown)]);
    let map = build(&all, &BTreeMap::new(), &BTreeMap::new());
    assert_eq!(node(&map, &Place::Role(hop(1, 1))).depth, 4);
    assert_eq!(node(&map, &Place::Role(hop(2, 1))).depth, 4);
}

#[test]
fn a_target_without_a_trace_is_one_step_past_its_last_hop_by_an_unmeasured_edge() {
    let all = statuses(&[
        (hop(1, 1), Health::Healthy),
        (hop(1, 2), Health::Healthy),
        (hop(1, 3), Health::Healthy),
        (target(1), Health::Failing),
    ]);
    let hops = BTreeMap::from([
        (hop(1, 1), ip("192.168.1.1")),
        (hop(1, 2), ip("10.0.0.1")),
        (hop(1, 3), ip("10.0.0.2")),
    ]);
    let map = build(&all, &hops, &BTreeMap::new());
    let t = node(&map, &Place::Role(target(1)));
    assert_eq!(t.depth, 7);
    assert_eq!(t.health, Some(Health::Failing));
    assert_eq!(
        edge(&map, &router("10.0.0.1", 2), &router("10.0.0.2", 3)).kind,
        EdgeKind::Probed
    );
    assert_eq!(
        edge(&map, &router("10.0.0.2", 3), &Place::Role(target(1))).kind,
        EdgeKind::Unmeasured
    );
}

#[test]
fn a_target_with_no_hops_or_trace_hangs_off_the_local_interface_by_an_unmeasured_edge() {
    let map = build(
        &statuses(&[(target(21), Health::Healthy)]),
        &BTreeMap::new(),
        &BTreeMap::new(),
    );
    let into = edge(
        &map,
        &Place::Role(Role::LocalInterface),
        &Place::Role(target(21)),
    );
    assert_eq!(into.kind, EdgeKind::Unmeasured);
    assert_eq!(into.targets, targets(&[21]));
    assert_eq!(node(&map, &Place::Role(target(21))).depth, 4);
    assert_eq!(
        edge(&map, &Place::Role(Role::Kernel), &Place::Role(Role::Nic)).targets,
        targets(&[21]),
        "every path runs through the host's layers"
    );
}

#[test]
fn the_latest_trace_extends_the_path_and_the_target_sits_where_the_trace_reached_it() {
    let all = statuses(&[
        (hop(1, 1), Health::Healthy),
        (hop(1, 2), Health::Healthy),
        (hop(1, 3), Health::Healthy),
        (target(1), Health::Healthy),
    ]);
    let hops = BTreeMap::from([
        (hop(1, 1), ip("192.168.1.1")),
        (hop(1, 2), ip("10.0.0.1")),
        (hop(1, 3), ip("10.0.0.2")),
    ]);
    let traces = BTreeMap::from([(
        TargetId::new(1),
        latest(
            "1.1.1.1",
            vec![trace_hop(4, &["10.9.0.1"]), trace_hop(5, &["1.1.1.1"])],
            None,
        ),
    )]);
    let map = build(&all, &hops, &traces);
    let beyond = node(&map, &router("10.9.0.1", 4));
    assert_eq!(beyond.depth, 7);
    assert_eq!(beyond.health, None, "only a trace has seen it");
    assert_eq!(beyond.sightings.len(), 1);
    assert_eq!(beyond.sightings[0].target, TargetId::new(1));
    assert_eq!(beyond.sightings[0].answered, 3);
    assert_eq!(
        edge(&map, &router("10.0.0.2", 3), &router("10.9.0.1", 4)).kind,
        EdgeKind::Traced
    );
    let t = node(&map, &Place::Role(target(1)));
    assert_eq!(t.depth, 8);
    assert_eq!(t.health, Some(Health::Healthy));
    assert_eq!(
        edge(&map, &router("10.9.0.1", 4), &Place::Role(target(1))).kind,
        EdgeKind::Traced
    );
    assert!(
        !map.nodes.iter().any(|n| n.place == router("1.1.1.1", 5)),
        "the destination answering is the target itself"
    );
}

#[test]
fn a_load_balanced_ttl_has_a_node_per_responder_each_joined_to_the_next_ttl() {
    let all = statuses(&[(target(11), Health::Healthy)]);
    let traces = BTreeMap::from([(
        TargetId::new(11),
        latest(
            "1.1.1.1",
            vec![
                trace_hop(4, &["10.9.0.1"]),
                trace_hop(5, &["10.9.1.1", "10.9.1.2"]),
                trace_hop(6, &["1.1.1.1"]),
            ],
            None,
        ),
    )]);
    let map = build(&all, &BTreeMap::new(), &traces);
    for responder in ["10.9.1.1", "10.9.1.2"] {
        assert_eq!(
            edge(&map, &router("10.9.0.1", 4), &router(responder, 5)).kind,
            EdgeKind::Traced
        );
        edge(&map, &router(responder, 5), &Place::Role(target(11)));
    }
    assert_eq!(
        edge(
            &map,
            &Place::Role(Role::LocalInterface),
            &router("10.9.0.1", 4)
        )
        .kind,
        EdgeKind::Unmeasured,
        "a DNS target's hops 1–3 aren't measured"
    );
}

#[test]
fn a_ttl_nobody_answered_is_a_silent_node_of_that_target_alone() {
    let all = statuses(&[(target(1), Health::Healthy), (target(2), Health::Healthy)]);
    let trace = |dest: &str| {
        latest(
            dest,
            vec![
                trace_hop(4, &["10.9.0.1"]),
                trace_hop(5, &[]),
                trace_hop(6, &[dest]),
            ],
            None,
        )
    };
    let traces = BTreeMap::from([
        (TargetId::new(1), trace("1.1.1.1")),
        (TargetId::new(2), trace("8.8.8.8")),
    ]);
    let map = build(&all, &BTreeMap::new(), &traces);
    let silent = |id| Place::Silent {
        target: TargetId::new(id),
        ttl: ttl(5),
    };
    let one = node(&map, &silent(1));
    assert_eq!(one.depth, 8);
    assert_eq!(one.health, None);
    node(&map, &silent(2));
    assert_eq!(
        edge(&map, &router("10.9.0.1", 4), &silent(1)).targets,
        targets(&[1])
    );
    assert_eq!(
        edge(
            &map,
            &Place::Role(Role::LocalInterface),
            &router("10.9.0.1", 4)
        )
        .targets,
        targets(&[1, 2])
    );
}

#[test]
fn a_trace_that_went_silent_ends_in_a_silent_node_then_an_unmeasured_edge_to_the_target() {
    let all = statuses(&[(target(1), Health::Failing)]);
    let traces = BTreeMap::from([(
        TargetId::new(1),
        latest("1.1.1.1", vec![trace_hop(4, &["10.9.0.1"])], Some(5)),
    )]);
    let map = build(&all, &BTreeMap::new(), &traces);
    let silent = Place::Silent {
        target: TargetId::new(1),
        ttl: ttl(5),
    };
    assert_eq!(
        edge(&map, &router("10.9.0.1", 4), &silent).kind,
        EdgeKind::Traced
    );
    assert_eq!(
        edge(&map, &silent, &Place::Role(target(1))).kind,
        EdgeKind::Unmeasured
    );
    assert_eq!(node(&map, &Place::Role(target(1))).depth, 9);
}

#[test]
fn radios_hang_off_the_kernel_by_off_path_edges_that_no_target_takes() {
    let map = build(
        &statuses(&[
            (Role::Wifi, Health::Healthy),
            (Role::Cellular, Health::Unknown),
        ]),
        &BTreeMap::new(),
        &BTreeMap::new(),
    );
    for radio in [Role::Wifi, Role::Cellular] {
        let place = Place::Role(radio);
        assert_eq!(node(&map, &place).depth, 1);
        let off = edge(&map, &Place::Role(Role::Kernel), &place);
        assert_eq!(off.kind, EdgeKind::OffPath);
        assert!(off.targets.is_empty());
    }
}

#[test]
fn a_router_that_is_both_a_hop_and_a_trace_responder_is_one_node() {
    // Target 1's hop 3 is the router that target 11's trace found at TTL 3.
    let all = statuses(&[
        (hop(1, 3), Health::Healthy),
        (target(1), Health::Healthy),
        (target(11), Health::Healthy),
    ]);
    let hops = BTreeMap::from([(hop(1, 3), ip("10.0.0.2"))]);
    let traces = BTreeMap::from([(
        TargetId::new(11),
        latest(
            "1.1.1.1",
            vec![trace_hop(3, &["10.0.0.2"]), trace_hop(4, &["1.1.1.1"])],
            None,
        ),
    )]);
    let map = build(&all, &hops, &traces);
    let into = edge(
        &map,
        &Place::Role(Role::LocalInterface),
        &router("10.0.0.2", 3),
    );
    assert_eq!(into.kind, EdgeKind::Unmeasured);
    assert_eq!(into.targets, targets(&[1, 11]));
    let shared = node(&map, &router("10.0.0.2", 3));
    assert_eq!(shared.health, Some(Health::Healthy));
    assert_eq!(shared.sightings.len(), 1);
}

// Properties, over arbitrary statuses, hop addresses, and traces drawn from a small
// pool of addresses, so that paths share routers often.

const POOL: [&str; 6] = [
    "10.0.0.1", "10.0.0.2", "10.0.0.3", "10.0.0.4", "1.1.1.1", "8.8.8.8",
];

fn health() -> impl Strategy<Value = Health> {
    prop_oneof![
        Just(Health::Unknown),
        Just(Health::Healthy),
        Just(Health::RateLimited),
        Just(Health::Degraded),
        Just(Health::Failing),
    ]
}

/// One target's inputs: whether its target role is graded, its hops 1–3 (graded or
/// not, with an address or not), and its trace.
type TargetInputs = (
    Health,
    Vec<Option<(Health, Option<usize>)>>,
    Option<(usize, Vec<(u8, Vec<usize>)>, Option<u8>)>,
);

fn target_inputs() -> impl Strategy<Value = TargetInputs> {
    let hops = prop::collection::vec(
        prop::option::of((health(), prop::option::of(0..POOL.len()))),
        3,
    );
    let trace = prop::option::of((
        0..POOL.len(),
        prop::collection::vec((1u8..12, prop::collection::vec(0..POOL.len(), 0..3)), 0..6),
        prop::option::of(1u8..14),
    ));
    (health(), hops, trace)
}

fn inputs() -> impl Strategy<
    Value = (
        BTreeMap<Role, Health>,
        BTreeMap<Role, IpAddr>,
        BTreeMap<TargetId, LatestTrace>,
    ),
> {
    (
        prop::collection::vec(target_inputs(), 0..5),
        prop::option::of(health()),
        prop::option::of(health()),
    )
        .prop_map(|(per_target, tunnel, wifi)| {
            let mut all = statuses(&[]);
            all.extend(tunnel.map(|h| (Role::Tunnel, h)));
            all.extend(wifi.map(|h| (Role::Wifi, h)));
            let mut hops = BTreeMap::new();
            let mut traces = BTreeMap::new();
            for (i, (status, hop_inputs, trace)) in per_target.into_iter().enumerate() {
                let id = u32::try_from(i).unwrap() + 1;
                all.insert(target(id), status);
                for (n, input) in hop_inputs.into_iter().enumerate() {
                    if let Some((h, address)) = input {
                        let role = hop(id, u8::try_from(n).unwrap() + 1);
                        all.insert(role.clone(), h);
                        if let Some(a) = address {
                            hops.insert(role, ip(POOL[a]));
                        }
                    }
                }
                if let Some((destination, rows, silent_from)) = trace {
                    // A trace's TTLs are distinct and ascending.
                    let rows: BTreeMap<u8, Vec<usize>> = rows.into_iter().collect();
                    let hops = rows
                        .into_iter()
                        .map(|(n, r)| {
                            let names: Vec<&str> = r.iter().map(|&a| POOL[a]).collect();
                            trace_hop(n, &names)
                        })
                        .collect();
                    traces.insert(
                        TargetId::new(id),
                        latest(POOL[destination], hops, silent_from),
                    );
                }
            }
            (all, hops, traces)
        })
}

proptest! {
    #[test]
    fn every_edge_goes_deeper_so_the_map_is_acyclic((all, hops, traces) in inputs()) {
        let map = build(&all, &hops, &traces);
        let depth: BTreeMap<&Place, u16> = map.nodes.iter().map(|n| (&n.place, n.depth)).collect();
        prop_assert_eq!(depth.len(), map.nodes.len(), "a place is one node");
        for e in &map.edges {
            let (from, to) = (depth.get(&e.from), depth.get(&e.to));
            prop_assert!(from.is_some() && to.is_some(), "edge {:?} has both ends", e);
            prop_assert!(from < to, "edge {:?} goes deeper", e);
        }
    }

    #[test]
    fn every_graded_role_is_in_exactly_one_node((all, hops, traces) in inputs()) {
        let map = build(&all, &hops, &traces);
        for (role, &health) in &all {
            let holders: Vec<&MapNode> = map.nodes.iter().filter(|n| n.roles.contains_key(role)).collect();
            prop_assert_eq!(holders.len(), 1, "{:?} is in one node", role);
            prop_assert_eq!(holders[0].roles[role], health);
        }
        for n in &map.nodes {
            let worst = n.roles.values().copied().reduce(Health::worse);
            prop_assert_eq!(n.health, worst);
        }
    }

    #[test]
    fn every_target_is_reachable_from_the_kernel_along_its_own_edges((all, hops, traces) in inputs()) {
        let map = build(&all, &hops, &traces);
        for role in all.keys() {
            let Role::Target(id) = role else { continue };
            let goal = Place::Role(role.clone());
            let mut seen = BTreeSet::from([Place::Role(Role::Kernel)]);
            let mut queue = VecDeque::from([Place::Role(Role::Kernel)]);
            while let Some(at) = queue.pop_front() {
                for e in map.edges.iter().filter(|e| e.from == at) {
                    if e.targets.contains(id) && seen.insert(e.to.clone()) {
                        queue.push_back(e.to.clone());
                    }
                }
            }
            prop_assert!(seen.contains(&goal), "{:?} is reachable in {:#?}", role, map);
        }
    }
}
