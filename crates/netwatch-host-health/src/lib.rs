//! The Host health context (supporting): is the host itself the problem?
//! (ADR-0008).
//!
//! # Responsibility
//!
//! This crate interprets raw *counter readings* and *gauges* from the kernel's
//! *telemetry sources*, and grades the *status* of the kernel layer and the NIC
//! layer. So far the sources are:
//! - kernel: protocol counters (`/proc/net/snmp`, `/proc/net/netstat`), per-CPU packet
//!   processing (`/proc/net/softnet_stat`), and pressure stall information
//!   (`/proc/pressure`)
//! - NIC: the device's own error and drop counters (`IFLA_STATS64`)
//! - the kernel log: network-relevant lines from `/dev/kmsg`, classified by [`classify`]
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
//! [`LayerReport`]: a status, the counters that got worse, and any pressure.
//!
//! This crate does no I/O (ADR-0012).

use std::collections::BTreeMap;

mod kernel_log;

pub use kernel_log::{KernelLogKind, KernelLogLine, classify};

use netwatch_verified::counter_increase;

/// How bad an increase in a watched counter is.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Severity {
    /// Something got worse, but the layer still works.
    Degrades,
    /// The layer itself is losing packets.
    Fails,
}

/// A counter to watch, and how bad its increase is.
pub type Rule = (&'static str, Severity);

/// The kernel counters that mean packets were dropped or mangled on this host
/// (ADR-0008). Drops the host itself made are severe.
pub const KERNEL_RULES: &[Rule] = &[
    ("Ip.InDiscards", Severity::Fails),
    ("Ip.InHdrErrors", Severity::Degrades),
    ("Ip.OutNoRoutes", Severity::Degrades),
    ("Ip.ReasmFails", Severity::Degrades),
    ("Udp.RcvbufErrors", Severity::Degrades),
    ("Udp.SndbufErrors", Severity::Degrades),
    ("Udp.InCsumErrors", Severity::Degrades),
    ("TcpExt.ListenOverflows", Severity::Degrades),
    ("TcpExt.ListenDrops", Severity::Degrades),
    ("TcpExt.TCPBacklogDrop", Severity::Degrades),
    ("Softnet.Dropped", Severity::Fails),
    ("Softnet.TimeSqueeze", Severity::Degrades),
    ("Conntrack.Drop", Severity::Fails),
    ("Conntrack.EarlyDrop", Severity::Degrades),
    ("Conntrack.InsertFailed", Severity::Degrades),
    ("Arp.TableFulls", Severity::Degrades),
    ("Arp.UnresolvedDiscards", Severity::Degrades),
    ("Ndisc.TableFulls", Severity::Degrades),
    ("Ndisc.UnresolvedDiscards", Severity::Degrades),
];

/// The NIC's own counters (`IFLA_STATS64`). Errors on the wire degrade it; packets
/// the device or driver dropped or missed fail it.
pub const NIC_RULES: &[Rule] = &[
    ("Link.RxErrors", Severity::Degrades),
    ("Link.TxErrors", Severity::Degrades),
    ("Link.RxCrcErrors", Severity::Degrades),
    ("Link.RxFrameErrors", Severity::Degrades),
    ("Link.TxCarrierErrors", Severity::Degrades),
    ("Link.Collisions", Severity::Degrades),
    ("Link.RxDropped", Severity::Fails),
    ("Link.TxDropped", Severity::Fails),
    ("Link.RxMissedErrors", Severity::Fails),
    ("Link.RxFifoErrors", Severity::Fails),
];

/// Above this share of time stalled on CPU (PSI `some avg10`, in percent), probe
/// timing can't be trusted, so the kernel layer is `Degraded`.
const CPU_PRESSURE_LIMIT: f64 = 20.0;
/// Connection-tracking fill levels, in percent: nearly full degrades, and at the
/// limit new connections are being dropped.
const CONNTRACK_NEARLY_FULL: f64 = 80.0;
const CONNTRACK_FULL: f64 = 99.0;
/// A clock whose estimated error exceeds this (µs) can't order samples across hosts
/// or reconcile them with logs, so the kernel layer is `Degraded`.
const CLOCK_ERROR_LIMIT_US: i64 = 100_000;

/// One reading of every telemetry source, already parsed by the procfs adapter.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Reading {
    /// Every counter read, by `Group.Name`.
    pub counters: BTreeMap<String, u64>,
    /// PSI `some avg10` for CPU, memory, and I/O, in percent, if available.
    pub pressure: Option<Pressure>,
    /// How full the connection-tracking table is, in percent, if conntrack is loaded.
    /// A full table drops every new connection silently.
    pub conntrack_fill: Option<f64>,
    /// Whether the clock is NTP-synchronized, and its estimated error in µs. An
    /// unsynchronized clock skews every timestamp (ADR-0004).
    pub clock: Option<(bool, i64)>,
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

/// How a layer is doing (glossary: *status*).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LayerStatus {
    /// Nothing got worse.
    Healthy,
    /// Some watched counter grew, or the CPU is under pressure.
    Degraded,
    /// The layer itself dropped packets.
    Failing,
    /// Not enough readings yet to tell.
    Unknown,
}

