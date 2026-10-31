//! Anti-corruption layer between the kernel's netlink families and the domain.
//!
//! This crate subscribes to rtnetlink multicast groups (link, IPv4/IPv6 addresses and
//! routes), asks `RTM_GETROUTE` for the egress path, and turns the answer into
//! Topology *observations* (ADR-0003, ADR-0022).
//!
//! Nothing netlink-shaped leaves this crate. If a domain type ever needs a netlink
//! attribute, the translation belongs here.
//!
//! # How it keeps the view truthful
//!
//! - **Every notification triggers a fresh resolution.** The content of a multicast
//!   message is never trusted, only the fact that it arrived. So what gets delivered
//!   always reflects the kernel's state at the moment of resolution, which is never
//!   older than the notification itself.
//! - **Subscribe first, then resolve once.** Nothing that changes between startup and
//!   the first notification can be missed.
//! - **`ENOBUFS` is never dropped.** The socket is drained until `EAGAIN`, and only then
//!   is [`Observation::NotificationsLost`] sent, exactly as ADR-0021 requires.

mod resolve;

use std::io;
use std::thread;

use netlink_sys::{Socket, SocketAddr, protocols::NETLINK_ROUTE};
use netwatch_topology::{Observation, ResyncGeneration, Snapshot, egress};
use tokio::sync::mpsc;

pub use resolve::{link_counters, resolve_egress};

// Legacy multicast group bitmasks from <linux/rtnetlink.h>.
const RTMGRP_LINK: u32 = 0x1;
const RTMGRP_IPV4_IFADDR: u32 = 0x10;
const RTMGRP_IPV4_ROUTE: u32 = 0x40;
const RTMGRP_IPV6_IFADDR: u32 = 0x100;
const RTMGRP_IPV6_ROUTE: u32 = 0x400;

/// Starts watching the kernel's topology on a dedicated thread, sending
/// observations to `observations` until the receiver is dropped.
///
/// The first batch describes the current state. Later batches follow every link,
/// address, and route change.
pub fn watch(observations: mpsc::Sender<Observation>) -> io::Result<thread::JoinHandle<()>> {
    let mut monitor = Socket::new(NETLINK_ROUTE)?;
    monitor.bind(&SocketAddr::new(
        0,
        RTMGRP_LINK
            | RTMGRP_IPV4_IFADDR
            | RTMGRP_IPV4_ROUTE
            | RTMGRP_IPV6_IFADDR
            | RTMGRP_IPV6_ROUTE,
    ))?;
    thread::Builder::new()
        .name("netlink-topology".to_owned())
        .spawn(move || {
            if let Err(error) = run(&monitor, &observations) {
                tracing::error!(%error, "topology watcher stopped");
            }
        })
}

/// Answers a resync: dumps the current egress path as the snapshot for
/// `generation`. Blocking; call it from a blocking context.
pub fn snapshot(generation: ResyncGeneration) -> io::Result<Snapshot> {
    Ok(egress::snapshot(generation, resolve_egress()?.as_ref()))
}

fn run(monitor: &Socket, observations: &mpsc::Sender<Observation>) -> io::Result<()> {
    publish(observations)?;
    loop {
        match monitor.recv_from_full() {
            Ok(_) => publish(observations)?,
            Err(error) if error.raw_os_error() == Some(libc::ENOBUFS) => {
                drain(monitor)?;
                tracing::warn!("netlink notifications lost; requesting a resync");
                send(observations, Observation::NotificationsLost)?;
            }
            Err(error) if error.kind() == io::ErrorKind::Interrupted => {}
            Err(error) => return Err(error),
        }
    }
}

/// Resolves the egress path now and sends the global roles' complete state.
fn publish(observations: &mpsc::Sender<Observation>) -> io::Result<()> {
    let path = resolve_egress()?;
    for observation in egress::observations(path.as_ref()) {
        send(observations, observation)?;
    }
    Ok(())
}

/// Discards everything queued on the socket. Every discarded message was
/// generated before the resync that follows is requested, so the snapshot
/// covers it (ADR-0021).
fn drain(monitor: &Socket) -> io::Result<()> {
    monitor.set_non_blocking(true)?;
    let drained = loop {
        match monitor.recv_from_full() {
            Ok(_) => {}
            Err(error) if error.kind() == io::ErrorKind::WouldBlock => break Ok(()),
            Err(error) if error.raw_os_error() == Some(libc::ENOBUFS) => {}
            Err(error) if error.kind() == io::ErrorKind::Interrupted => {}
            Err(error) => break Err(error),
        }
    };
    monitor.set_non_blocking(false)?;
    drained
}

fn send(observations: &mpsc::Sender<Observation>, observation: Observation) -> io::Result<()> {
    observations
        .blocking_send(observation)
        .map_err(|_| io::Error::new(io::ErrorKind::BrokenPipe, "topology consumer has stopped"))
}
