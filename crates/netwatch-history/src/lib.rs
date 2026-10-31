//! The History context (supporting): what happened, at every resolution (ADR-0006).
//!
//! # Responsibility
//!
//! This crate turns *samples* into *rollups* at each *tier* (raw, 1m, 1h), merges
//! rollups, and decides what the *retention* policy discards.
//!
//! # Invariants
//!
//! - A rollup never spans two *identity epochs*.
//! - *Merge* is associative and commutative. The algebra is proven in the verified
//!   crate, so rolling up 1m into 1h gives the same result as computing 1h directly
//!   from raw samples.
//!
//! # What it hides
//!
//! How percentiles are summarized so that they can be merged, and how tiers are laid
//! out. This context knows nothing about networks: it aggregates facts keyed by
//! *role*, *identity*, and *identity epoch*, and nothing more.
//!
//! This crate does no I/O. Persistence lives in `netwatch-adapter-sqlite` (ADR-0012).
