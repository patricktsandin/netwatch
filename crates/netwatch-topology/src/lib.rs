//! The Topology context (core): what the host's path to the network looks like right
//! now, and the record of how it has changed (ADR-0002, ADR-0003).
//!
//! # Responsibility
//!
//! This crate turns *observations* of links, addresses, routes, neighbors, access
//! points, and cells into:
//! - an *egress path* for each destination
//! - the current *identity* behind every *role*
//! - *topology events*, including the start of each new *identity epoch*
//!
//! # Invariants
//!
//! - A sample is never attributed to an identity it didn't measure.
//! - Only the topology aggregate assigns identity epochs, and epochs only ever
//!   advance.
//!
//! These invariants are specified in `specs/topology.qnt` and checked against this code
//! by replaying traces generated from that spec.
//!
//! # What it hides
//!
//! How the kernel describes topology (netlink attributes, route tables, metrics, policy
//! rules). Other contexts see only roles, identities, epochs, and egress paths.
//!
//! This crate does no I/O. It is a pure state machine: *observations* go in, and
//! *commands* and *facts* come out (ADR-0012).
//!
//! # Using it
//!
//! Feed every [`Observation`] to [`Topology::observe`], and act on the [`Output`]s in
//! the order they are returned:
//! - persist and publish each [`TopologyEvent`]
//! - carry out each [`Command`]
//!
//! To pin a probe, take the role's current [`Binding`] from [`Topology::binding`]
//! when the probe is scheduled (ADR-0020).

mod aggregate;
pub mod egress;
mod observation;
mod output;

pub use aggregate::{Binding, Topology};
pub use egress::{EgressPath, InterfaceKind};
pub use observation::{Observation, ResyncGeneration, Snapshot};
pub use output::{Command, Output, TopologyEvent};
