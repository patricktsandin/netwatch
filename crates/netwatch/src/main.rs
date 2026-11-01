//! The composition root: reads the configuration, builds the adapters, connects them to
//! the domain state machines, and owns the async runtime (ADR-0012).
//!
//! This is the only crate allowed to use `anyhow` (ADR-0015).

mod clock;
mod config;
mod persist;
mod probe;

use std::net::{IpAddr, Ipv4Addr};
use std::num::NonZeroU64;

use anyhow::Context;
use netwatch_adapter_web::{
    BindingView, EventPoint, LayerReportView, LiveMessage, TargetView, status_text,
};
use netwatch_history::{Measurement, Sample};
use netwatch_host_health::{CounterWatch, KERNEL_RULES, NIC_RULES};
use netwatch_probing::health::{self, Health};
use netwatch_probing::{SequenceSpace, StreamConfig};
use netwatch_shared::uom::si::f64::Time;
use netwatch_shared::uom::si::time::{millisecond, second};
use netwatch_shared::{
    HopTtl, Identity, IdentityEpoch, InterfaceIndex, InterfaceName, Role, TargetId, WallTime,
};
use netwatch_topology::{Binding, Command, Observation, Output, Topology, TopologyEvent};
use std::collections::{BTreeMap, HashMap, VecDeque};
use tokio::sync::{broadcast, mpsc, watch};

use crate::clock::Clock;
use crate::config::{Config, TargetProbe};

/// The hops probed toward every target (ADR-0002).
const HOPS: [std::num::NonZeroU8; 3] = [
    std::num::NonZeroU8::MIN,
    std::num::NonZeroU8::MIN.saturating_add(1),
    std::num::NonZeroU8::MIN.saturating_add(2),
];

/// Every echo target, with the address it probes.
fn echo_targets(config: &Config) -> Vec<(TargetId, IpAddr)> {
    config
        .targets
        .iter()
        .filter_map(|t| match t.probe {
            TargetProbe::Echo(address) => Some((t.id, address)),
            _ => None,
        })
        .collect()
}

/// The echo target whose hop-1 stream is shared by every echo target of the same
/// address family (ADR-0026). IPv4 and IPv6 have different gateways.
fn shares_first_hop_for(echo: &[(TargetId, IpAddr)], id: TargetId) -> TargetId {
    let family = |address: &IpAddr| address.is_ipv4();
    let own = echo.iter().find(|&&(i, _)| i == id).map(|(_, a)| family(a));
    echo.iter()
        .find(|(_, address)| Some(family(address)) == own)
        .map_or(id, |&(i, _)| i)
}

