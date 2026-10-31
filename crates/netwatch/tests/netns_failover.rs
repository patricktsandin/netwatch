//! End to end: the daemon follows a default-route failover inside a throwaway network
//! namespace. Needs passwordless `sudo` for `ip`, so it is ignored by default:
//! `cargo test -p netwatch --test netns_failover -- --ignored`.

#![allow(clippy::unwrap_used)] // Tests may unwrap (ADR-0015), including in helpers.

use std::process::{Child, Command, Stdio};
use std::thread::sleep;
use std::time::Duration;

const NS: &str = "netwatch-failover-test";

fn ip(args: &str) {
    let status = Command::new("sudo")
        .args(["-n", "ip", "-n", NS])
        .args(args.split_whitespace())
        .status()
        .unwrap();
    assert!(status.success(), "ip -n {NS} {args}");
}

/// Deletes the namespace (and everything in it) even if the test fails.
struct Namespace(Option<Child>);

impl Drop for Namespace {
    fn drop(&mut self) {
        if let Some(mut daemon) = self.0.take() {
            let _ = Command::new("sudo")
                .args(["-n", "kill", &daemon.id().to_string()])
                .status();
            let _ = daemon.wait();
        }
        let _ = Command::new("sudo")
            .args(["-n", "ip", "netns", "del", NS])
            .status();
    }
}

#[test]
#[ignore = "needs passwordless sudo to create a network namespace"]
fn daemon_follows_default_route_failover_and_back() {
    let _ = Command::new("sudo")
        .args(["-n", "ip", "netns", "del", NS])
        .status();
    let status = Command::new("sudo")
        .args(["-n", "ip", "netns", "add", NS])
        .status()
        .unwrap();
    assert!(status.success());
    let mut namespace = Namespace(None);

    ip("link set lo up");
    ip("link add a0 type veth peer name a1");
    ip("link add b0 type veth peer name b1");
    for link in ["a0", "a1", "b0", "b1"] {
        ip(&format!("link set {link} up"));
    }
    ip("addr add 10.0.1.2/24 dev a0");
    ip("addr add 10.0.2.2/24 dev b0");
    ip("route add default via 10.0.1.1 dev a0 metric 100");
    ip("route add default via 10.0.2.1 dev b0 metric 600");

    // `sudo` doesn't pass the parent's environment, so set the log filter explicitly.
    let daemon = Command::new("sudo")
        .args([
            "-n",
            "ip",
            "netns",
            "exec",
            NS,
            "env",
            "RUST_LOG=info",
            "NO_COLOR=1",
        ])
        .arg(env!("CARGO_BIN_EXE_netwatch"))
        .stdout(Stdio::piped())
        .spawn()
        .unwrap();
    namespace.0 = Some(daemon);
    sleep(Duration::from_millis(500));

    ip("link set a0 down");
    sleep(Duration::from_millis(500));
    ip("link set a0 up");
    ip("route add default via 10.0.1.1 dev a0 metric 100");
    sleep(Duration::from_millis(500));

    let mut daemon = namespace.0.take().unwrap();
    let _ = Command::new("sudo")
        .args(["-n", "kill", &daemon.id().to_string()])
        .status();
    let mut log = String::new();
    std::io::Read::read_to_string(daemon.stdout.as_mut().unwrap(), &mut log).unwrap();
    let _ = daemon.wait();

    let nic_epochs: Vec<&str> = log
        .lines()
        .filter(|line| line.contains("epoch started") && line.contains("role=Nic"))
        .map(|line| {
            if line.contains("\"a0\"") {
                "a0"
            } else if line.contains("\"b0\"") {
                "b0"
            } else {
                "?"
            }
        })
        .collect();
    assert_eq!(nic_epochs, ["a0", "b0", "a0"], "daemon log:\n{log}");
}
