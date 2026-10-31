//! Anti-corruption layer for the kernel's text interfaces: `/proc/net/*`, `/sys`,
//! `/proc/pressure/*`, `adjtimex`, sysctls, and `/dev/kmsg`.
//!
//! This crate parses each file format once into Host health *counter readings*, *gauges*,
//! and *kernel log lines* (ADR-0008). Every parser has Kani proof harnesses (ADR-0016).
//! The parsers accept unknown fields, because kernel versions add new ones.
