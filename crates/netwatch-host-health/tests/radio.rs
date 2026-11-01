//! Grading the radio roles from what their devices report (ADR-0036).

use netwatch_host_health::LayerStatus;
use netwatch_host_health::radio::{
    Association, CellularLink, ModemState, RadioAccess, WifiLink, cellular_report, wifi_report,
};
use netwatch_shared::Gauge;

fn associated(signal_dbm: i8) -> WifiLink {
    WifiLink {
        associated: Some(Association {
            bssid: [0xaa, 0xbb, 0xcc, 0, 1, 2],
            signal_dbm: Some(signal_dbm),
            tx_bitrate_bps: Some(866.7e6),
            rx_bitrate_bps: None,
        }),
    }
}

fn modem(state: ModemState, quality: u8) -> CellularLink {
    CellularLink {
        state,
        quality_percent: Some(quality),
        access: Some(RadioAccess::Nr5g),
        operator: Some("Telia".to_owned()),
        rsrp_dbm: Some(-95.0),
        rsrq_db: None,
        snr_db: Some(12.5),
    }
}

#[test]
fn wifi_is_graded_by_signal_once_associated_and_unknown_while_standing_by() {
    assert_eq!(wifi_report(&associated(-55)).status, LayerStatus::Healthy);
    assert_eq!(wifi_report(&associated(-67)).status, LayerStatus::Healthy);
    assert_eq!(wifi_report(&associated(-72)).status, LayerStatus::Degraded);
    assert_eq!(wifi_report(&associated(-81)).status, LayerStatus::Failing);
    let standing_by = wifi_report(&WifiLink { associated: None });
    assert_eq!(standing_by.status, LayerStatus::Unknown);
    assert_eq!(standing_by.summary, "not associated");
    assert!(standing_by.readings.is_empty());
}

#[test]
fn an_associated_wifi_reports_its_access_point_and_the_gauges_it_has() {
    let report = wifi_report(&associated(-60));
    assert_eq!(report.attachment.as_deref(), Some("aa:bb:cc:00:01:02"));
    assert_eq!(
        report.summary,
        "associated with aa:bb:cc:00:01:02 · -60 dBm"
    );
    assert_eq!(
        report.readings,
        vec![(Gauge::WifiSignal, -60.0), (Gauge::WifiTxBitrate, 866.7e6)]
    );
}

#[test]
fn cellular_is_graded_by_registration_then_quality() {
    assert_eq!(
        cellular_report(&modem(ModemState::Registered, 30)).status,
        LayerStatus::Healthy
    );
    assert_eq!(
        cellular_report(&modem(ModemState::Connected, 29)).status,
        LayerStatus::Degraded
    );
    assert_eq!(
        cellular_report(&modem(ModemState::Registered, 9)).status,
        LayerStatus::Failing
    );
    assert_eq!(
        cellular_report(&modem(ModemState::Searching, 50)).status,
        LayerStatus::Failing
    );
    assert_eq!(
        cellular_report(&modem(ModemState::Failed, 0)).status,
        LayerStatus::Failing
    );
    let disabled = cellular_report(&modem(ModemState::Disabled, 0));
    assert_eq!(disabled.status, LayerStatus::Unknown);
    assert_eq!(disabled.summary, "modem disabled");
    assert!(disabled.readings.is_empty());
}

#[test]
fn a_registered_modem_reports_operator_access_and_signal() {
    let report = cellular_report(&modem(ModemState::Registered, 64));
    assert_eq!(report.attachment.as_deref(), Some("Telia · 5G NR"));
    assert_eq!(report.summary, "registered on Telia · 5G NR · 64 %");
    assert_eq!(
        report.readings,
        vec![
            (Gauge::CellularQuality, 64.0),
            (Gauge::CellularRsrp, -95.0),
            (Gauge::CellularSnr, 12.5)
        ]
    );
}
