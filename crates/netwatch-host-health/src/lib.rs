//! The Host health context (supporting): is the host itself the problem?
//! (ADR-0008).
//!
//! # Responsibility
//!
//! This crate interprets raw *counter readings* and *gauges* from the kernel's
//! *telemetry sources*, and grades the kernel layer's *status*. So far the sources
//! are: protocol counters (`/proc/net/snmp`, `/proc/net/netstat`), per-CPU packet
//! processing (`/proc/net/softnet_stat`), and pressure stall information
//! (`/proc/pressure`).
//!
//! # Invariants
//!
//! - A *counter reset* is detected and never reported as negative traffic. The
//!   verified crate's `counter_increase` proves it.
//! - A *disabled source* is reported as disabled, never as healthy.
//!
//! # What it hides
//!
//! Which counters matter, and which direction counts as "bad" for each. Callers get a
//! [`KernelReport`]: a status, the counters that got worse, and the pressure.
//!
//! This crate does no I/O (ADR-0012).

use std::collections::BTreeMap;

use netwatch_verified::counter_increase;

/// The kernel counters that mean packets were dropped or mangled on this host:
/// any increase in them is bad (ADR-0008).
pub const WATCHED: &[&str] = &[
    "Ip.InDiscards",
    "Ip.InHdrErrors",
    "Ip.OutNoRoutes",
    "Ip.ReasmFails",
    "Udp.RcvbufErrors",
    "Udp.SndbufErrors",
    "Udp.InCsumErrors",
    "TcpExt.ListenOverflows",
    "TcpExt.ListenDrops",
    "TcpExt.TCPBacklogDrop",
    "Softnet.Dropped",
    "Softnet.TimeSqueeze",
];

/// Drops in these mean the host itself discarded packets it should have handled,
/// which makes the kernel layer `Failing` rather than `Degraded`.
const SEVERE: &[&str] = &["Softnet.Dropped", "Ip.InDiscards"];

/// Above this share of time stalled on CPU (PSI `some avg10`, in percent), probe
/// timing can't be trusted, so the kernel layer is `Degraded`.
const CPU_PRESSURE_LIMIT: f64 = 20.0;

/// One reading of every telemetry source, already parsed by the procfs adapter.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Reading {
    /// Every counter read, by `Group.Name`.
    pub counters: BTreeMap<String, u64>,
    /// PSI `some avg10` for CPU, memory, and I/O, in percent, if available.
    pub pressure: Option<Pressure>,
}

/// Pressure stall information: the share of the last 10 s in which some task
/// stalled on each resource, in percent.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Pressure {
    /// Stalled on CPU.
    pub cpu: f64,
    /// Stalled on memory.
    pub memory: f64,
    /// Stalled on I/O.
    pub io: f64,
}

/// How the kernel layer is doing (glossary: *status*).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum KernelStatus {
    /// Nothing got worse.
    Healthy,
    /// Some watched counter grew, or the CPU is under pressure.
    Degraded,
    /// The host itself dropped packets.
    Failing,
    /// Not enough readings yet to tell.
    Unknown,
}

/// The kernel layer's state after a reading.
#[derive(Debug, Clone, PartialEq)]
pub struct KernelReport {
    /// The graded status.
    pub status: KernelStatus,
    /// Watched counters that grew since the previous reading, by how much.
    pub increases: BTreeMap<String, u64>,
    /// Counters that went backwards: reset or wrapped, so their difference means
    /// nothing.
    pub resets: Vec<String>,
    /// The latest pressure, if available.
    pub pressure: Option<Pressure>,
}

/// Compares each reading with the previous one.
#[derive(Debug, Default)]
pub struct KernelWatch {
    previous: Option<Reading>,
}

impl KernelWatch {
    /// A watch with no readings yet.
    pub fn new() -> Self {
        Self::default()
    }

    /// Takes a reading and reports what changed since the previous one. The first
    /// reading has nothing to compare against, so it reports `Unknown`.
    pub fn observe(&mut self, reading: Reading) -> KernelReport {
        let Some(previous) = self.previous.replace(reading.clone()) else {
            return KernelReport {
                status: KernelStatus::Unknown,
                increases: BTreeMap::new(),
                resets: Vec::new(),
                pressure: reading.pressure,
            };
        };
        let mut increases = BTreeMap::new();
        let mut resets = Vec::new();
        for &name in WATCHED {
            let (Some(&before), Some(&now)) =
                (previous.counters.get(name), reading.counters.get(name))
            else {
                continue;
            };
            match counter_increase(before, now) {
                Some(0) => {}
                Some(increase) => {
                    increases.insert(name.to_owned(), increase);
                }
                None => resets.push(name.to_owned()),
            }
        }
        let severe = increases.keys().any(|name| SEVERE.contains(&name.as_str()));
        let pressured = reading.pressure.is_some_and(|p| p.cpu > CPU_PRESSURE_LIMIT);
        let status = if severe {
            KernelStatus::Failing
        } else if !increases.is_empty() || pressured {
            KernelStatus::Degraded
        } else {
            KernelStatus::Healthy
        };
        KernelReport {
            status,
            increases,
            resets,
            pressure: reading.pressure,
        }
    }
}
