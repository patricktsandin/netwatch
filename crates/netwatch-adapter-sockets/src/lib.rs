//! Anti-corruption layer for probe I/O: ICMP/ICMPv6 datagram and raw sockets, UDP,
//! `AF_PACKET` (ARP and neighbor solicitation), DNS, and HTTP(S).
//!
//! This crate carries out the Probing context's send *commands* and turns everything it
//! receives into *observations*, including ICMP errors from `IP_RECVERR` and kernel
//! timestamps. Every packet parser here has Kani proof harnesses (ADR-0016).
//!
//! At startup it detects which socket modes and capabilities are available, and
//! reports the ones that are missing as *disabled sources* (ADR-0007).
