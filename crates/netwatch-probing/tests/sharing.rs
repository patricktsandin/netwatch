//! Behaviour of hop stream sharing (ADR-0026, ADR-0043).

#![allow(clippy::unwrap_used)] // Tests may unwrap (ADR-0015).

use std::collections::BTreeSet;
use std::net::IpAddr;

use netwatch_probing::sharing::{HOP_TTLS, HopStream, plan};
use netwatch_shared::{HopTtl, TargetId};
use proptest::prelude::*;

fn ip(text: &str) -> IpAddr {
    text.parse().unwrap()
}

fn t(id: u32) -> TargetId {
    TargetId::new(id)
}

fn at(streams: &[HopStream], ttl: HopTtl) -> Vec<&HopStream> {
    streams.iter().filter(|s| s.ttl == ttl).collect()
}

#[test]
fn a_dns_target_shares_every_hop_stream_of_an_icmp_target_with_its_address() {
    let streams = plan(&[(t(1), ip("1.1.1.1")), (t(11), ip("1.1.1.1"))]);
    assert_eq!(streams.len(), 3);
    for s in &streams {
        assert_eq!(s.owner, t(1));
        assert_eq!(s.serves, vec![t(1), t(11)]);
        assert_eq!(s.destination, ip("1.1.1.1"));
    }
}

#[test]
fn hop_1_is_one_stream_per_address_family_and_hops_2_and_3_one_per_address() {
    let streams = plan(&[
        (t(1), ip("1.1.1.1")),
        (t(2), ip("8.8.8.8")),
        (t(4), ip("2606:4700:4700::1111")),
        (t(12), ip("8.8.8.8")),
    ]);
    let first = at(&streams, HOP_TTLS[0]);
    assert_eq!(first.len(), 2, "IPv4 and IPv6 have different gateways");
    assert_eq!(first[0].serves, vec![t(1), t(2), t(12)]);
    assert_eq!(first[1].serves, vec![t(4)]);
    for ttl in &HOP_TTLS[1..] {
        let beyond = at(&streams, *ttl);
        let serves: Vec<Vec<TargetId>> = beyond.iter().map(|s| s.serves.clone()).collect();
        assert_eq!(serves, vec![vec![t(1)], vec![t(2), t(12)], vec![t(4)]]);
    }
}

#[test]
fn a_shared_stream_runs_as_its_lowest_targets_hop() {
    let streams = plan(&[(t(12), ip("8.8.8.8")), (t(2), ip("8.8.8.8"))]);
    assert_eq!(streams.len(), 3);
    assert!(
        streams
            .iter()
            .all(|s| s.owner == t(2) && s.serves[0] == t(2))
    );
}

#[test]
fn no_targets_need_no_streams() {
    assert!(plan(&[]).is_empty());
}

const POOL: [&str; 5] = [
    "1.1.1.1",
    "8.8.8.8",
    "9.9.9.9",
    "2606:4700:4700::1111",
    "2620:fe::fe",
];

proptest! {
    #[test]
    fn every_targets_hop_is_served_by_exactly_one_stream(
        chosen in prop::collection::btree_map(0u32..30, 0..POOL.len(), 0..12)
    ) {
        let targets: Vec<(TargetId, IpAddr)> = chosen.iter().map(|(&id, &a)| (t(id), ip(POOL[a]))).collect();
        let streams = plan(&targets);
        for &(target, address) in &targets {
            for ttl in HOP_TTLS {
                let serving: Vec<&HopStream> = streams.iter().filter(|s| s.ttl == ttl && s.serves.contains(&target)).collect();
                prop_assert_eq!(serving.len(), 1, "{:?} at {:?}", target, ttl);
                let s = serving[0];
                prop_assert_eq!(s.destination.is_ipv4(), address.is_ipv4(), "same family");
                if ttl != HOP_TTLS[0] {
                    prop_assert_eq!(s.destination, address, "beyond hop 1, the same address");
                }
            }
        }
        for s in &streams {
            prop_assert_eq!(Some(&s.owner), s.serves.first());
            let sorted: BTreeSet<TargetId> = s.serves.iter().copied().collect();
            prop_assert_eq!(sorted.into_iter().collect::<Vec<_>>(), s.serves.clone());
            let owner_address = targets.iter().find(|(id, _)| *id == s.owner).map(|(_, a)| *a);
            prop_assert_eq!(owner_address, Some(s.destination));
        }
        let keys: BTreeSet<(HopTtl, TargetId)> = streams.iter().map(|s| (s.ttl, s.owner)).collect();
        prop_assert_eq!(keys.len(), streams.len(), "no two streams for one role");
    }
}

#[test]
fn a_shared_rows_status_is_the_worst_of_its_graded_members() {
    use netwatch_probing::Health;
    use netwatch_probing::sharing::row_health;
    use netwatch_shared::Role;
    use std::collections::BTreeMap;

    let streams = plan(&[(t(1), ip("1.1.1.1")), (t(11), ip("1.1.1.1"))]);
    let second = &at(&streams, HOP_TTLS[1])[0];
    let hop = |id| Role::Hop {
        target: t(id),
        ttl: HOP_TTLS[1],
    };
    let grades = BTreeMap::from([(hop(1), Health::RateLimited), (hop(11), Health::Degraded)]);
    assert_eq!(row_health(second, &grades), Some(Health::Degraded));
    let one = BTreeMap::from([(hop(1), Health::Healthy)]);
    assert_eq!(row_health(second, &one), Some(Health::Healthy));
    assert_eq!(row_health(second, &BTreeMap::new()), None);
}
