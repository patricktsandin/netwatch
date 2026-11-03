//! End to end: the daemon follows a default-route failover inside a throwaway network
//! namespace. Needs passwordless `sudo` for `ip`, so it is ignored by default:
//! `cargo test -p netwatch --test netns_failover -- --ignored`.

#![allow(clippy::unwrap_used)] // Tests may unwrap (ADR-0015), including in helpers.

use std::process::{Child, Command, Stdio};
use std::thread::sleep;
use std::time::Duration;

fn ip(ns: &str, args: &str) {
    let status = Command::new("sudo")
        .args(["-n", "ip", "-n", ns])
        .args(args.split_whitespace())
        .status()
        .unwrap();
    assert!(status.success(), "ip -n {ns} {args}");
}

/// Deletes the namespace (and everything in it) even if the test fails.
struct Namespace(&'static str, Option<Child>);

impl Drop for Namespace {
    fn drop(&mut self) {
        if let Some(mut daemon) = self.1.take() {
            let _ = Command::new("sudo")
                .args(["-n", "kill", &daemon.id().to_string()])
                .status();
            let _ = daemon.wait();
        }
        let _ = Command::new("sudo")
            .args(["-n", "ip", "netns", "del", self.0])
            .status();
    }
}

/// Runs the daemon in a fresh namespace `ns` prepared by `setup`, applies `changes`, and
/// returns the NIC epochs it started, in order, by interface name.
fn nic_epochs(ns: &'static str, setup: &[&str], changes: &[&str]) -> Vec<String> {
    let _ = Command::new("sudo")
        .args(["-n", "ip", "netns", "del", ns])
        .status();
    let status = Command::new("sudo")
        .args(["-n", "ip", "netns", "add", ns])
        .status()
        .unwrap();
    assert!(status.success());
    let mut namespace = Namespace(ns, None);
    for command in setup {
        ip(ns, command);
    }

    // `sudo` doesn't pass the parent's environment, so set it explicitly. The daemon
    // runs as root, which would otherwise find the installed service's configuration
    // and write into its database: it gets the example configuration and a database
    // of its own instead.
    let config = concat!(env!("CARGO_MANIFEST_DIR"), "/../../deploy/netwatch.toml");
    let database = std::env::temp_dir().join(format!("{ns}-{}.db", std::process::id()));
    let daemon = Command::new("sudo")
        .args([
            "-n",
            "ip",
            "netns",
            "exec",
            ns,
            "env",
            "RUST_LOG=info",
            "NO_COLOR=1",
        ])
        .arg(format!("NETWATCH_CONFIG={config}"))
        .arg(format!("NETWATCH_DB={}", database.display()))
        .arg(env!("CARGO_BIN_EXE_netwatch"))
        .stdout(Stdio::piped())
        .spawn()
        .unwrap();
    namespace.1 = Some(daemon);
    sleep(Duration::from_millis(500));
    for command in changes {
        ip(ns, command);
        sleep(Duration::from_millis(500));
    }

    let mut daemon = namespace.1.take().unwrap();
    let _ = Command::new("sudo")
        .args(["-n", "kill", &daemon.id().to_string()])
        .status();
    let mut log = String::new();
    std::io::Read::read_to_string(daemon.stdout.as_mut().unwrap(), &mut log).unwrap();
    let _ = daemon.wait();
    let _ = Command::new("sudo")
        .args(["-n", "rm", "-f"])
        .args(["", "-wal", "-shm"].map(|suffix| format!("{}{suffix}", database.display())))
        .status();
    log.lines()
        .filter(|line| line.contains("epoch started") && line.contains("role=Nic"))
        .map(|line| {
            ["a0", "b0"]
                .into_iter()
                .find(|name| line.contains(&format!("\"{name}\"")))
                .unwrap_or("?")
                .to_owned()
        })
        .collect()
}

const TWO_UPLINKS: &[&str] = &[
    "link set lo up",
    "link add a0 type veth peer name a1",
    "link add b0 type veth peer name b1",
    "link set a0 up",
    "link set a1 up",
    "link set b0 up",
    "link set b1 up",
    "addr add 10.0.1.2/24 dev a0",
    "addr add 10.0.2.2/24 dev b0",
    "route add default via 10.0.1.1 dev a0 metric 100",
    "route add default via 10.0.2.1 dev b0 metric 600",
];

#[test]
#[ignore = "needs passwordless sudo to create a network namespace"]
fn daemon_follows_default_route_failover_and_back() {
    let epochs = nic_epochs(
        "netwatch-failover-test",
        TWO_UPLINKS,
        &[
            "link set a0 down",
            "link set a0 up",
            "route add default via 10.0.1.1 dev a0 metric 100",
        ],
    );
    assert_eq!(epochs, ["a0", "b0", "a0"]);
}

#[test]
#[ignore = "needs passwordless sudo to create a network namespace"]
fn daemon_keeps_following_topology_through_a_kill_switch_route() {
    // A `blackhole` or `prohibit` default route makes the route lookup fail. That is
    // no egress path, and the watcher must still see the uplink come back after it.
    let epochs = nic_epochs(
        "netwatch-kill-switch-test",
        TWO_UPLINKS,
        &[
            "route add blackhole default metric 50",
            "route del blackhole default metric 50",
            "route add prohibit default metric 50",
            "route del prohibit default metric 50",
        ],
    );
    assert_eq!(epochs, ["a0", "a0", "a0"]);
}
