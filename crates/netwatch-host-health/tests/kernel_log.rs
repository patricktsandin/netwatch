//! Kernel log lines, as real drivers write them.

use netwatch_host_health::{KernelLogKind, classify};

#[test]
fn classifies_real_driver_messages_and_finds_their_interface() {
    let cases = [
        // Captured from the development host's own kernel log.
        (
            "r8152 2-1.4.2.4:1.0 eth0: carrier on",
            KernelLogKind::LinkUp,
            Some("eth0"),
        ),
        (
            "e1000e 0000:00:1f.6 eth0: NIC Link is Up 1000 Mbps Full Duplex, Flow Control: None",
            KernelLogKind::LinkUp,
            Some("eth0"),
        ),
        (
            "e1000e 0000:00:1f.6 eth0: NIC Link is Down",
            KernelLogKind::LinkDown,
            Some("eth0"),
        ),
        (
            "NETDEV WATCHDOG: eth0 (e1000e): transmit queue 0 timed out 5304 ms",
            KernelLogKind::TransmitTimeout,
            Some("eth0"),
        ),
        // A PCI address isn't an interface name, so this one names none.
        (
            "iwlwifi 0000:00:14.3: Microcode SW error detected. Restarting 0x0. firmware error",
            KernelLogKind::FirmwareError,
            None,
        ),
        (
            "IPv6: ADDRCONF(NETDEV_CHANGE): wlp0s20f3: link becomes ready",
            KernelLogKind::LinkUp,
            Some("wlp0s20f3"),
        ),
        (
            "igb 0000:03:00.0 enp3s0: Reset adapter",
            KernelLogKind::DeviceReset,
            Some("enp3s0"),
        ),
    ];
    for (message, kind, interface) in cases {
        let line = classify(message).unwrap_or_else(|| panic!("unclassified: {message}"));
        assert_eq!(line.kind, kind, "{message}");
        assert_eq!(line.interface.as_deref(), interface, "{message}");
    }
}

#[test]
fn ignores_everything_else() {
    for message in [
        "usb 3-2: new high-speed USB device number 4",
        "EXT4-fs (dm-0): mounted filesystem",
        "",
    ] {
        assert_eq!(classify(message), None, "{message}");
    }
}
