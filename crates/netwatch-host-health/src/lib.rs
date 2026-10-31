//! The Host health context (supporting): is the host itself the problem?
//! (ADR-0008).
//!
//! # Responsibility
//!
//! This crate interprets raw *counter readings* and *gauges* from every *telemetry
//! source*: protocol counters, softnet, kernel tables, qdisc stats, PSI, clock health,
//! config drift, kernel log lines, and drop reasons. It also covers the NIC's driver
//! counters and its wireless/cellular gauges.
//!
//! The output is a *status* for the kernel layer and the NIC layer, plus *domain events*
//! such as *suspend gaps*, *config drift*, and *disabled sources*.
//!
//! # Invariants
//!
//! - A *counter reset* is detected and never reported as negative traffic.
//! - A *disabled source* is reported as disabled, never as healthy.
//!
//! # What it hides
//!
//! Which counters matter and which direction counts as "bad" for each of them. Callers
//! get statuses and events, not hundreds of raw counters.
//!
//! This crate does no I/O (ADR-0012).
