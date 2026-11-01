//! Roles: the logical positions netwatch measures (ADR-0002).

use std::num::NonZeroU8;

/// A logical position on the monitored path. Series are keyed by role, never by
/// the identity behind it, so history stays continuous across failover and
/// reroutes.
///
/// The kernel layer's role, [`Role::Kernel`], is bound to the loopback interface,
/// whose identity never changes. It still needs a role, because its stack-baseline
/// series needs a key.
///
/// The derived `Ord` (declaration order, then fields) is the order in which the
/// topology aggregate visits roles during a resync. Epoch allocation depends on
/// it, and `specs/topology.qnt` assumes it, so don't reorder the variants.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Role {
    /// The host's own network stack, measured over loopback (ADR-0008).
    Kernel,
    /// The physical device under the current egress path.
    Nic,
    /// The VPN interface the egress path goes through, if any.
    Tunnel,
    /// The interface's addresses and its link to the gateway.
    LocalInterface,
    /// The router at `ttl` on the path toward `target`. Hops are per target, so
    /// that they are correct behind VPNs and policy routing.
    Hop {
        /// The target whose path this hop is on.
        target: TargetId,
        /// The hop's distance from the host.
        ttl: HopTtl,
    },
    /// A configured endpoint.
    Target(TargetId),
    /// The Wi-Fi device, whether it carries the egress path or stands by (ADR-0036).
    Wifi,
    /// The cellular modem, whether it carries the egress path or stands by.
    Cellular,
}

/// Identifies a configured target. Assigned by configuration, and stable for as
/// long as the target exists.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct TargetId(u32);

impl TargetId {
    /// Wraps a configuration-assigned target number.
    pub const fn new(raw: u32) -> Self {
        Self(raw)
    }

    /// The number as configured.
    pub const fn get(self) -> u32 {
        self.0
    }
}

/// The IP TTL (or IPv6 hop limit) at which a hop answers. It is never zero,
/// because a packet sent with TTL 0 never leaves the host.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct HopTtl(NonZeroU8);

impl HopTtl {
    /// Wraps a TTL. The type rules out zero.
    pub const fn new(ttl: NonZeroU8) -> Self {
        Self(ttl)
    }

    /// The TTL to put on the wire.
    pub const fn get(self) -> NonZeroU8 {
        self.0
    }
}

impl Role {
    /// Whether the role's identity comes from the kernel's topology (netlink), as
    /// opposed to being learned by Probing, as hops are.
    pub fn is_global(&self) -> bool {
        matches!(self, Role::Nic | Role::Tunnel | Role::LocalInterface)
    }

    /// Whether `self` comes before `other` on a path out of this host (glossary:
    /// *upstream / downstream*): kernel, NIC, tunnel, local interface, then each
    /// target's hops in order, then the target. Hops and targets are upstream only
    /// along their own target's path. A problem upstream explains one downstream
    /// (ADR-0031), never the other way around.
    pub fn is_upstream_of(&self, other: &Role) -> bool {
        // Radio roles are off the path (ADR-0036).
        let radio = |role: &Role| matches!(role, Role::Wifi | Role::Cellular);
        if radio(self) || radio(other) {
            return false;
        }
        let path = |role: &Role| match role {
            Role::Hop { target, .. } | Role::Target(target) => Some(*target),
            Role::Kernel
            | Role::Nic
            | Role::Tunnel
            | Role::LocalInterface
            | Role::Wifi
            | Role::Cellular => None,
        };
        let on_its_path = match (path(self), path(other)) {
            (None, _) => true,
            (Some(mine), Some(theirs)) => mine == theirs,
            (Some(_), None) => false,
        };
        on_its_path && self.depth() < other.depth()
    }

    /// The position along a path that [`is_upstream_of`](Self::is_upstream_of)
    /// compares.
    fn depth(&self) -> u16 {
        match self {
            Role::Kernel => 0,
            Role::Nic => 1,
            Role::Tunnel => 2,
            Role::LocalInterface => 3,
            Role::Hop { ttl, .. } => 3 + u16::from(ttl.get().get()),
            Role::Target(_) | Role::Wifi | Role::Cellular => u16::MAX,
        }
    }

    /// A stable text key, used wherever a role crosses a boundary as text: database
    /// rows, API paths, the UI. `nic`, `tunnel`, `local-interface`, `hop:<target>:<ttl>`,
    /// `target:<id>`, `wifi`, `cellular`.
    pub fn key(&self) -> String {
        match self {
            Role::Kernel => "kernel".to_owned(),
            Role::Nic => "nic".to_owned(),
            Role::Tunnel => "tunnel".to_owned(),
            Role::LocalInterface => "local-interface".to_owned(),
            Role::Hop { target, ttl } => format!("hop:{}:{}", target.get(), ttl.get()),
            Role::Target(target) => format!("target:{}", target.get()),
            Role::Wifi => "wifi".to_owned(),
            Role::Cellular => "cellular".to_owned(),
        }
    }

    /// Parses a [`key`](Self::key). `None` for anything else.
    pub fn from_key(key: &str) -> Option<Self> {
        match key {
            "kernel" => Some(Role::Kernel),
            "nic" => Some(Role::Nic),
            "tunnel" => Some(Role::Tunnel),
            "local-interface" => Some(Role::LocalInterface),
            "wifi" => Some(Role::Wifi),
            "cellular" => Some(Role::Cellular),
            _ => {
                let mut parts = key.split(':');
                let role = match (parts.next()?, parts.next(), parts.next()) {
                    ("target", Some(id), None) => Role::Target(TargetId(canonical(id)?)),
                    ("hop", Some(id), Some(ttl)) => Role::Hop {
                        target: TargetId(canonical(id)?),
                        ttl: HopTtl(NonZeroU8::new(u8::try_from(canonical(ttl)?).ok()?)?),
                    },
                    _ => return None,
                };
                parts.next().is_none().then_some(role)
            }
        }
    }
}

/// A decimal number written the way `key` writes it: digits only, no sign, and no
/// leading zeros. Accepting only that form makes `from_key` the exact inverse of
/// `key`.
fn canonical(digits: &str) -> Option<u32> {
    let valid = !digits.is_empty()
        && digits.bytes().all(|b| b.is_ascii_digit())
        && (digits == "0" || !digits.starts_with('0'));
    valid.then(|| digits.parse().ok()).flatten()
}
