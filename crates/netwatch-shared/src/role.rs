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
