//! Role keys are a published representation, so they must round-trip exactly.

#![allow(clippy::unwrap_used)] // Tests may unwrap (ADR-0015), including in helpers.

use std::num::NonZeroU8;

use netwatch_shared::{HopTtl, Role, TargetId};

fn every_kind_of_role() -> Vec<Role> {
    vec![
        Role::Nic,
        Role::Tunnel,
        Role::LocalInterface,
        Role::Target(TargetId::new(0)),
        Role::Target(TargetId::new(u32::MAX)),
        Role::Hop {
            target: TargetId::new(7),
            ttl: HopTtl::new(NonZeroU8::new(3).unwrap()),
        },
    ]
}

#[test]
fn every_role_round_trips_through_its_key() {
    for role in every_kind_of_role() {
        assert_eq!(Role::from_key(&role.key()), Some(role));
    }
}

#[test]
fn keys_look_as_documented() {
    let hop = Role::Hop {
        target: TargetId::new(7),
        ttl: HopTtl::new(NonZeroU8::new(3).unwrap()),
    };
    assert_eq!(hop.key(), "hop:7:3");
    assert_eq!(Role::Target(TargetId::new(1)).key(), "target:1");
}

#[test]
fn non_canonical_and_malformed_keys_are_rejected() {
    for key in [
        "",
        "NIC",
        "target:",
        "target:01",
        "target:+1",
        "target:-1",
        "target:1:2",
        "hop:1",
        "hop:1:0",
        "hop:1:256",
        "hop:1:2:3",
        "target:4294967296",
    ] {
        assert_eq!(Role::from_key(key), None, "{key:?}");
    }
}
