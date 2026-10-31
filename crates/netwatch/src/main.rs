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
use netwatch_adapter_web::{EventPoint, KernelReportView, LiveMessage, TargetView};
use netwatch_history::{Measurement, Sample};
use netwatch_host_health::KernelWatch;
use netwatch_probing::{SequenceSpace, StreamConfig};
use netwatch_shared::uom::si::f64::Time;
use netwatch_shared::uom::si::time::{millisecond, second};
use netwatch_shared::{
    HopTtl, Identity, IdentityEpoch, InterfaceIndex, InterfaceName, Role, TargetId,
};
use netwatch_topology::{Binding, Command, Observation, Output, Topology, TopologyEvent};
use tokio::sync::{broadcast, mpsc, watch};

use crate::clock::Clock;

/// The hops probed toward every target (ADR-0002).
const HOPS: [std::num::NonZeroU8; 3] = [
    std::num::NonZeroU8::MIN,
    std::num::NonZeroU8::MIN.saturating_add(1),
    std::num::NonZeroU8::MIN.saturating_add(2),
];

/// The default echo targets (ADR-0009). Configuration replaces these later.
const DEFAULT_ECHO_TARGETS: [(u32, IpAddr); 3] = [
    (1, IpAddr::V4(Ipv4Addr::new(1, 1, 1, 1))),
    (2, IpAddr::V4(Ipv4Addr::new(8, 8, 8, 8))),
    (3, IpAddr::V4(Ipv4Addr::new(9, 9, 9, 9))),
];

/// The default DNS targets (ADR-0009, ADR-0027): the same public resolvers, queried
/// over DNS. Their ids are the echo ids plus 10.
const DEFAULT_DNS_TARGETS: [(u32, IpAddr); 3] = [
    (11, IpAddr::V4(Ipv4Addr::new(1, 1, 1, 1))),
    (12, IpAddr::V4(Ipv4Addr::new(8, 8, 8, 8))),
    (13, IpAddr::V4(Ipv4Addr::new(9, 9, 9, 9))),
];

