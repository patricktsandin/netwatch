//! From the kernel's answer to "how would you route this?" to role bindings.

use std::collections::BTreeMap;
use std::net::IpAddr;

use netwatch_shared::{Identity, InterfaceIndex, InterfaceName, Role};

use crate::{Observation, ResyncGeneration, Snapshot};

/// The kernel's resolution of the path to the reference destination
/// (`RTM_GETROUTE`), as an adapter reports it. This is the source of the global
/// roles: [`Role::Nic`], [`Role::Tunnel`], and [`Role::LocalInterface`] (ADR-0022).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EgressPath {
    /// The interface the kernel would send through.
    pub index: InterfaceIndex,
    /// That interface's current name.
    pub name: InterfaceName,
    /// Whether that interface is a physical device or a tunnel.
    pub kind: InterfaceKind,
    /// The source address the kernel would pick, if it reports one.
    pub source: Option<IpAddr>,
}

/// What kind of interface an egress path leaves through.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InterfaceKind {
    /// Anything that isn't a tunnel: Ethernet, Wi-Fi, a cellular modem, a bridge.
    Physical,
    /// A VPN interface, such as WireGuard or tun/tap.
    Tunnel,
}

/// The bindings implied by an egress path, or by its absence. A role that isn't
/// in the map is unbound.
///
/// When the path goes through a tunnel, the tunnel is bound and the NIC is left
/// unbound. Finding the physical device under a tunnel needs a second lookup,
/// toward the tunnel endpoint, which isn't implemented yet (ADR-0022).
pub fn bindings(path: Option<&EgressPath>) -> BTreeMap<Role, Identity> {
    let mut bindings = BTreeMap::new();
    let Some(path) = path else {
        return bindings;
    };
    let interface = Identity::Interface {
        index: path.index,
        name: path.name,
    };
    let role = match path.kind {
        InterfaceKind::Physical => Role::Nic,
        InterfaceKind::Tunnel => Role::Tunnel,
    };
    bindings.insert(role, interface);
    if let Some(source) = path.source {
        bindings.insert(Role::LocalInterface, Identity::Address(source));
    }
    bindings
}

/// The global roles' complete state as notifications: one per role, either
/// observed or vanished. Every role is always included, so the adapter needs no
/// memory of what it last reported. A repeated notification never splits an epoch.
pub fn observations(path: Option<&EgressPath>) -> Vec<Observation> {
    let mut bound = bindings(path);
    [Role::Nic, Role::Tunnel, Role::LocalInterface]
        .into_iter()
        .map(|role| match bound.remove(&role) {
            Some(identity) => Observation::IdentityObserved { role, identity },
            None => Observation::RoleVanished { role },
        })
        .collect()
}

/// The snapshot that answers a resync of `generation`.
pub fn snapshot(generation: ResyncGeneration, path: Option<&EgressPath>) -> Snapshot {
    Snapshot {
        generation,
        bindings: bindings(path),
    }
}
