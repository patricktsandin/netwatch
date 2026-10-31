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
