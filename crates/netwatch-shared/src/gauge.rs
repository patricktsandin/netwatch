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
    /// The Wi-Fi access point's signal at this host (ADR-0036).
    WifiSignal,
    /// The Wi-Fi link's current transmit rate.
    WifiTxBitrate,
    /// The Wi-Fi link's current receive rate.
    WifiRxBitrate,
    /// The modem's own 0–100 % signal quality.
    CellularQuality,
    /// LTE or 5G NR reference signal received power.
    CellularRsrp,
    /// LTE or 5G NR reference signal received quality.
    CellularRsrq,
    /// LTE or 5G NR signal-to-noise ratio.
    CellularSnr,
}

/// How a gauge's values are measured, which is also how they are formatted.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum GaugeUnit {
    /// Bits per second.
    BitsPerSecond,
    /// Decibel-milliwatts, for absolute signal power.
    Dbm,
    /// Decibels, for ratios.
    Db,
    /// Percent.
    Percent,
}

impl Gauge {
    /// Every gauge, in display order.
    pub const ALL: &[Gauge] = &[
        Gauge::RxThroughput,
        Gauge::TxThroughput,
        Gauge::WifiSignal,
        Gauge::WifiTxBitrate,
        Gauge::WifiRxBitrate,
        Gauge::CellularQuality,
        Gauge::CellularRsrp,
        Gauge::CellularRsrq,
        Gauge::CellularSnr,
    ];

    /// The stable key, used wherever a gauge crosses a boundary as text.
    pub const fn key(self) -> &'static str {
        match self {
            Gauge::RxThroughput => "rx-throughput",
            Gauge::TxThroughput => "tx-throughput",
            Gauge::WifiSignal => "wifi-signal",
            Gauge::WifiTxBitrate => "wifi-tx-bitrate",
            Gauge::WifiRxBitrate => "wifi-rx-bitrate",
            Gauge::CellularQuality => "cellular-quality",
            Gauge::CellularRsrp => "cellular-rsrp",
            Gauge::CellularRsrq => "cellular-rsrq",
            Gauge::CellularSnr => "cellular-snr",
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
            Gauge::WifiSignal => "signal",
            Gauge::WifiTxBitrate => "tx rate",
            Gauge::WifiRxBitrate => "rx rate",
            Gauge::CellularQuality => "quality",
            Gauge::CellularRsrp => "RSRP",
            Gauge::CellularRsrq => "RSRQ",
            Gauge::CellularSnr => "SNR",
        }
    }

    /// The unit every value of this gauge is in.
    pub const fn unit(self) -> GaugeUnit {
        match self {
            Gauge::RxThroughput
            | Gauge::TxThroughput
            | Gauge::WifiTxBitrate
            | Gauge::WifiRxBitrate => GaugeUnit::BitsPerSecond,
            Gauge::WifiSignal | Gauge::CellularRsrp => GaugeUnit::Dbm,
            Gauge::CellularRsrq | Gauge::CellularSnr => GaugeUnit::Db,
            Gauge::CellularQuality => GaugeUnit::Percent,
        }
    }
}

impl GaugeUnit {
    /// The stable key, as published.
    pub const fn key(self) -> &'static str {
        match self {
            GaugeUnit::BitsPerSecond => "bit/s",
            GaugeUnit::Dbm => "dBm",
            GaugeUnit::Db => "dB",
            GaugeUnit::Percent => "%",
        }
    }
}
