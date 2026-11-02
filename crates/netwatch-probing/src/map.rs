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
    /// A TTL at which nobody answered a trace. Paths that reach it from the same
    /// places share it: nothing tells them apart there.
    Silent {
        /// The places just before it, in order.
        after: Vec<Place>,
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

/// A node's position along the paths (ADR-0042): kernel 0, NIC 1, tunnel 2, local
/// interface 3, a hop at TTL n 3 + n. Every edge goes to a greater depth, which is what
/// keeps the map acyclic.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Depth(u16);

impl Depth {
    const KERNEL: Depth = Depth(0);
    const NIC: Depth = Depth(1);
    const TUNNEL: Depth = Depth(2);
    const LOCAL_INTERFACE: Depth = Depth(3);
    /// Radios sit beside the NIC.
    const RADIO: Depth = Depth(1);

    /// A hop's depth: one past the local interface's at TTL 1.
    fn of(ttl: HopTtl) -> Depth {
        Depth(3 + u16::from(ttl.get().get()))
    }

    /// The next depth along a path.
    fn next(self) -> Depth {
        Depth(self.0.saturating_add(1))
    }

    /// The number, for the wire.
    pub fn get(self) -> u16 {
        self.0
    }
}

/// One node of the map.
#[derive(Debug, Clone, PartialEq)]
pub struct MapNode {
    /// Where it is.
    pub place: Place,
    /// Its position along the path: kernel 0, NIC 1, tunnel 2, local interface 3, a
    /// hop at TTL n 3 + n. Radios are 1. A target is where its trace reached it, or
    /// else in the bottom row.
    pub depth: Depth,
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

/// Builds the map from every role's status, each hop's responding address, each
/// target's address, and each target's latest trace.
///
/// Each graded target's path runs from the host's layers through its hops in TTL
/// order, then through its trace beyond the last of them, and ends at the target.
/// Consecutive positions on a path are joined by an edge from every node at one to
/// every node at the next. Where a hop or a trace hop is answered by the target's
/// own address, the path has reached the target there: a nearby target, such as a
/// resolver on the LAN, is that close, and its hop roles beyond merge into it.
pub fn build(
    statuses: &BTreeMap<Role, Health>,
    hops: &BTreeMap<Role, IpAddr>,
    destinations: &BTreeMap<TargetId, IpAddr>,
    traces: &BTreeMap<TargetId, &PathTrace>,
) -> NetworkMap {
    let mut map = Builder {
        statuses,
        nodes: BTreeMap::new(),
        edges: BTreeMap::new(),
    };
    let targets: BTreeSet<TargetId> = statuses
        .keys()
        .filter_map(|role| match role {
            Role::Target(target) | Role::Hop { target, .. } => Some(*target),
            _ => None,
        })
        .collect();

    // The host's layers. The kernel is always there, as the root every path starts
    // from.
    let kernel = map.node(Place::Role(Role::Kernel), Depth::KERNEL);
    let mut trunk = kernel.clone();
    for (role, depth) in [
        (Role::Nic, Depth::NIC),
        (Role::Tunnel, Depth::TUNNEL),
        (Role::LocalInterface, Depth::LOCAL_INTERFACE),
    ] {
        if statuses.contains_key(&role) {
            let place = map.node(Place::Role(role), depth);
            map.step(
                &[trunk],
                std::slice::from_ref(&place),
                Link::Path { gap: false },
                &targets,
            );
            trunk = place;
        }
    }
    for radio in [Role::Wifi, Role::Cellular] {
        if statuses.contains_key(&radio) {
            let place = map.node(Place::Role(radio), Depth::RADIO);
            map.step(
                std::slice::from_ref(&kernel),
                &[place],
                Link::OffPath,
                &BTreeSet::new(),
            );
        }
    }
    let trunk_depth = map.nodes.get(&trunk).map_or(Depth::KERNEL, |n| n.depth);

    // Targets whose distance is unknown wait until every other node is in, so that
    // they can join the bottom row.
    let mut unplaced = Vec::new();
    for &target in &targets {
        let only = BTreeSet::from([target]);
        let mut path = Path {
            at: vec![trunk.clone()],
            ttl: 0,
            depth: trunk_depth,
        };
        let mut own_hops: Vec<(HopTtl, &Role)> = statuses
            .keys()
            .filter_map(|role| match role {
                Role::Hop { target: t, ttl } if *t == target => Some((*ttl, role)),
                _ => None,
            })
            .collect();
        own_hops.sort();
        let destination = destinations.get(&target).copied();
        // Where the path reached the target, and what a trace saw there.
        let mut reached: Option<(HopTtl, Option<Sighting>)> = None;
        let mut at_target: Vec<&Role> = Vec::new();
        for (ttl, role) in own_hops {
            let answered = hops.get(role).copied();
            if reached.is_some() || (answered.is_some() && answered == destination) {
                reached = reached.or(Some((ttl, None)));
                at_target.push(role);
                continue;
            }
            let place = match answered {
                Some(address) => Place::Router { address, ttl },
                None => Place::Role(role.clone()),
            };
            map.advance(&mut path, vec![place], ttl, &only, Some(role));
        }

        if let (None, Some(trace)) = (&reached, traces.get(&target)) {
            for hop in &trace.hops {
                if hop.ttl.get().get() <= path.ttl {
                    continue;
                }
                let sighting = Sighting {
                    target,
                    sent: hop.sent,
                    answered: hop.answered,
                    median: hop.median,
                };
                if destination.is_some_and(|d| hop.responders.contains(&d)) {
                    reached = Some((hop.ttl, Some(sighting)));
                    break;
                }
                let places: Vec<Place> = if hop.responders.is_empty() {
                    vec![path.silent(hop.ttl)]
                } else {
                    hop.responders
                        .iter()
                        .map(|&address| Place::Router {
                            address,
                            ttl: hop.ttl,
                        })
                        .collect()
                };
                map.advance(&mut path, places.clone(), hop.ttl, &only, None);
                for place in &places {
                    map.sight(place, sighting.clone());
                }
            }
            if reached.is_none()
                && let Some(ttl) = trace.silent_from
                && ttl.get().get() > path.ttl
            {
                let silent = path.silent(ttl);
                map.advance(&mut path, vec![silent], ttl, &only, None);
            }
        }

        // The target sits where its path reached it.
        let Some((ttl, sighting)) = reached else {
            unplaced.push((target, path));
            continue;
        };
        let place = map.node(Place::Role(Role::Target(target)), Depth::of(ttl));
        let gap = ttl.get().get() != path.ttl + 1;
        map.step(
            &path.at,
            std::slice::from_ref(&place),
            Link::Path { gap },
            &only,
        );
        for role in at_target {
            map.merge(&place, role);
        }
        if let Some(sighting) = sighting {
            map.sight(&place, sighting);
        }
    }

    // A target no trace reached is an unknown distance past what leads to it, so it
    // goes in the bottom row: beside the deepest target placed, or below every other
    // node, and always past its own path.
    let bottom = map
        .nodes
        .values()
        .map(|n| match n.place {
            Place::Role(Role::Target(_)) => n.depth,
            _ => n.depth.next(),
        })
        .max()
        .unwrap_or(Depth::KERNEL);
    for (target, path) in unplaced {
        let depth = bottom.max(path.depth.next());
        let place = map.node(Place::Role(Role::Target(target)), depth);
        map.step(
            &path.at,
            std::slice::from_ref(&place),
            Link::Path { gap: true },
            &BTreeSet::from([target]),
        );
    }
    map.finish()
}

/// How an edge was found, before its kind can be told: whether a path edge's ends
/// are probed depends on every path merged into them.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Link {
    Path { gap: bool },
    OffPath,
}

/// Where one target's path has got to.
struct Path {
    at: Vec<Place>,
    ttl: u8,
    depth: Depth,
}

impl Path {
    /// The silent place at `ttl`, next on from here.
    fn silent(&self, ttl: HopTtl) -> Place {
        let mut after = self.at.clone();
        after.sort();
        Place::Silent { after, ttl }
    }
}

struct Builder<'a> {
    statuses: &'a BTreeMap<Role, Health>,
    nodes: BTreeMap<Place, MapNode>,
    edges: BTreeMap<(Place, Place), (Link, BTreeSet<TargetId>)>,
}

