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
//! (`/proc/net/stat/{arp_cache,ndisc_cache}`), the clock's discipline (`adjtimex`), and
//! network sysctls (for config drift). A source that can't be read (for example PSI on a
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
        clock: clock_state().map(|c| (c.synchronized, c.estimated_error_us)),
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

/// Starts tailing the kernel log (`/dev/kmsg`) from its current end, sending each
/// message's text. Fails if the log can't be opened, which on Fedora means the
/// process lacks `CAP_SYSLOG` (`kernel.dmesg_restrict=1`). The caller reports that
/// as a disabled source (ADR-0007).
pub fn watch_kernel_log(
    messages: tokio::sync::mpsc::Sender<String>,
) -> std::io::Result<std::thread::JoinHandle<()>> {
    use std::io::{Read, Seek, SeekFrom};
    let mut log = fs::File::open("/dev/kmsg")?;
    // Only new messages: history before the daemon started isn't an event now.
    log.seek(SeekFrom::End(0))?;
    std::thread::Builder::new()
        .name("kernel-log".to_owned())
        .spawn(move || {
            // Each read returns exactly one record.
            let mut record = vec![0u8; 8192];
            loop {
                match log.read(&mut record) {
                    Ok(0) => return,
                    Ok(len) => {
                        let text = String::from_utf8_lossy(&record[..len]);
                        if let Some(message) = parse_kmsg_record(&text)
                            && messages.blocking_send(message.to_owned()).is_err()
                        {
                            return;
                        }
                    }
                    // EPIPE: records were overwritten before we read them; carry on.
                    Err(error) if error.raw_os_error() == Some(32) => {}
                    Err(error) if error.kind() == std::io::ErrorKind::Interrupted => {}
                    Err(_) => return,
                }
            }
        })
}

/// The message text of one `/dev/kmsg` record: `prio,seq,usec,flags;text`, then
/// optional continuation lines (` KEY=value`), which are dropped.
pub fn parse_kmsg_record(record: &str) -> Option<&str> {
    let (_, rest) = record.split_once(';')?;
    Some(rest.lines().next().unwrap_or("").trim_end())
}

/// The kernel clock's discipline, from `adjtimex` in read-only mode (ADR-0008).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ClockState {
    /// Whether an NTP daemon has the clock synchronized (`TIME_ERROR` is not set).
    pub synchronized: bool,
    /// The kernel's estimate of the clock's error, in µs.
    pub estimated_error_us: i64,
}

/// Reads the clock's discipline. `adjtimex` with no mode bits set only reads, and
/// needs no privileges.
pub fn clock_state() -> Option<ClockState> {
    // SAFETY: `timex` is plain old data, so all-zero is a valid value, and with
    // `modes == 0` the kernel only writes into the struct we pass.
    #[allow(unsafe_code)]
    let (state, timex) = unsafe {
        let mut timex: libc::timex = std::mem::zeroed();
        (libc::adjtimex(&mut timex), timex)
    };
    (state >= 0).then_some(ClockState {
        synchronized: state != libc::TIME_ERROR,
        estimated_error_us: timex.esterror,
    })
}

/// The time since boot, suspend included (`CLOCK_BOOTTIME`). Unlike the monotonic
/// clock, it keeps counting while the host sleeps, so the wall clock advancing over
/// a suspend is not a jump against it.
pub fn since_boot() -> Option<std::time::Duration> {
    let mut now = libc::timespec {
        tv_sec: 0,
        tv_nsec: 0,
    };
    // SAFETY: `now` is a live, writable `timespec`, which is all `clock_gettime`
    // writes into.
    #[allow(unsafe_code)]
    let result = unsafe { libc::clock_gettime(libc::CLOCK_BOOTTIME, &raw mut now) };
    (result == 0).then(|| {
        std::time::Duration::new(
            u64::try_from(now.tv_sec).unwrap_or(0),
            u32::try_from(now.tv_nsec).unwrap_or(0),
        )
    })
}

/// Network-relevant sysctls whose changes are recorded as config drift (ADR-0008).
pub const WATCHED_SYSCTLS: &[&str] = &[
    "net/ipv4/ip_forward",
    "net/ipv4/conf/all/rp_filter",
    "net/ipv4/ping_group_range",
    "net/ipv4/tcp_congestion_control",
    "net/ipv4/icmp_ratelimit",
    "net/ipv6/conf/all/disable_ipv6",
    "net/netfilter/nf_conntrack_max",
    "net/core/rmem_max",
    "net/core/netdev_max_backlog",
];

/// The current value of each watched sysctl that exists, by its dotted name.
pub fn sysctls() -> BTreeMap<String, String> {
    WATCHED_SYSCTLS
        .iter()
        .filter_map(|path| {
            let value = fs::read_to_string(format!("/proc/sys/{path}")).ok()?;
            Some((
                path.replace('/', "."),
                value.split_whitespace().collect::<Vec<_>>().join(" "),
            ))
        })
        .collect()
}

/// The link facts of interface `name`, from `/sys/class/net/<name>` (ADR-0039). A
/// file that can't be read or parsed (a link that is down has no speed) is `None`.
pub fn link_facts(name: &str) -> netwatch_host_health::facts::LinkFacts {
    let dir = std::path::Path::new("/sys/class/net").join(name);
    let text = |file: &str| {
        std::fs::read_to_string(dir.join(file))
            .ok()
            .map(|t| t.trim().to_owned())
    };
    netwatch_host_health::facts::LinkFacts {
        // The kernel writes -1 for an unknown speed, which doesn't parse as u32.
        speed_mbps: text("speed").and_then(|s| s.parse().ok()),
        full_duplex: text("duplex").and_then(|d| match d.as_str() {
            "full" => Some(true),
            "half" => Some(false),
            _ => None,
        }),
        driver: std::fs::read_link(dir.join("device/driver"))
            .ok()
            .and_then(|p| p.file_name().map(|n| n.to_string_lossy().into_owned())),
        mtu: text("mtu").and_then(|m| m.parse().ok()),
        carrier_changes: text("carrier_changes").and_then(|c| c.parse().ok()),
    }
}
