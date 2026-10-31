//! How an egress path determines the global roles (ADR-0022).

#![allow(clippy::unwrap_used)] // Tests may unwrap (ADR-0015), including in helpers.

use std::net::{IpAddr, Ipv4Addr};
use std::num::NonZeroU32;

use netwatch_shared::{Identity, InterfaceIndex, InterfaceName, Role};
use netwatch_topology::egress::observations;
use netwatch_topology::{EgressPath, InterfaceKind, Observation};

fn path(name: &str, kind: InterfaceKind, source: Option<IpAddr>) -> EgressPath {
    EgressPath {
        index: InterfaceIndex::new(NonZeroU32::new(2).unwrap()),
        name: InterfaceName::parse(name.as_bytes()).unwrap(),
        kind,
        source,
    }
}

fn interface(path: &EgressPath) -> Identity {
    Identity::Interface {
        index: path.index,
        name: path.name,
    }
}

const SOURCE: IpAddr = IpAddr::V4(Ipv4Addr::new(192, 168, 1, 207));

#[test]
fn physical_egress_binds_the_nic_and_the_local_address() {
    let eth0 = path("eth0", InterfaceKind::Physical, Some(SOURCE));

    assert_eq!(
        observations(Some(&eth0)),
        vec![
            Observation::IdentityObserved {
                role: Role::Nic,
                identity: interface(&eth0)
            },
            Observation::RoleVanished { role: Role::Tunnel },
            Observation::IdentityObserved {
                role: Role::LocalInterface,
                identity: Identity::Address(SOURCE)
            },
        ]
    );
}

#[test]
fn tunnel_egress_binds_the_tunnel_and_leaves_the_nic_unbound() {
    let wg0 = path("wg0", InterfaceKind::Tunnel, None);

    assert_eq!(
        observations(Some(&wg0)),
        vec![
            Observation::RoleVanished { role: Role::Nic },
            Observation::IdentityObserved {
                role: Role::Tunnel,
                identity: interface(&wg0)
            },
            Observation::RoleVanished {
                role: Role::LocalInterface
            },
        ]
    );
}

#[test]
fn no_route_unbinds_every_global_role() {
    assert!(
        observations(None)
            .iter()
            .all(|o| matches!(o, Observation::RoleVanished { .. }))
    );
}