impl Builder<'_> {
    /// The node at `place`, made if new, with its role merged in if it is graded.
    fn node(&mut self, place: Place, depth: Depth) -> Place {
        let role = match &place {
            Place::Role(role) => Some(role.clone()),
            Place::Router { .. } | Place::Silent { .. } => None,
        };
        self.nodes.entry(place.clone()).or_insert_with(|| MapNode {
            place: place.clone(),
            depth,
            roles: BTreeMap::new(),
            health: None,
            sightings: Vec::new(),
        });
        if let Some(role) = role {
            self.merge(&place, &role);
        }
        place
    }

    fn merge(&mut self, place: &Place, role: &Role) {
        if let (Some(node), Some(&health)) = (self.nodes.get_mut(place), self.statuses.get(role)) {
            node.roles.insert(role.clone(), health);
        }
    }

    fn sight(&mut self, place: &Place, sighting: Sighting) {
        if let Some(node) = self.nodes.get_mut(place) {
            node.sightings.push(sighting);
        }
    }

    /// Moves `path` on to `places`, all at `ttl`. `role` is the hop role measured
    /// there, if any, which merges into its place.
    fn advance(
        &mut self,
        path: &mut Path,
        places: Vec<Place>,
        ttl: HopTtl,
        targets: &BTreeSet<TargetId>,
        role: Option<&Role>,
    ) {
        let depth = Depth::of(ttl);
        for place in &places {
            self.node(place.clone(), depth);
            if let Some(role) = role {
                self.merge(place, role);
            }
        }
        let gap = ttl.get().get() != path.ttl + 1;
        self.step(&path.at, &places, Link::Path { gap }, targets);
        *path = Path {
            at: places,
            ttl: ttl.get().get(),
            depth,
        };
    }

    fn step(&mut self, from: &[Place], to: &[Place], link: Link, targets: &BTreeSet<TargetId>) {
        for a in from {
            for b in to {
                let (known, on) = self
                    .edges
                    .entry((a.clone(), b.clone()))
                    .or_insert((link, BTreeSet::new()));
                if let (Link::Path { gap }, Link::Path { gap: other }) = (known, link) {
                    *gap = *gap && other;
                }
                on.extend(targets.iter().copied());
            }
        }
    }

    fn finish(self) -> NetworkMap {
        let probed = |place: &Place| self.nodes.get(place).is_some_and(|n| !n.roles.is_empty());
        let edges = self
            .edges
            .iter()
            .map(|((from, to), (link, targets))| MapEdge {
                from: from.clone(),
                to: to.clone(),
                kind: match link {
                    Link::OffPath => EdgeKind::OffPath,
                    Link::Path { gap: true } => EdgeKind::Unmeasured,
                    Link::Path { gap: false } if probed(from) && probed(to) => EdgeKind::Probed,
                    Link::Path { gap: false } => EdgeKind::Traced,
                },
                targets: targets.clone(),
            })
            .collect();
        let mut nodes: Vec<MapNode> = self
            .nodes
            .into_values()
            .map(|mut node| {
                node.health = node.roles.values().copied().reduce(Health::worse);
                node
            })
            .collect();
        nodes.sort_by(|a, b| (a.depth, &a.place).cmp(&(b.depth, &b.place)));
        NetworkMap { nodes, edges }
    }
}
