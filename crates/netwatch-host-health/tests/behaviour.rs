//! How the kernel layer is graded from successive readings.

use netwatch_host_health::{KernelStatus, KernelWatch, Pressure, Reading};

fn reading(counters: &[(&str, u64)], cpu: f64) -> Reading {
    Reading {
        counters: counters.iter().map(|&(n, v)| (n.to_owned(), v)).collect(),
        pressure: Some(Pressure {
            cpu,
            memory: 0.0,
            io: 0.0,
        }),
    }
}

#[test]
fn first_reading_is_unknown() {
    let mut watch = KernelWatch::new();
    assert_eq!(
        watch.observe(reading(&[], 0.0)).status,
        KernelStatus::Unknown
    );
}

#[test]
fn unchanged_counters_are_healthy() {
    let mut watch = KernelWatch::new();
    watch.observe(reading(&[("Udp.RcvbufErrors", 7)], 0.0));
    assert_eq!(
        watch
            .observe(reading(&[("Udp.RcvbufErrors", 7)], 0.0))
            .status,
        KernelStatus::Healthy
    );
}

#[test]
fn a_growing_watched_counter_degrades_and_says_by_how_much() {
    let mut watch = KernelWatch::new();
    watch.observe(reading(&[("Udp.RcvbufErrors", 7)], 0.0));
    let report = watch.observe(reading(&[("Udp.RcvbufErrors", 10)], 0.0));
    assert_eq!(report.status, KernelStatus::Degraded);
    assert_eq!(report.increases.get("Udp.RcvbufErrors"), Some(&3));
}

#[test]
fn host_dropping_packets_is_failing() {
    let mut watch = KernelWatch::new();
    watch.observe(reading(&[("Softnet.Dropped", 0)], 0.0));
    assert_eq!(
        watch
            .observe(reading(&[("Softnet.Dropped", 1)], 0.0))
            .status,
        KernelStatus::Failing
    );
}

#[test]
fn a_counter_that_goes_backwards_is_a_reset_not_traffic() {
    let mut watch = KernelWatch::new();
    watch.observe(reading(&[("Ip.InDiscards", 500)], 0.0));
    let report = watch.observe(reading(&[("Ip.InDiscards", 2)], 0.0));
    assert_eq!(report.status, KernelStatus::Healthy);
    assert!(report.increases.is_empty());
    assert_eq!(report.resets, ["Ip.InDiscards"]);
}

#[test]
fn cpu_pressure_degrades() {
    let mut watch = KernelWatch::new();
    watch.observe(reading(&[], 0.0));
    assert_eq!(
        watch.observe(reading(&[], 35.0)).status,
        KernelStatus::Degraded
    );
}
