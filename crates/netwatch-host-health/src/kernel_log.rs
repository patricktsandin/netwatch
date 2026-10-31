//! Network-relevant kernel log lines (ADR-0008, glossary: *kernel log line*).

/// What a network-relevant kernel log line reports.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum KernelLogKind {
    /// A link came up.
    LinkUp,
    /// A link went down.
    LinkDown,
    /// The transmit watchdog fired: the NIC stopped sending (`NETDEV WATCHDOG`).
    TransmitTimeout,
    /// The driver reset the device.
    DeviceReset,
    /// The device's firmware reported an error or crashed.
    FirmwareError,
}

/// A classified kernel log line.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct KernelLogLine {
    /// What it reports.
    pub kind: KernelLogKind,
    /// The interface it names, if it names one the usual way (`<driver> <bus id> <ifname>:`
    /// or `<ifname>:`).
    pub interface: Option<String>,
    /// The message itself, for people.
    pub message: String,
}

/// The patterns, checked in order, so that the more specific ones win. Matching is
/// case-insensitive. The list needs maintenance as drivers change their messages.
const PATTERNS: &[(&str, KernelLogKind)] = &[
    ("netdev watchdog", KernelLogKind::TransmitTimeout),
    ("tx timeout", KernelLogKind::TransmitTimeout),
    ("transmit queue", KernelLogKind::TransmitTimeout),
    ("firmware crash", KernelLogKind::FirmwareError),
    ("firmware error", KernelLogKind::FirmwareError),
    ("fw error", KernelLogKind::FirmwareError),
    ("reset adapter", KernelLogKind::DeviceReset),
    ("resetting", KernelLogKind::DeviceReset),
    ("link is up", KernelLogKind::LinkUp),
    ("link becomes ready", KernelLogKind::LinkUp),
    ("link is down", KernelLogKind::LinkDown),
    // USB Ethernet drivers such as r8152 report link changes as carrier changes.
    ("carrier on", KernelLogKind::LinkUp),
    ("carrier off", KernelLogKind::LinkDown),
];

/// Classifies one kernel log message. `None` for anything not network-relevant.
pub fn classify(message: &str) -> Option<KernelLogLine> {
    let lower = message.to_ascii_lowercase();
    let (at, kind) = PATTERNS
        .iter()
        .find_map(|&(pattern, kind)| lower.find(pattern).map(|at| (at, kind)))?;
    Some(KernelLogLine {
        kind,
        interface: interface_in(message, at),
        message: message.trim().to_owned(),
    })
}

/// The interface a message names: the word just before the last `": "` ahead of the
/// matched phrase. That is how `netdev_*` messages are prefixed
/// (`e1000e 0000:00:1f.6 eth0: NIC Link is Up`), and it also skips subsystem prefixes
/// (`IPv6: ADDRCONF(NETDEV_CHANGE): wlp0s20f3: link becomes ready`).
/// `NETDEV WATCHDOG: eth0 (e1000e): ...` names it right after its own prefix instead.
fn interface_in(message: &str, matched_at: usize) -> Option<String> {
    let candidate = if let Some(rest) = message.strip_prefix("NETDEV WATCHDOG: ") {
        rest.split_whitespace().next()?
    } else {
        let (prefix, _) = message.get(..matched_at)?.rsplit_once(": ")?;
        prefix.split_whitespace().last()?
    };
    let name = candidate.trim_end_matches(':');
    let plausible = !name.is_empty()
        && name.len() <= 15
        && name
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b"._-@".contains(&b));
    plausible.then(|| name.to_owned())
}