/// Every target's name for the UI, by role key.
fn target_names(config: &Config) -> Vec<TargetView> {
    config
        .targets
        .iter()
        .map(|t| TargetView {
            key: Role::Target(t.id).key(),
            name: t.name.clone(),
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
    let config = config::load()?;
    let database = config.database.clone();
    let resume_after = persist::last_epoch(&database)
        .with_context(|| format!("opening {}", database.display()))?;
    let writes = persist::spawn(database.clone(), clock)?;

    // The UI listens on localhost only, unless configured otherwise (ADR-0007).
    let listen = config.listen;
    let names = target_names(&config);
    let (live, _) = broadcast::channel::<LiveMessage>(1024);
    let flush: netwatch_adapter_web::Flush = {
        let writes = writes.clone();
        std::sync::Arc::new(move || {
            let (done, flushed) = tokio::sync::oneshot::channel();
            let _ = writes.send(persist::Write::Flush(done));
            flushed
        })
    };
    let (layers_tx, layers) =
        watch::channel::<std::collections::BTreeMap<String, LayerReportView>>(Default::default());
    let (current_tx, current) = watch::channel(netwatch_adapter_web::Current::default());
    tokio::spawn({
        let live = live.clone();
        async move {
            if let Err(error) =
                netwatch_adapter_web::serve(listen, database, live, layers, names, flush, current)
                    .await
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
    // Sources that can't run: reported by the probe tasks and the kernel log watcher,
    // and shown with every kernel report (ADR-0007).
    let (disabled_tx, mut disabled_rx) = mpsc::unbounded_channel::<String>();
    let mut disabled = std::collections::BTreeSet::new();
    let reports = probe::Reports {
        resolutions: resolutions_tx.clone(),
        disabled: disabled_tx,
    };
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
        reports.clone(),
        clock,
    ));

    let echo_targets = echo_targets(&config);
    for &(target, destination) in &echo_targets {
        let mut streams = vec![(Role::Target(target), None, echo)];
        for ttl in HOPS {
            // Hop 1 is the gateway for every target pinned to the same interface, so
            // one shared stream serves them all (ADR-0026). Separate streams would
            // trip the gateway's ICMP rate limit and show loss that isn't there.
            if ttl == HOPS[0] && target != shares_first_hop_for(&echo_targets, target) {
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
                reports.clone(),
                clock,
            ));
        }
    }

    for (id, server) in config.targets.iter().filter_map(|t| match t.probe {
        TargetProbe::Dns(server) => Some((t.id, server)),
        _ => None,
    }) {
        tokio::spawn(probe::run_role(
            Role::Target(id),
            server,
            probe::ProbeKind::Dns,
            echo,
            pin.clone(),
            reports.clone(),
            clock,
        ));
    }

    // HTTPS every 10 s with a 5 s timeout (ADR-0009).
    let https = StreamConfig::new(
        Time::new::<second>(10.0),
        Time::new::<second>(5.0),
        SequenceSpace::ICMP,
    )
    .context("https stream configuration")?;
    for (id, target) in config.targets.iter().filter_map(|t| match &t.probe {
        TargetProbe::Https(target) => Some((t.id, target.clone())),
        _ => None,
    }) {
        tokio::spawn(probe::run_https_role(
            Role::Target(id),
            target,
            https,
            pin.clone(),
            resolutions_tx.clone(),
            clock,
        ));
    }

    // The kernel layer's counters and pressure, read every second (ADR-0008).
    let mut kernel_watch = CounterWatch::new(KERNEL_RULES);
    // The NIC's counters only compare within one device, so the watch restarts
    // whenever the NIC changes (a failover).
    let mut nic_watch: Option<(InterfaceIndex, CounterWatch)> = None;
    let mut kernel_tick = tokio::time::interval(std::time::Duration::from_secs(1));
    // Every role's recent outcomes, and the layers' latest reports, from which the
    // daemon grades every role each second (ADR-0030).
    let mut recent: HashMap<Role, VecDeque<(WallTime, bool)>> = HashMap::new();
    let mut layer_health: BTreeMap<Role, Health> = BTreeMap::new();
    // Wall-clock jumps and changes to network sysctls are events (ADR-0008).
    let mut last_clocks = (clock.wall_now(), clock.now());
    let mut last_sysctls = netwatch_adapter_procfs::sysctls();
    // Network-relevant kernel log lines become event markers (ADR-0008). Reading the
    // log needs CAP_SYSLOG; without it the source is disabled, and says so (ADR-0007).
    let (kmsg_tx, mut kmsg) = mpsc::channel::<String>(64);
    if let Err(error) = netwatch_adapter_procfs::watch_kernel_log(kmsg_tx) {
        let hint = if error.kind() == std::io::ErrorKind::PermissionDenied {
            " (needs CAP_SYSLOG)"
        } else {
            ""
        };
        let reason = format!("kernel log: {error}{hint}");
        tracing::warn!(%error, "kernel log disabled");
        note("source-disabled", reason.clone(), &writes, &live, clock);
        disabled.insert(reason);
    }

    loop {
        tokio::select! {
            _ = kernel_tick.tick() => {
                let Ok(reading) = tokio::task::spawn_blocking(netwatch_adapter_procfs::read).await else {
                    continue;
                };
                layer_health.insert(
                    Role::Kernel,
                    publish_layer(
                        Role::Kernel,
                        &kernel_watch.observe(reading),
                        &disabled.iter().cloned().collect::<Vec<_>>(),
                        &live,
                        &layers_tx,
                    ),
                );
                let clocks = (clock.wall_now(), clock.now());
                let wall_us = clocks.0.as_unix_micros() - last_clocks.0.as_unix_micros();
                let monotonic_us = i64::try_from(clocks.1.as_nanos().saturating_sub(last_clocks.1.as_nanos()) / 1_000).unwrap_or(i64::MAX);
                if (wall_us - monotonic_us).abs() > 1_000_000 {
                    let jump_s = (wall_us - monotonic_us) as f64 / 1e6;
                    tracing::warn!(jump_s, "wall clock jumped");
                    note("clock-jump", format!("wall clock jumped by {jump_s:+.1} s"), &writes, &live, clock);
                }
                last_clocks = clocks;
                let sysctls = netwatch_adapter_procfs::sysctls();
                for (name, value) in &sysctls {
                    if let Some(before) = last_sysctls.get(name).filter(|before| *before != value) {
                        tracing::warn!(%name, %before, %value, "network sysctl changed");
                        note("config-drift", format!("{name}: {before} → {value}"), &writes, &live, clock);
                    }
                }
                last_sysctls = sysctls;
                let nic = topology.binding(&Role::Nic).and_then(|b| match b.identity {
                    Identity::Interface { index, .. } => Some(index),
                    Identity::Address(_) => None,
                });
                if let Some(index) = nic {
                    if nic_watch.as_ref().is_none_or(|(watched, _)| *watched != index) {
                        nic_watch = Some((index, CounterWatch::new(NIC_RULES)));
                    }
                    let counters = tokio::task::spawn_blocking(move || {
                        netwatch_adapter_netlink::link_counters(index.get())
                    })
                    .await;
                    if let (Ok(Ok(counters)), Some((_, watch))) = (counters, nic_watch.as_mut()) {
                        let reading = netwatch_host_health::Reading { counters, ..Default::default() };
                        layer_health.insert(
                            Role::Nic,
                            publish_layer(Role::Nic, &watch.observe(reading), &[], &live, &layers_tx),
                        );
                    }
                }
                publish_current(&recent, &layer_health, &topology, clock.wall_now(), &live, &current_tx);
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
                for resolution in fan_out(resolution, &echo_targets) {
                    let Some(sample) = attribute(resolution, &mut topology, clock, |output| {
                        handle(output, &observations_tx, &writes, &live, clock);
                    }) else {
                        continue;
                    };
                    log_sample(&sample);
                    let outcomes = recent.entry(sample.role.clone()).or_default();
                    outcomes.push_back((sample.at, matches!(sample.measurement, Measurement::RoundTrip(_))));
                    if outcomes.len() > health::WINDOW {
                        outcomes.pop_front();
                    }
                    let _ = live.send(LiveMessage::from(&sample));
                    let _ = writes.send(persist::Write::Sample(sample));
                }
            }
            Some(reason) = disabled_rx.recv() => {
                disabled.insert(reason);
            }
            Some(message) = kmsg.recv() => {
                if let Some(line) = netwatch_host_health::classify(&message) {
                    tracing::info!(kind = ?line.kind, interface = ?line.interface, "kernel log: {}", line.message);
                    note("kernel-log", line.message, &writes, &live, clock);
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
fn fan_out(resolution: probe::Resolution, echo: &[(TargetId, IpAddr)]) -> Vec<probe::Resolution> {
    match resolution.role {
        Role::Hop { target, ttl } if ttl.get() == HOPS[0] => echo
            .iter()
            .filter(|&&(id, _)| shares_first_hop_for(echo, id) == target)
            .map(|&(id, _)| probe::Resolution {
                role: Role::Hop { target: id, ttl },
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

/// Records an event that isn't a topology event, and shows it live.
fn note(
    kind: &str,
    detail: String,
    writes: &std::sync::mpsc::Sender<persist::Write>,
    live: &broadcast::Sender<LiveMessage>,
    clock: Clock,
) {
    let stored = netwatch_adapter_sqlite::StoredEvent {
        at: clock.wall_now(),
        kind: kind.to_owned(),
        role: Some(Role::Kernel.key()),
        epoch: None,
        detail: Some(detail),
    };
    let _ = live.send(LiveMessage::Event {
        event: EventPoint::from(stored.clone()),
    });
    let _ = writes.send(persist::Write::Note(stored));
}

/// Publishes a layer's health report to the UI, and logs it when something got worse.
/// A role's outcomes older than this are too old to grade it by, so it is `Unknown`.
const STALE_AFTER_US: i64 = 30_000_000;

/// Grades every role from its recent outcomes, reads loss along each path, folds in
/// the layer reports, and publishes the result together with every role's current
/// binding. This is the only place statuses are decided (ADR-0030).
fn publish_current(
    recent: &HashMap<Role, VecDeque<(WallTime, bool)>>,
    layers: &BTreeMap<Role, Health>,
    topology: &Topology,
    now: WallTime,
    live: &broadcast::Sender<LiveMessage>,
    current: &watch::Sender<netwatch_adapter_web::Current>,
) {
    let mut grades: BTreeMap<Role, Health> = recent
        .iter()
        .map(|(role, outcomes)| {
            let answered: Vec<bool> = outcomes.iter().map(|&(_, answered)| answered).collect();
            let fresh = outcomes.back().is_some_and(|(at, _)| {
                now.as_unix_micros() - at.as_unix_micros() <= STALE_AFTER_US
            });
            (role.clone(), health::grade(&answered, fresh))
        })
        .collect();
    for (role, &layer) in layers {
        let probes = grades.get(role).copied().unwrap_or(Health::Unknown);
        grades.insert(role.clone(), probes.worse(layer));
    }
    let statuses: BTreeMap<String, String> = health::along_paths(&grades)
        .into_iter()
        .map(|(role, health)| (role.key(), status_text(health)))
        .collect();
    let bindings: BTreeMap<String, BindingView> = topology
        .bindings()
        .map(|(role, binding)| {
            let identity = match &binding.identity {
                Identity::Interface { index, name } => format!("{name} (ifindex {})", index.get()),
                Identity::Address(address) => address.to_string(),
            };
            (
                role.key(),
                BindingView {
                    identity,
                    epoch: binding.epoch.get().get(),
                },
            )
        })
        .collect();
    let _ = live.send(LiveMessage::Statuses {
        statuses: statuses.clone(),
    });
    let _ = live.send(LiveMessage::Bindings {
        bindings: bindings.clone(),
    });
    current.send_replace(netwatch_adapter_web::Current { statuses, bindings });
}

/// Publishes a layer's report, and returns its grade for [`publish_current`].
fn publish_layer(
    role: Role,
    report: &netwatch_host_health::LayerReport,
    disabled: &[String],
    live: &broadcast::Sender<LiveMessage>,
    layers: &watch::Sender<std::collections::BTreeMap<String, LayerReportView>>,
) -> Health {
    use netwatch_host_health::LayerStatus;
    let view = LayerReportView {
        disabled: disabled.to_vec(),
        ..LayerReportView::from(report)
    };
    if !view.increases.is_empty() {
        tracing::warn!(?role, status = %view.status, increases = ?view.increases, "counters grew");
    }
    let _ = live.send(LiveMessage::Layer {
        role: role.key(),
        report: view.clone(),
    });
    layers.send_modify(|all| {
        all.insert(role.key(), view);
    });
    match report.status {
        LayerStatus::Unknown => Health::Unknown,
        LayerStatus::Healthy => Health::Healthy,
        LayerStatus::Degraded => Health::Degraded,
        LayerStatus::Failing => Health::Failing,
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
