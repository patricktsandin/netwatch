//! Anti-corruption layer for probe I/O (ADR-0007, ADR-0020, ADR-0023, ADR-0024).
//!
//! This crate carries out the Probing context's send *commands* and turns what comes
//! back into [`Reply`](netwatch_probing::Reply) values.
//!
//! - [`icmp`] encodes echo requests and parses echo replies. It is pure, and proven
//!   with Kani never to panic, for every possible input.
//! - [`ProbeSocket`] is an unprivileged ICMP or ICMPv6 datagram socket, pinned to one
//!   interface with `SO_BINDTOIFINDEX` (ADR-0024), optionally with a limited TTL for
//!   hop probes. ICMP errors arrive through `IP_RECVERR`, and [`errqueue`] parses
//!   their control messages, which Kani proves panic-free.
//! - [`DnsSocket`] is a pinned UDP socket for DNS probes. [`dns`] encodes the query
//!   and parses responses, also Kani-proven.
//!
//! Not yet implemented: the raw-socket fallback and kernel receive timestamps. Every
//! receive timestamp is currently taken in userspace.

pub mod dns;
mod dns_socket;
pub mod errqueue;
pub mod icmp;
mod socket;

pub use dns_socket::DnsSocket;
pub use socket::{ProbeSocket, Received};
