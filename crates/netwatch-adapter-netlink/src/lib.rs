//! Anti-corruption layer between the kernel's netlink families and the domain.
//!
//! This crate subscribes to rtnetlink multicast groups (link, IPv4/IPv6 addresses and
//! routes, neighbors), asks `RTM_GETROUTE` for egress paths, and reads ethtool, nl80211,
//! and tc statistics. It parses every message once into Topology and Host health
//! *observations* (ADR-0003, ADR-0014).
//!
//! Nothing netlink-shaped leaves this crate. If a domain type ever needs a netlink
//! attribute, the translation belongs here.
