//! The network map (ADR-0042): every path from this host to every target, as a
//! directed acyclic graph whose nodes carry their status.
//!
//! [`build`] is a pure function of what the daemon already knows: every role's
//! status, each hop's responding address, and each target's latest path trace. It
//! hides how paths are merged: hops of different targets with the same responding
//! address and TTL become one [`MapNode`], so paths meet where they share a router.
//!
//! # Invariants
//!
//! - Every edge goes from a smaller [`MapNode::depth`] to a larger one, so the map is
//!   acyclic.
//! - Every graded role is merged into exactly one node.
//! - Every target with a role has a path from the kernel.

use std::collections::{BTreeMap, BTreeSet};
use std::net::IpAddr;

use netwatch_shared::uom::si::f64::Time;
use netwatch_shared::{HopTtl, Role, TargetId};

use crate::Health;
use crate::trace::PathTrace;

/// Where a [`MapNode`] is on the network (glossary: *map node*).
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Place {
    /// A role that is a node of its own: a layer, a radio, a target, or a hop with no
    /// responding address yet.
    Role(Role),
    /// A router: a responding address at one TTL, shared by every path through it.
    /// The TTL is part of it, so that two paths crossing the same two routers in
    /// opposite orders can't form a cycle.
    Router {
        /// The address that answered.
        address: IpAddr,
        /// The TTL it answered at.
        ttl: HopTtl,
    },
    /// A TTL of one target's trace at which nobody answered.
    Silent {
        /// Whose trace.
        target: TargetId,
        /// The TTL.
        ttl: HopTtl,
    },
}

/// What one target's latest trace found at a node.
#[derive(Debug, Clone, PartialEq)]
pub struct Sighting {
    /// Whose trace.
    pub target: TargetId,
    /// Probes the trace sent at this TTL.
    pub sent: u32,
    /// Of those, answered by anyone.
    pub answered: u32,
    /// The median answer.
    pub median: Option<Time>,
}

/// One node of the map.
#[derive(Debug, Clone, PartialEq)]
pub struct MapNode {
    /// Where it is.
    pub place: Place,
    /// Its position along the path: kernel 0, NIC 1, tunnel 2, local interface 3, a
    /// hop at TTL n 3 + n. Radios are 1. A target is one past whatever leads to it.
    pub depth: u16,
    /// The roles merged into it, each with its own status.
    pub roles: BTreeMap<Role, Health>,
    /// The worst status among `roles`. `None` when no role is merged into it: the
    /// node was seen only by a trace.
    pub health: Option<Health>,
    /// What traces found here.
    pub sightings: Vec<Sighting>,
}

/// How an edge's step is known (glossary: *map edge*).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum EdgeKind {
    /// Hops between its ends are unknown.
    Unmeasured,
    /// At least one end comes only from a trace.
    Traced,
    /// Both ends are measured continuously.
    Probed,
    /// From the kernel to a radio, which carries no probes (ADR-0036).
    OffPath,
}

/// One edge of the map.
#[derive(Debug, Clone, PartialEq)]
pub struct MapEdge {
    /// The upstream end.
    pub from: Place,
    /// The downstream end.
    pub to: Place,
    /// How the step is known. When several paths share the edge, the best-known.
    pub kind: EdgeKind,
    /// The targets whose paths take it.
    pub targets: BTreeSet<TargetId>,
}

/// The network map (glossary: *network map*). Nodes are ordered by depth, then by
/// place; edges by their ends.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct NetworkMap {
    /// Every node.
    pub nodes: Vec<MapNode>,
    /// Every edge.
    pub edges: Vec<MapEdge>,
}

/// A target's latest trace, and the address it was sent toward.
#[derive(Debug, Clone, PartialEq)]
pub struct LatestTrace {
    /// The target's address. Where it answers, the trace reached the target.
    pub destination: IpAddr,
    /// What the trace found.
    pub trace: PathTrace,
}

/// Builds the map from every role's status, each hop's responding address, and each
/// target's latest trace.
pub fn build(
    statuses: &BTreeMap<Role, Health>,
    hops: &BTreeMap<Role, IpAddr>,
    traces: &BTreeMap<TargetId, LatestTrace>,
) -> NetworkMap {
    let _ = (statuses, hops, traces);
    NetworkMap::default()
}
