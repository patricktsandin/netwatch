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
//!
//! Status: samples and their rollups ([`roll_up`], [`coarsen`]) exist. Persisting
//! the rollup tiers and their retention comes next.

mod reading;
mod rollup;

pub use reading::{Reading, ReadingRollup, ReadingSummary, coarsen_readings, roll_up_readings};
pub use rollup::{Histogram, Rollup, Tier, coarsen, roll_up};

use netwatch_shared::uom::si::f64::Time;
use netwatch_shared::{IdentityEpoch, Role, WallTime};

/// One measurement of one role (glossary: *sample*).
#[derive(Debug, Clone, PartialEq)]
pub struct Sample {
    /// What was measured.
    pub role: Role,
    /// The epoch of the *pin* the probe traveled under. It names exactly the identity
    /// that was measured (ADR-0020).
    pub epoch: IdentityEpoch,
    /// When the measurement was resolved.
    pub at: WallTime,
    /// What was measured.
    pub measurement: Measurement,
}

/// A measurement's value: a round-trip time, or a loss together with its reason.
/// These are never collapsed into a missing value (ADR-0004).
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Measurement {
    /// Answered after this round-trip time.
    RoundTrip(Time),
    /// Not answered.
    Lost(Loss),
}

/// Why a probe got no usable answer.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Loss {
    /// No answer within the timeout.
    TimedOut,
    /// An ICMP destination-unreachable error, with its code.
    Unreachable(u8),
    /// The server answered with an error, with its code (a DNS rcode).
    Refused(u8),
    /// A response that wasn't the expected one, with what arrived (an HTTP status).
    UnexpectedResponse(u16),
    /// The probe couldn't be sent.
    SendFailed,
}
