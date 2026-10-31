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

#[test]
fn sums_stat_table_columns_over_every_cpu() {
    use netwatch_adapter_procfs::parse_stat_table;
    let text =
        "entries drop insert_failed\n00000001 00000002 00000003\n00000001 00000004 00000005\n";
    let counters = parse_stat_table(
        text,
        "Conntrack",
        &[("drop", "Drop"), ("insert_failed", "InsertFailed")],
    );
    assert_eq!(counters.get("Conntrack.Drop"), Some(&6));
    assert_eq!(counters.get("Conntrack.InsertFailed"), Some(&8));
    let real = parse_stat_table(
        include_str!("fixtures/stat_nf_conntrack"),
        "Conntrack",
        &[("drop", "Drop")],
    );
    assert!(real.contains_key("Conntrack.Drop"));
    let arp = parse_stat_table(
        include_str!("fixtures/stat_arp_cache"),
        "Arp",
        &[("table_fulls", "TableFulls")],
    );
    assert!(arp.contains_key("Arp.TableFulls"));
}

#[test]
fn reads_the_text_of_a_kmsg_record() {
    use netwatch_adapter_procfs::parse_kmsg_record;
    assert_eq!(
        parse_kmsg_record(
            "6,1234,5678901,-;e1000e 0000:00:1f.6 eth0: NIC Link is Down\n SUBSYSTEM=net\n"
        ),
        Some("e1000e 0000:00:1f.6 eth0: NIC Link is Down")
    );
    assert_eq!(parse_kmsg_record("no separator"), None);
}