/// A layer's state after a reading.
#[derive(Debug, Clone, PartialEq)]
pub struct LayerReport {
    /// The graded status.
    pub status: LayerStatus,
    /// Watched counters that grew since the previous reading, by how much.
    pub increases: BTreeMap<String, u64>,
    /// Counters that went backwards: reset or wrapped, so their difference means
    /// nothing.
    pub resets: Vec<String>,
    /// The latest pressure, if available.
    pub pressure: Option<Pressure>,
    /// The connection-tracking table's fill level, in percent, if available.
    pub conntrack_fill: Option<f64>,
    /// Whether the clock is synchronized, and its estimated error in µs.
    pub clock: Option<(bool, i64)>,
}

/// Compares each reading of one layer with the previous one, by that layer's rules.
///
/// A watch must only ever see one source. When the source changes, such as the NIC
/// after a failover, start a new watch: another device's counters aren't
/// comparable.
#[derive(Debug)]
pub struct CounterWatch {
    rules: &'static [Rule],
    previous: Option<Reading>,
}

impl CounterWatch {
    /// A watch with no readings yet.
    pub fn new(rules: &'static [Rule]) -> Self {
        Self {
            rules,
            previous: None,
        }
    }

    /// Takes a reading and reports what changed since the previous one. The first
    /// reading has nothing to compare against, so it reports `Unknown`.
    pub fn observe(&mut self, reading: Reading) -> LayerReport {
        let Some(previous) = self.previous.replace(reading.clone()) else {
            return LayerReport {
                status: LayerStatus::Unknown,
                increases: BTreeMap::new(),
                resets: Vec::new(),
                pressure: reading.pressure,
                conntrack_fill: reading.conntrack_fill,
                clock: reading.clock,
            };
        };
        let mut increases = BTreeMap::new();
        let mut resets = Vec::new();
        let mut severe = false;
        for &(name, severity) in self.rules {
            let (Some(&before), Some(&now)) =
                (previous.counters.get(name), reading.counters.get(name))
            else {
                continue;
            };
            match counter_increase(before, now) {
                Some(0) => {}
                Some(increase) => {
                    increases.insert(name.to_owned(), increase);
                    severe |= severity == Severity::Fails;
                }
                None => resets.push(name.to_owned()),
            }
        }
        let pressured = reading.pressure.is_some_and(|p| p.cpu > CPU_PRESSURE_LIMIT);
        let conntrack = reading.conntrack_fill.unwrap_or(0.0);
        severe |= conntrack >= CONNTRACK_FULL;
        let clock_untrusted = reading
            .clock
            .is_some_and(|(synchronized, error)| !synchronized || error > CLOCK_ERROR_LIMIT_US);
        let pressured = pressured || conntrack >= CONNTRACK_NEARLY_FULL || clock_untrusted;
        let status = if severe {
            LayerStatus::Failing
        } else if !increases.is_empty() || pressured {
            LayerStatus::Degraded
        } else {
            LayerStatus::Healthy
        };
        LayerReport {
            status,
            increases,
            resets,
            pressure: reading.pressure,
            conntrack_fill: reading.conntrack_fill,
            clock: reading.clock,
        }
    }
}
