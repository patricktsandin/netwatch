//! Gauges: named quantities that are graphed but aren't probe outcomes (ADR-0032).

/// A quantity a role reports over time, with a fixed unit (glossary: *gauge*).
///
/// Keys are stored and published, so a variant's key never changes once released.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Gauge {
    /// Bytes the NIC received, as a rate.
    RxThroughput,
    /// Bytes the NIC transmitted, as a rate.
    TxThroughput,
}

/// How a gauge's values are measured, which is also how they are formatted.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum GaugeUnit {
    /// Bits per second.
    BitsPerSecond,
}

impl Gauge {
    /// Every gauge, in display order.
    pub const ALL: &[Gauge] = &[Gauge::RxThroughput, Gauge::TxThroughput];

    /// The stable key, used wherever a gauge crosses a boundary as text.
    pub const fn key(self) -> &'static str {
        match self {
            Gauge::RxThroughput => "rx-throughput",
            Gauge::TxThroughput => "tx-throughput",
        }
    }

    /// Parses a [`key`](Self::key). `None` for anything else.
    pub fn from_key(key: &str) -> Option<Gauge> {
        Gauge::ALL.iter().copied().find(|g| g.key() == key)
    }

    /// A short name, for people.
    pub const fn label(self) -> &'static str {
        match self {
            Gauge::RxThroughput => "received",
            Gauge::TxThroughput => "sent",
        }
    }

    /// The unit every value of this gauge is in.
    pub const fn unit(self) -> GaugeUnit {
        match self {
            Gauge::RxThroughput | Gauge::TxThroughput => GaugeUnit::BitsPerSecond,
        }
    }
}

impl GaugeUnit {
    /// The stable key, as published.
    pub const fn key(self) -> &'static str {
        match self {
            GaugeUnit::BitsPerSecond => "bit/s",
        }
    }
}
