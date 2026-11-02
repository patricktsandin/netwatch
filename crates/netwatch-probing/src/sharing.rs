//! Stream sharing for hops (ADR-0026, ADR-0043): which hop streams run, and whose
//! hop roles each one serves.
//!
//! [`plan`] hides the sharing rules. Hop 1 is the gateway, the same router for
//! every target of an address family, so one stream per family serves them all.
//! Hops 2 and 3 are shared by targets toward the same destination address, whose
//! route lookups are the same. Fewer streams keep routers' ICMP rate limits from
//! dropping answers, which would show as loss that isn't there.
//!
//! # Invariants
//!
//! - Every target's hop at every TTL in [`HOP_TTLS`] is served by exactly one stream.
//! - A stream's owner is the lowest target it serves.

use std::net::IpAddr;
use std::num::NonZeroU8;

use netwatch_shared::{HopTtl, Role, TargetId};

/// The TTLs probed continuously toward every target with hops (ADR-0002).
pub const HOP_TTLS: [HopTtl; 3] = [
    HopTtl::new(NonZeroU8::MIN),
    HopTtl::new(NonZeroU8::MIN.saturating_add(1)),
    HopTtl::new(NonZeroU8::MIN.saturating_add(2)),
];

/// One hop stream to run.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HopStream {
    /// The TTL it probes at.
    pub ttl: HopTtl,
    /// Where its probes are addressed: the owner's address.
    pub destination: IpAddr,
    /// The target whose hop role the stream runs as.
    pub owner: TargetId,
    /// Every target whose hop at `ttl` it serves, the owner first, ascending.
    pub serves: Vec<TargetId>,
}

impl HopStream {
    /// The role the stream runs as: the owner's hop.
    pub fn role(&self) -> Role {
        Role::Hop {
            target: self.owner,
            ttl: self.ttl,
        }
    }
}

/// The hop streams for `targets`: every target with hops (ICMP and DNS), with the
/// address its hops are probed toward.
pub fn plan(targets: &[(TargetId, IpAddr)]) -> Vec<HopStream> {
    let _ = targets;
    Vec::new()
}
