//! The drop-reason program, loaded for real. Needs CAP_BPF and CAP_PERFMON, so it is
//! ignored by default, like the netns tests: run it with
//! `CARGO_TARGET_X86_64_UNKNOWN_LINUX_GNU_RUNNER='sudo -E' cargo test -p
//! netwatch-adapter-ebpf --test privileged -- --ignored`, which builds as you and runs
//! only the test binary as root, so that nothing in `target/` becomes root's.

#![allow(clippy::unwrap_used)] // Tests may unwrap (ADR-0015), including in helpers.

use std::net::UdpSocket;

use netwatch_adapter_ebpf::DropReasons;

#[test]
#[ignore = "needs CAP_BPF and CAP_PERFMON"]
fn a_datagram_to_a_closed_port_is_counted_as_no_socket() {
    let drops = DropReasons::start().unwrap();
    let before = drops.read().get("Drop.NO_SOCKET").copied().unwrap_or(0);
    // A port nothing listens on: bind one, note it, and close it.
    let port = UdpSocket::bind("127.0.0.1:0")
        .unwrap()
        .local_addr()
        .unwrap()
        .port();
    let sender = UdpSocket::bind("127.0.0.1:0").unwrap();
    for _ in 0..5 {
        sender.send_to(b"netwatch", ("127.0.0.1", port)).unwrap();
    }
    std::thread::sleep(std::time::Duration::from_millis(200));
    let after = drops.read().get("Drop.NO_SOCKET").copied().unwrap_or(0);
    assert!(
        after >= before + 5,
        "NO_SOCKET went from {before} to {after}"
    );
}
