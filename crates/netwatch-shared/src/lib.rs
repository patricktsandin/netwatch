//! Value objects shared by every domain context: the *shared crate* of the
//! [glossary](../../../docs/glossary.md).
//!
//! # Responsibility
//!
//! This crate holds only the domain values whose meaning is the same in every bounded
//! context, such as interface indexes, roles, identities, identity epochs, and instants.
//! A value that only one context cares about belongs in that context's crate instead.
//!
//! # Invariants
//!
//! Every type here is a value object: immutable, compared by value, and impossible to
//! construct in an invalid state (ADR-0014). Raw OS data is parsed into these types
//! by the adapter crates, never here.
//!
//! # Why so small
//!
//! Every domain crate depends on this one, so any change here ripples through all of
//! them. Keep it small, and move things into it only after two contexts have
//! genuinely needed the same concept.

pub mod address;
mod gauge;
mod identity;
mod role;
mod time;

pub use gauge::{Gauge, GaugeUnit};
pub use identity::{Identity, IdentityEpoch, InterfaceIndex, InterfaceName, InvalidInterfaceName};
pub use role::{HopTtl, Role, TargetId};
pub use time::{Instant, WallTime};

/// Physical quantities (ADR-0014), re-exported so that every crate uses the same
/// `uom` configuration.
pub use uom;