/// Every target's name for the UI, by role key.
fn target_names() -> Vec<TargetView> {
    let echo = DEFAULT_ECHO_TARGETS
        .iter()
        .map(|&(id, address)| (id, format!("{address} (ICMP)")));
    let dns = DEFAULT_DNS_TARGETS
        .iter()
        .map(|&(id, address)| (id, format!("{address} (DNS)")));
    echo.chain(dns)
        .map(|(id, name)| TargetView {
            key: Role::Target(TargetId::new(id)).key(),
            name,
        })
        .collect()
}

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
    let (kernel_tx, kernel) = watch::channel::<Option<KernelReportView>>(None);
    tokio::spawn({
        let live = live.clone();
        async move {
            if let Err(error) =
                netwatch_adapter_web::serve(listen, database, live, kernel, target_names()).await
            {
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
    let (resolutions_tx, mut resolutions) = mpsc::channel::<probe::Resolution>(1024);
    // Targets every 5 s with a 2 s timeout; hops every 1 s with a 0.9 s timeout
    // (ADR-0009). Both are valid for ICMP's sequence space.
    let echo = StreamConfig::new(
        Time::new::<second>(5.0),
        Time::new::<second>(2.0),
        SequenceSpace::ICMP,
    )
    .context("echo stream configuration")?;
    let hop = StreamConfig::new(
        Time::new::<second>(1.0),
        Time::new::<second>(0.9),
        SequenceSpace::ICMP,
    )
    .context("hop stream configuration")?;
    // The stack baseline (ADR-0008): ICMP echo over loopback, which the kernel always
    // numbers ifindex 1 (LOOPBACK_IFINDEX).
    let loopback = Identity::Interface {
        index: InterfaceIndex::new(std::num::NonZeroU32::MIN),
        name: InterfaceName::parse(b"lo").context("\"lo\" is a valid interface name")?,
    };
    for output in topology.observe(Observation::ProbeIdentified {
        role: Role::Kernel,
        identity: loopback,
    }) {
        handle(output, &observations_tx, &writes, &live, clock);
    }
    let (_kernel_pin_tx, kernel_pin) = watch::channel(topology.binding(&Role::Kernel).cloned());
    let baseline = StreamConfig::new(
        Time::new::<second>(1.0),
        Time::new::<second>(0.5),
        SequenceSpace::ICMP,
    )
    .context("loopback stream configuration")?;
    tokio::spawn(probe::run_role(
        Role::Kernel,
        IpAddr::V4(Ipv4Addr::LOCALHOST),
        probe::ProbeKind::Echo(None),
        baseline,
        kernel_pin,
        resolutions_tx.clone(),
        clock,
    ));

    for (id, destination) in DEFAULT_ECHO_TARGETS {
        let target = TargetId::new(id);
        let mut streams = vec![(Role::Target(target), None, echo)];
        for ttl in HOPS {
            // Hop 1 is the gateway for every target pinned to the same interface, so
            // one shared stream serves them all (ADR-0026). Separate streams would
            // trip the gateway's ICMP rate limit and show loss that isn't there.
            if ttl == HOPS[0] && id != DEFAULT_ECHO_TARGETS[0].0 {
                continue;
            }
            let ttl = HopTtl::new(ttl);
            streams.push((Role::Hop { target, ttl }, Some(ttl), hop));
        }
        for (role, ttl, config) in streams {
            tokio::spawn(probe::run_role(
                role,
                destination,
                probe::ProbeKind::Echo(ttl),
                config,
                pin.clone(),
                resolutions_tx.clone(),
                clock,
            ));
        }
    }

    for (id, server) in DEFAULT_DNS_TARGETS {
        tokio::spawn(probe::run_role(
            Role::Target(TargetId::new(id)),
            server,
            probe::ProbeKind::Dns,
            echo,
            pin.clone(),
            resolutions_tx.clone(),
            clock,
        ));
    }

    // The kernel layer's counters and pressure, read every second (ADR-0008).
    let mut kernel_watch = KernelWatch::new();
    let mut kernel_tick = tokio::time::interval(std::time::Duration::from_secs(1));

    loop {
        tokio::select! {
            _ = kernel_tick.tick() => {
                let Ok(reading) = tokio::task::spawn_blocking(netwatch_adapter_procfs::read).await else {
                    continue;
                };
                let report = KernelReportView::from(&kernel_watch.observe(reading));
                if !report.increases.is_empty() {
                    tracing::warn!(status = %report.status, increases = ?report.increases, "kernel counters grew");
                }
                let _ = live.send(LiveMessage::Kernel { report: report.clone() });
                kernel_tx.send_replace(Some(report));
            }
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
            Some(resolution) = resolutions.recv() => {
                for resolution in fan_out(resolution) {
                    let Some(sample) = attribute(resolution, &mut topology, clock, |output| {
                        handle(output, &observations_tx, &writes, &live, clock);
                    }) else {
                        continue;
                    };
                    log_sample(&sample);
                    let _ = live.send(LiveMessage::from(&sample));
                    let _ = writes.send(persist::Write::Sample(sample));
                }
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

/// The shared first-hop stream's resolutions, copied to every target's first-hop
/// role (ADR-0026). Each role then gets its own epoch and sample.
fn fan_out(resolution: probe::Resolution) -> Vec<probe::Resolution> {
    match resolution.role {
        Role::Hop { ttl, .. } if ttl.get() == HOPS[0] => DEFAULT_ECHO_TARGETS
            .iter()
            .map(|&(id, _)| probe::Resolution {
                role: Role::Hop {
                    target: TargetId::new(id),
                    ttl,
                },
                pin: resolution.pin.clone(),
                responder: resolution.responder,
                measurement: resolution.measurement,
            })
            .collect(),
        _ => vec![resolution],
    }
}

/// Turns a probe's resolution into a sample, attributed to the right epoch
/// (ADR-0020, ADR-0025):
/// - a target's sample carries its pin's epoch: the egress path it traveled
/// - a hop's identity is whoever answered, so Topology learns about it first, and
///   the sample carries the hop's resulting epoch
///
/// A hop that has never answered has no identity to attribute a loss to, so that
/// loss isn't recorded as a sample.
fn attribute(
    resolution: probe::Resolution,
    topology: &mut Topology,
    clock: Clock,
    mut emit: impl FnMut(Output),
) -> Option<Sample> {
    let epoch = match resolution.role {
        Role::Hop { .. } => {
            if let Some(responder) = resolution.responder {
                for output in topology.observe(Observation::ProbeIdentified {
                    role: resolution.role.clone(),
                    identity: Identity::Address(responder),
                }) {
                    emit(output);
                }
            }
            topology.binding(&resolution.role)?.epoch
        }
        _ => resolution.pin.epoch,
    };
    Some(Sample {
        role: resolution.role,
        epoch,
        at: clock.wall_now(),
        measurement: resolution.measurement,
    })
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
