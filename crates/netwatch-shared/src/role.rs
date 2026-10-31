//! Roles: the logical positions netwatch measures (ADR-0002).

use std::num::NonZeroU8;

/// A logical position on the monitored path. Series are keyed by role, never by
/// the identity behind it, so history stays continuous across failover and
/// reroutes.
///
/// The kernel layer has no role: it is the host itself and has no identity that
/// could change.
///
/// The derived `Ord` (declaration order, then fields) is the order in which the
/// topology aggregate visits roles during a resync. Epoch allocation depends on
/// it, and `specs/topology.qnt` assumes it, so don't reorder the variants.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Role {
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

    /// A stable text key, used wherever a role crosses a boundary as text: database
    /// rows, API paths, the UI. `nic`, `tunnel`, `local-interface`, `hop:<target>:<ttl>`,
    /// `target:<id>`.
    pub fn key(&self) -> String {
        match self {
            Role::Nic => "nic".to_owned(),
            Role::Tunnel => "tunnel".to_owned(),
            Role::LocalInterface => "local-interface".to_owned(),
            Role::Hop { target, ttl } => format!("hop:{}:{}", target.get(), ttl.get()),
            Role::Target(target) => format!("target:{}", target.get()),
        }
    }

    /// Parses a [`key`](Self::key). `None` for anything else.
    pub fn from_key(key: &str) -> Option<Self> {
        match key {
            "nic" => Some(Role::Nic),
            "tunnel" => Some(Role::Tunnel),
            "local-interface" => Some(Role::LocalInterface),
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
