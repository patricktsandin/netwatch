//! Anti-corruption layer for probe I/O (ADR-0007, ADR-0020, ADR-0023, ADR-0024).
//!
//! This crate carries out the Probing context's send *commands* and turns what comes
//! back into [`Reply`](netwatch_probing::Reply) values.
//!
//! - [`icmp`] encodes echo requests and parses echo replies. It is pure, and proven
//!   with Kani never to panic, for every possible input.
//! - [`EchoSocket`] is an unprivileged ICMP or ICMPv6 datagram socket, pinned to one
//!   interface with `SO_BINDTOIFINDEX` (ADR-0024), so that its probes can't take any
//!   other path.
//!
//! Not yet implemented: ICMP errors through `IP_RECVERR` (which would give the
//! `Unreachable` outcome and hop probes), raw-socket fallback, and kernel receive
//! timestamps. Every receive timestamp is currently taken in userspace.

pub mod icmp;
mod socket;

pub use socket::EchoSocket;
