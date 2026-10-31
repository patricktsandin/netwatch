//! Algorithms proven correct with Verus: the *verified crate* (ADR-0016).
//!
//! # Responsibility
//!
//! This crate holds the few algorithms where a subtle bug would quietly corrupt every
//! number netwatch reports:
//! - reply matching and probe-sequence wraparound (used by Probing)
//! - counter wraparound and counter-reset detection (used by Host health)
//! - the merge algebra for rollups and percentiles (used by History)
//! - identity-epoch advancement (used by Topology)
//!
//! # Rules
//!
//! - Everything is written inside `verus!` and fully proven. No `assume`, and no
//!   `external_body` without an ADR explaining why.
//! - The interface uses plain integer newtypes, not `uom`, because Verus can't reason
//!   about `uom` (ADR-0014). Callers convert at this boundary.
//! - Proofs are erased in a normal build, so `cargo build` compiles this crate like any
//!   other. Verify it with `cargo verus verify -p netwatch-verified`.

#![allow(missing_docs)] // `verus!` expands to items that rustdoc can't see; document inside it.

#[allow(unused_imports)]
use vstd::prelude::*;

verus! {} // verus!
