//! Throughput from a NIC's byte counters (ADR-0032).

use std::collections::BTreeMap;

use netwatch_host_health::Throughput;
use netwatch_shared::Instant;

fn counters(rx: u64, tx: u64) -> BTreeMap<String, u64> {
    BTreeMap::from([
        ("Link.RxBytes".to_owned(), rx),
        ("Link.TxBytes".to_owned(), tx),
    ])
}

fn at(seconds: f64) -> Instant {
    Instant::from_nanos((seconds * 1e9) as u64)
}

#[test]
fn the_first_reading_has_nothing_to_compare_with() {
    assert_eq!(
        Throughput::default().observe(&counters(10, 10), at(1.0)),
        None
    );
}

#[test]
fn throughput_is_bits_per_second_over_the_interval_between_readings() {
    let mut throughput = Throughput::default();
    throughput.observe(&counters(1_000, 500), at(1.0));
    assert_eq!(
        throughput.observe(&counters(3_500, 500), at(1.5)),
        Some((40_000.0, 0.0))
    );
}

#[test]
fn a_counter_reset_is_a_gap_never_negative_traffic() {
    let mut throughput = Throughput::default();
    throughput.observe(&counters(10_000, 10_000), at(1.0));
    assert_eq!(throughput.observe(&counters(5, 20_000), at(2.0)), None);
    // Afterwards, the reset value is the new baseline.
    assert_eq!(
        throughput.observe(&counters(130, 20_000), at(3.0)),
        Some((1_000.0, 0.0))
    );
}

#[test]
fn missing_counters_or_a_stalled_clock_give_no_reading() {
    let mut throughput = Throughput::default();
    throughput.observe(&counters(0, 0), at(1.0));
    assert_eq!(throughput.observe(&counters(10, 10), at(1.0)), None);
    assert_eq!(throughput.observe(&BTreeMap::new(), at(2.0)), None);
}
