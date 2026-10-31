//! The composition root: reads the configuration, builds the adapters, connects them to
//! the domain state machines, and owns the async runtime (ADR-0012).
//!
//! This is the only crate allowed to use `anyhow` (ADR-0015).

mod clock;
mod persist;
mod probe;

use std::net::{IpAddr, Ipv4Addr, SocketAddr};
use std::num::NonZeroU64;
use std::path::PathBuf;

use anyhow::Context;
use netwatch_adapter_web::{EventPoint, LiveMessage};
use netwatch_history::{Measurement, Sample};
use netwatch_probing::{SequenceSpace, StreamConfig};
use netwatch_shared::uom::si::f64::Time;
use netwatch_shared::uom::si::time::{millisecond, second};
use netwatch_shared::{IdentityEpoch, Role, TargetId};
use netwatch_topology::{Binding, Command, Observation, Output, Topology, TopologyEvent};
use tokio::sync::{broadcast, mpsc, watch};

use crate::clock::Clock;

/// The default echo targets (ADR-0009). Configuration replaces these later.
const DEFAULT_ECHO_TARGETS: [(u32, IpAddr); 3] = [
    (1, IpAddr::V4(Ipv4Addr::new(1, 1, 1, 1))),
    (2, IpAddr::V4(Ipv4Addr::new(8, 8, 8, 8))),
    (3, IpAddr::V4(Ipv4Addr::new(9, 9, 9, 9))),
];

#[tokio::main(flavor = "current_thread")]
async fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env().unwrap_or_else(|_| "info".into()),
        )
        .init();
    let clock = Clock::start();
    let database =
        std::env::var_os("NETWATCH_DB").map_or_else(|| PathBuf::from("netwatch.db"), PathBuf::from);
    let resume_after = persist::last_epoch(&database)
        .with_context(|| format!("opening {}", database.display()))?;
    let writes = persist::spawn(database.clone(), clock)?;

    // The UI listens on localhost only, unless configured otherwise (ADR-0007).
    let listen: SocketAddr = std::env::var("NETWATCH_LISTEN")
        .unwrap_or_else(|_| "127.0.0.1:8732".to_owned())
        .parse()
        .context("NETWATCH_LISTEN must be an address:port")?;
    let (live, _) = broadcast::channel::<LiveMessage>(1024);
    tokio::spawn({
        let live = live.clone();
        async move {
            if let Err(error) = netwatch_adapter_web::serve(listen, database, live).await {
                tracing::error!(%error, "the UI server stopped");
            }
        }
    });

    let (observations_tx, mut observations) = mpsc::channel(256);
    let _watcher = netwatch_adapter_netlink::watch(observations_tx.clone())
        .context("starting the netlink topology watcher")?;
    // Epochs stay unique across restarts, so stored samples never share one.
    let mut topology = match resume_after.and_then(NonZeroU64::new) {
        Some(last) => Topology::resuming_after(IdentityEpoch::new(last)),
        None => Topology::new(),
    };

    let (pin_tx, pin) = watch::channel::<Option<Binding>>(None);
    let (samples_tx, mut samples) = mpsc::channel::<Sample>(1024);
    // Targets every 5 s with a 2 s timeout (ADR-0009). Valid for ICMP's sequence space.
    let echo = StreamConfig::new(
        Time::new::<second>(5.0),
        Time::new::<second>(2.0),
        SequenceSpace::ICMP,
    )
    .context("echo stream configuration")?;
    for (id, destination) in DEFAULT_ECHO_TARGETS {
        tokio::spawn(probe::run_target(
            Role::Target(TargetId::new(id)),
            destination,
            echo,
            pin.clone(),
            samples_tx.clone(),
            clock,
        ));
    }

    loop {
        tokio::select! {
            Some(observation) = observations.recv() => {
                for output in topology.observe(observation) {
                    handle(output, &observations_tx, &writes, &live, clock);
                }
                // Targets are pinned to the egress interface: the tunnel if there
                // is one, otherwise the NIC (ADR-0020, ADR-0022).
                let egress = topology
                    .binding(&Role::Tunnel)
                    .or_else(|| topology.binding(&Role::Nic))
                    .cloned();
                pin_tx.send_if_modified(|current| {
                    let changed = *current != egress;
                    *current = egress;
                    changed
                });
            }
            Some(sample) = samples.recv() => {
                log_sample(&sample);
                let _ = live.send(LiveMessage::from(&sample));
                let _ = writes.send(persist::Write::Sample(sample));
            }
            _ = tokio::signal::ctrl_c() => break,
        }
    }
    Ok(())
}

fn handle(
    output: Output,
    observations: &mpsc::Sender<Observation>,
    writes: &std::sync::mpsc::Sender<persist::Write>,
    live: &broadcast::Sender<LiveMessage>,
    clock: Clock,
) {
    match output {
        Output::Event(event) => {
            log_event(&event);
            let at = clock.wall_now();
            let _ = live.send(LiveMessage::Event {
                event: EventPoint::from(netwatch_adapter_sqlite::describe(&event, at)),
            });
            let _ = writes.send(persist::Write::Event(event, at));
        }
        Output::Command(Command::Resync { generation }) => {
            let observations = observations.clone();
            tokio::task::spawn_blocking(move || {
                // Retry until the dump succeeds. The aggregate stays desynchronized
                // (and keeps buffering) in the meantime.
                loop {
                    match netwatch_adapter_netlink::snapshot(generation) {
                        Ok(snapshot) => {
                            let _ =
                                observations.blocking_send(Observation::SnapshotReceived(snapshot));
                            return;
                        }
                        Err(error) => {
                            tracing::warn!(%error, "resync dump failed; retrying");
                            std::thread::sleep(std::time::Duration::from_secs(1));
                        }
                    }
                }
            });
        }
    }
}

fn log_sample(sample: &Sample) {
    match sample.measurement {
        Measurement::RoundTrip(rtt) => tracing::info!(
            role = ?sample.role,
            epoch = sample.epoch.get(),
            rtt_ms = format!("{:.3}", rtt.get::<millisecond>()),
            "sample"
        ),
        Measurement::Lost(loss) => {
            tracing::info!(role = ?sample.role, epoch = sample.epoch.get(), ?loss, "sample");
        }
    }
}

fn log_event(event: &TopologyEvent) {
    match event {
        TopologyEvent::EpochStarted {
            epoch,
            role,
            identity,
        } => {
            tracing::info!(epoch = epoch.get(), ?role, ?identity, "epoch started");
        }
        TopologyEvent::EpochEnded { epoch, role } => {
            tracing::info!(epoch = epoch.get(), ?role, "epoch ended");
        }
        TopologyEvent::Desynchronized { generation } => {
            tracing::warn!(generation = generation.get(), "topology desynchronized");
        }
        TopologyEvent::Resynchronized { generation } => {
            tracing::info!(generation = generation.get(), "topology resynchronized");
        }
    }
}
