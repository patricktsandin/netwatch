//! The Probing context (core): sending probes and interpreting what comes back
//! (ADR-0002, ADR-0004, ADR-0023).
//!
//! # Responsibility
//!
//! This crate runs a *probe stream* for every *role* at a fixed *cadence*. It covers:
//! - *loopback probes*, *link-layer probes*, *hop probes*, *echo probes*, *DNS probes*,
//!   and *HTTP probes*
//! - *reply matching*: pairing each received packet with the probe that caused it
//! - *hop discovery* for each target, and *stream sharing* between targets whose hops
//!   resolve identically
//!
//! The output is *samples*, each carrying exactly one *probe outcome*.
//!
//! # Invariants
//!
//! - Every probe gets exactly one outcome. Timeouts, ICMP errors, and send failures are
//!   always distinct outcomes.
//! - Replies are matched by sequence number and token, never by arrival order.
//! - Probes are sent on schedule, never relative to the last reply.
//!
//! These are specified in `specs/probing.qnt` and checked against [`ProbeStream`] by
//! trace conformance.
//!
//! # What it hides
//!
//! Packet formats, how sequence numbers are allocated and wrap around, and how
//! scheduling works. This crate does no I/O: it asks the sockets adapter to send packets
//! by returning [`Output::Send`], and receives the replies through
//! [`ProbeStream::reply`] (ADR-0012).

mod config;
pub mod health;
pub mod incident;
mod stream;

pub use config::{InvalidStreamConfig, SequenceSpace, StreamConfig};
pub use health::Health;
pub use stream::{
    Output, ProbeIndex, ProbeOutcome, ProbeSequence, ProbeStream, Reply, ReplyKind, SendFailure,
    UnreachableCode,
};
