//! The parsers against files captured from a real host (Fedora 43, kernel 7.1).

use netwatch_adapter_procfs::{parse_paired_lines, parse_pressure_some_avg10, parse_softnet};

#[test]
fn reads_ip_counters_from_snmp() {
    let counters = parse_paired_lines(include_str!("fixtures/snmp"));
    assert_eq!(counters.get("Ip.InHdrErrors"), Some(&7919));
    assert_eq!(counters.get("Ip.OutNoRoutes"), Some(&192));
    assert!(counters.contains_key("Udp.RcvbufErrors"));
}

#[test]
fn reads_tcp_extensions_from_netstat() {
    let counters = parse_paired_lines(include_str!("fixtures/netstat"));
    assert!(counters.contains_key("TcpExt.ListenOverflows"));
}

#[test]
fn sums_softnet_columns_over_every_cpu() {
    let counters = parse_softnet("0000000a 00000001 00000002\n0000000b 00000003 00000004\n");
    assert_eq!(counters.get("Softnet.Dropped"), Some(&4));
    assert_eq!(counters.get("Softnet.TimeSqueeze"), Some(&6));
    assert!(parse_softnet(include_str!("fixtures/softnet_stat")).contains_key("Softnet.Dropped"));
}

#[test]
fn reads_pressure() {
    assert_eq!(
        parse_pressure_some_avg10(include_str!("fixtures/pressure_cpu")),
        Some(0.0)
    );
    assert_eq!(
        parse_pressure_some_avg10("some avg10=12.50 avg60=1\n"),
        Some(12.5)
    );
}

#[test]
fn malformed_input_is_skipped_not_zeroed() {
    assert!(parse_paired_lines("Ip: A B\nUdp: 1 2\n").is_empty());
    assert_eq!(parse_pressure_some_avg10("full avg10=1.0\n"), None);
    assert_eq!(parse_pressure_some_avg10("some avg10=NaN\n"), None);
}
