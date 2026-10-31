//! Anti-corruption layer for the kernel's text interfaces in `/proc` (ADR-0008).
//!
//! The parsers are pure functions of the file contents, so they are tested against
//! real files captured from a host (`tests/fixtures`). They accept unknown fields,
//! because kernel versions add new ones, and they reject nothing silently: a line
//! that doesn't parse is skipped, never turned into a zero.
//!
//! Sources so far: `/proc/net/snmp`, `/proc/net/netstat`, `/proc/net/softnet_stat`,
//! `/proc/pressure/{cpu,memory,io}`, the connection-tracking table
//! (`/proc/net/stat/nf_conntrack` and its fill level), and the neighbor tables
//! (`/proc/net/stat/{arp_cache,ndisc_cache}`). A source that can't be read (for example PSI on a
//! kernel without `CONFIG_PSI`) is left out of the reading, never reported as zero.

use std::collections::BTreeMap;
use std::fs;

use netwatch_host_health::{Pressure, Reading};

/// Reads every source now. Never fails: an unreadable source is just absent.
pub fn read() -> Reading {
    let mut counters = BTreeMap::new();
    for path in ["/proc/net/snmp", "/proc/net/netstat"] {
        if let Ok(text) = fs::read_to_string(path) {
            counters.extend(parse_paired_lines(&text));
        }
    }
    if let Ok(text) = fs::read_to_string("/proc/net/softnet_stat") {
        counters.extend(parse_softnet(&text));
    }
    let stat_tables = [
        (
            "/proc/net/stat/nf_conntrack",
            "Conntrack",
            &[
                ("drop", "Drop"),
                ("early_drop", "EarlyDrop"),
                ("insert_failed", "InsertFailed"),
            ][..],
        ),
        (
            "/proc/net/stat/arp_cache",
            "Arp",
            &[
                ("table_fulls", "TableFulls"),
                ("unresolved_discards", "UnresolvedDiscards"),
            ][..],
        ),
        (
            "/proc/net/stat/ndisc_cache",
            "Ndisc",
            &[
                ("table_fulls", "TableFulls"),
                ("unresolved_discards", "UnresolvedDiscards"),
            ][..],
        ),
    ];
    for (path, group, columns) in stat_tables {
        if let Ok(text) = fs::read_to_string(path) {
            counters.extend(parse_stat_table(&text, group, columns));
        }
    }
    let conntrack_fill = match (
        read_number("/proc/sys/net/netfilter/nf_conntrack_count"),
        read_number("/proc/sys/net/netfilter/nf_conntrack_max"),
    ) {
        (Some(count), Some(max)) if max > 0 => Some(100.0 * count as f64 / max as f64),
        _ => None,
    };
    let avg10 = |resource: &str| {
        fs::read_to_string(format!("/proc/pressure/{resource}"))
            .ok()
            .and_then(|text| parse_pressure_some_avg10(&text))
    };
    let pressure = match (avg10("cpu"), avg10("memory"), avg10("io")) {
        (Some(cpu), Some(memory), Some(io)) => Some(Pressure { cpu, memory, io }),
        _ => None,
    };
    Reading {
        counters,
        pressure,
        conntrack_fill,
    }
}

fn read_number(path: &str) -> Option<u64> {
    fs::read_to_string(path).ok()?.trim().parse().ok()
}

/// `/proc/net/stat/*`: a header line naming the columns, then one line of hexadecimal
/// values per CPU. Returns `Group.Name` → the total over every CPU, for each wanted
/// `(column, Name)`.
pub fn parse_stat_table(text: &str, group: &str, wanted: &[(&str, &str)]) -> BTreeMap<String, u64> {
    let mut lines = text.lines();
    let Some(header) = lines.next() else {
        return BTreeMap::new();
    };
    let columns: Vec<&str> = header.split_whitespace().collect();
    let mut totals = BTreeMap::new();
    for line in lines {
        let values: Vec<u64> = line
            .split_whitespace()
            .map_while(|v| u64::from_str_radix(v, 16).ok())
            .collect();
        for &(column, name) in wanted {
            if let Some(&value) = columns
                .iter()
                .position(|c| *c == column)
                .and_then(|i| values.get(i))
            {
                let total: &mut u64 = totals.entry(format!("{group}.{name}")).or_default();
                *total = total.saturating_add(value);
            }
        }
    }
    totals
}

/// `/proc/net/snmp` and `/proc/net/netstat`: pairs of lines, a header naming the
/// fields and a line of values, both prefixed with the group (`Ip:`, `TcpExt:`, …).
/// Returns `Group.Field` → value.
pub fn parse_paired_lines(text: &str) -> BTreeMap<String, u64> {
    let mut counters = BTreeMap::new();
    let mut lines = text.lines();
    while let (Some(header), Some(values)) = (lines.next(), lines.next()) {
        let (Some((group, names)), Some((value_group, numbers))) =
            (header.split_once(':'), values.split_once(':'))
        else {
            continue;
        };
        if group != value_group {
            continue;
        }
        for (name, number) in names.split_whitespace().zip(numbers.split_whitespace()) {
            // Some fields (Icmp's InType/OutType maps, MaxConn) can be negative; they
            // aren't counters, so skipping them is right.
            if let Ok(value) = number.parse::<u64>() {
                counters.insert(format!("{group}.{name}"), value);
            }
        }
    }
    counters
}

/// `/proc/net/softnet_stat`: one line per CPU of hexadecimal columns. Column 1 is
/// packets dropped because the backlog was full, column 2 is the number of times the
/// budget ran out ("time squeeze"). Returns the totals over all CPUs.
pub fn parse_softnet(text: &str) -> BTreeMap<String, u64> {
    let mut dropped = 0u64;
    let mut squeezed = 0u64;
    for line in text.lines() {
        let columns: Vec<u64> = line
            .split_whitespace()
            .map_while(|c| u64::from_str_radix(c, 16).ok())
            .collect();
        if let (Some(&d), Some(&s)) = (columns.get(1), columns.get(2)) {
            dropped = dropped.saturating_add(d);
            squeezed = squeezed.saturating_add(s);
        }
    }
    BTreeMap::from([
        ("Softnet.Dropped".to_owned(), dropped),
        ("Softnet.TimeSqueeze".to_owned(), squeezed),
    ])
}

/// `/proc/pressure/*`: the `avg10` of the `some` line, in percent.
pub fn parse_pressure_some_avg10(text: &str) -> Option<f64> {
    let line = text.lines().find(|l| l.starts_with("some "))?;
    let field = line
        .split_whitespace()
        .find_map(|f| f.strip_prefix("avg10="))?;
    field
        .parse()
        .ok()
        .filter(|v: &f64| v.is_finite() && *v >= 0.0)
}
