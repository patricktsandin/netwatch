//! Upstream and downstream along a target's path (glossary: *upstream /
//! downstream*, ADR-0031).

#![allow(clippy::unwrap_used)] // Tests may unwrap (ADR-0015), including in helpers.

use std::num::NonZeroU8;

use netwatch_shared::{HopTtl, Role, TargetId};

fn hop(target: u32, ttl: u8) -> Role {
    Role::Hop {
        target: TargetId::new(target),
        ttl: HopTtl::new(NonZeroU8::new(ttl).unwrap()),
    }
}

fn target(id: u32) -> Role {
    Role::Target(TargetId::new(id))
}

#[test]
fn the_host_layers_are_upstream_of_every_path_in_stack_order() {
    let stack = [Role::Kernel, Role::Nic, Role::Tunnel, Role::LocalInterface];
    for (i, upper) in stack.iter().enumerate() {
        for lower in &stack[i + 1..] {
            assert!(upper.is_upstream_of(lower), "{upper:?} before {lower:?}");
            assert!(!lower.is_upstream_of(upper));
        }
        assert!(upper.is_upstream_of(&hop(1, 1)));
        assert!(upper.is_upstream_of(&target(7)));
    }
}

#[test]
fn hops_are_upstream_of_later_hops_and_the_target_on_their_own_path_only() {
    assert!(hop(1, 1).is_upstream_of(&hop(1, 2)));
    assert!(hop(1, 2).is_upstream_of(&target(1)));
    assert!(!hop(1, 2).is_upstream_of(&hop(1, 1)));
    assert!(!hop(1, 1).is_upstream_of(&hop(2, 2)));
    assert!(!hop(1, 1).is_upstream_of(&target(2)));
    assert!(!hop(1, 1).is_upstream_of(&Role::Nic));
}

#[test]
fn no_role_is_upstream_of_itself_and_targets_of_nothing() {
    for role in [Role::Kernel, Role::Nic, hop(1, 2), target(1)] {
        assert!(!role.is_upstream_of(&role));
    }
    assert!(!target(1).is_upstream_of(&target(2)));
    assert!(!target(1).is_upstream_of(&hop(1, 3)));
}

#[test]
fn radio_roles_are_off_the_path_neither_upstream_nor_downstream() {
    for radio in [Role::Wifi, Role::Cellular] {
        for other in [
            Role::Kernel,
            Role::Nic,
            Role::LocalInterface,
            hop(1, 1),
            target(1),
            Role::Wifi,
            Role::Cellular,
        ] {
            assert!(!radio.is_upstream_of(&other), "{radio:?} before {other:?}");
            assert!(!other.is_upstream_of(&radio), "{other:?} before {radio:?}");
        }
    }
}
