//! The radio roles (ADR-0036): what a Wi-Fi device or a cellular modem reports,
//! graded, summarized, and turned into gauge readings.
//!
//! A radio may be a backup that carries no traffic, so standing by (a Wi-Fi device
//! that isn't associated, a modem that is disabled) is `Unknown`, not a failure. Once
//! a radio is in use, its signal grades it.

use netwatch_shared::Gauge;

use crate::LayerStatus;

/// A Wi-Fi device's link, as nl80211 reports it.
#[derive(Debug, Clone, PartialEq)]
pub struct WifiLink {
    /// The access point it's associated with, if any.
    pub associated: Option<Association>,
}

/// A Wi-Fi association.
#[derive(Debug, Clone, PartialEq)]
pub struct Association {
    /// The access point (glossary: *BSSID*).
    pub bssid: [u8; 6],
    /// The access point's signal, averaged by the driver where it can.
    pub signal_dbm: Option<i8>,
    /// The rate frames are sent at now.
    pub tx_bitrate_bps: Option<f64>,
    /// The rate frames are received at now.
    pub rx_bitrate_bps: Option<f64>,
}

/// A modem's state, as ModemManager reports it, folded to what grading needs.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ModemState {
    /// The modem failed (no SIM, or a hardware fault).
    Failed,
    /// Turned off: standing by.
    Disabled,
    /// Starting up, locked by a PIN, or changing state.
    Busy,
    /// Enabled, but not registered with a network.
    Searching,
    /// Registered with a network.
    Registered,
    /// Registered, with a data connection.
    Connected,
}

/// The radio access technology a modem is using.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RadioAccess {
    /// 2G: GSM, GPRS, or EDGE.
    Gsm,
    /// 3G: UMTS or HSPA.
    Umts,
    /// 4G.
    Lte,
    /// 5G New Radio.
    Nr5g,
}

/// A modem's link, as ModemManager reports it. Never includes subscriber or device
/// identifiers (ADR-0036).
#[derive(Debug, Clone, PartialEq)]
pub struct CellularLink {
    /// Whether it is enabled and registered.
    pub state: ModemState,
    /// The modem's own 0–100 % signal quality.
    pub quality_percent: Option<u8>,
    /// The technology it is using now.
    pub access: Option<RadioAccess>,
    /// The network's name, as the network announces it.
    pub operator: Option<String>,
    /// Reference signal received power, when extended signal is reported.
    pub rsrp_dbm: Option<f64>,
    /// Reference signal received quality, likewise.
    pub rsrq_db: Option<f64>,
    /// Signal-to-noise ratio, likewise.
    pub snr_db: Option<f64>,
}

/// A radio's grade, its summary for people, what it's attached to, and its readings.
#[derive(Debug, Clone, PartialEq)]
pub struct RadioReport {
    /// The grade.
    pub status: LayerStatus,
    /// One line for the row header, such as `registered on Telia · 5G NR · 64 %`.
    pub summary: String,
    /// The access point or the network: when this changes, that's an event.
    pub attachment: Option<String>,
    /// The gauges the device reported this time.
    pub readings: Vec<(Gauge, f64)>,
}

/// Signal levels for Wi-Fi: -67 dBm is the usual floor for reliable throughput, and
/// below -80 dBm a link barely works.
const WIFI_GOOD_DBM: i8 = -67;
const WIFI_POOR_DBM: i8 = -80;
/// Modem quality levels, in percent.
const CELL_GOOD_PERCENT: u8 = 30;
const CELL_POOR_PERCENT: u8 = 10;

fn bssid_text(bssid: &[u8; 6]) -> String {
    bssid
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect::<Vec<_>>()
        .join(":")
}

/// Grades and summarizes a Wi-Fi device.
pub fn wifi_report(link: &WifiLink) -> RadioReport {
    let Some(a) = &link.associated else {
        return RadioReport {
            status: LayerStatus::Unknown,
            summary: "not associated".to_owned(),
            attachment: None,
            readings: Vec::new(),
        };
    };
    let status = match a.signal_dbm {
        Some(s) if s >= WIFI_GOOD_DBM => LayerStatus::Healthy,
        Some(s) if s >= WIFI_POOR_DBM => LayerStatus::Degraded,
        Some(_) => LayerStatus::Failing,
        None => LayerStatus::Unknown,
    };
    let bssid = bssid_text(&a.bssid);
    let signal = a
        .signal_dbm
        .map(|s| format!(" · {s} dBm"))
        .unwrap_or_default();
    let readings = [
        (Gauge::WifiSignal, a.signal_dbm.map(f64::from)),
        (Gauge::WifiTxBitrate, a.tx_bitrate_bps),
        (Gauge::WifiRxBitrate, a.rx_bitrate_bps),
    ]
    .into_iter()
    .filter_map(|(gauge, value)| Some((gauge, value?)))
    .collect();
    RadioReport {
        status,
        summary: format!("associated with {bssid}{signal}"),
        attachment: Some(bssid),
        readings,
    }
}

fn access_text(access: RadioAccess) -> &'static str {
    match access {
        RadioAccess::Gsm => "GSM",
        RadioAccess::Umts => "UMTS",
        RadioAccess::Lte => "LTE",
        RadioAccess::Nr5g => "5G NR",
    }
}

/// Grades and summarizes a cellular modem.
pub fn cellular_report(link: &CellularLink) -> RadioReport {
    let standing_by = |summary: &str, status| RadioReport {
        status,
        summary: summary.to_owned(),
        attachment: None,
        readings: Vec::new(),
    };
    match link.state {
        ModemState::Disabled => return standing_by("modem disabled", LayerStatus::Unknown),
        ModemState::Busy => return standing_by("modem starting or locked", LayerStatus::Unknown),
        ModemState::Failed => return standing_by("modem failed", LayerStatus::Failing),
        ModemState::Searching | ModemState::Registered | ModemState::Connected => {}
    }
    let registered = link.state != ModemState::Searching;
    let status = match (registered, link.quality_percent) {
        (false, _) => LayerStatus::Failing,
        (true, Some(q)) if q >= CELL_GOOD_PERCENT => LayerStatus::Healthy,
        (true, Some(q)) if q >= CELL_POOR_PERCENT => LayerStatus::Degraded,
        (true, Some(_)) => LayerStatus::Failing,
        (true, None) => LayerStatus::Unknown,
    };
    let network: Vec<&str> = link
        .operator
        .as_deref()
        .filter(|o| !o.is_empty())
        .into_iter()
        .chain(link.access.map(access_text))
        .collect();
    let attachment = (registered && !network.is_empty()).then(|| network.join(" · "));
    let quality = link
        .quality_percent
        .map(|q| format!(" · {q} %"))
        .unwrap_or_default();
    let summary = match (&attachment, registered) {
        (Some(on), true) => format!("registered on {on}{quality}"),
        (None, true) => format!("registered{quality}"),
        (_, false) => format!("searching for a network{quality}"),
    };
    let readings = [
        (Gauge::CellularQuality, link.quality_percent.map(f64::from)),
        (Gauge::CellularRsrp, link.rsrp_dbm),
        (Gauge::CellularRsrq, link.rsrq_db),
        (Gauge::CellularSnr, link.snr_db),
    ]
    .into_iter()
    .filter_map(|(gauge, value)| Some((gauge, value?)))
    .collect();
    RadioReport {
        status,
        summary,
        attachment,
        readings,
    }
}
