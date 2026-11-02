//! The composition root: reads the configuration, builds the adapters, connects them to
//! the domain state machines, and owns the async runtime (ADR-0012).
//!
//! This is the only crate allowed to use `anyhow` (ADR-0015).

mod clock;
mod config;
mod names;
mod persist;
mod probe;
mod public;
mod radio;
mod settings;
mod trace;

use std::net::{IpAddr, Ipv4Addr};
use std::num::NonZeroU64;

use anyhow::Context;
use netwatch_adapter_web::{
    BindingView, EventPoint, IncidentView, LayerReportView, LiveMessage, ReadingPoint, RowView,
    TargetView, status_text,
};
use netwatch_history::{Measurement, Reading, Sample};
use netwatch_host_health::{CounterWatch, KERNEL_RULES, NIC_RULES, Throughput};
use netwatch_probing::health::{self, Health};
use netwatch_probing::incident::{Incident, IncidentTracker};
use netwatch_probing::sharing;
use netwatch_probing::trace::{TraceRequest, TraceSchedule};
use netwatch_probing::{SequenceSpace, StreamConfig};
use netwatch_shared::uom::si::f64::Time;
use netwatch_shared::uom::si::time::{millisecond, second};
use netwatch_shared::{
    Gauge, Identity, IdentityEpoch, InterfaceIndex, InterfaceName, Role, TargetId, WallTime,
};
use netwatch_topology::{Binding, Command, Observation, Output, Topology, TopologyEvent};
use std::collections::{BTreeMap, HashMap, VecDeque};
use tokio::sync::{broadcast, mpsc, watch};

use crate::clock::Clock;
use crate::config::{Config, TargetProbe};

/// Every target with hops (ICMP and DNS, ADR-0043), with the address they are
/// probed toward. HTTPS targets have no fixed address.
fn hopped_targets(config: &Config) -> Vec<(TargetId, IpAddr)> {
    config
        .targets
        .iter()
        .filter_map(|t| match t.probe {
            TargetProbe::Echo(address) | TargetProbe::Dns(address) => Some((t.id, address)),
            TargetProbe::Https(_) => None,
        })
        .collect()
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
    // What each role was, and the public address, before this start: a role that
    // comes back the same has resumed, not changed (ADR-0044). Read before the writer
    // starts, which closes the epochs the last run left open.
    let (identities_before, mut public_before) =
        persist::before_start(&database).unwrap_or_else(|error| {
            tracing::warn!(%error, "cannot read the identities before this start");
            Default::default()
        });
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
    let recorder = Recorder {
        writes: writes.clone(),
        live: live.clone(),
        current: current_tx.subscribe(),
        clock,
        before_start: std::sync::Mutex::new(identities_before),
        started: clock.now(),
    };
    recorder.note(
        "started",
        Role::Kernel,
        format!("netwatch {} started", env!("CARGO_PKG_VERSION")),
    );
    let settings: std::sync::Arc<dyn netwatch_adapter_web::Settings> =
        std::sync::Arc::new(settings::FileSettings {
            path: config.source.clone(),
            database: database.clone(),
            writes: writes.clone(),
        });
    // Flushing history also deletes the traces the network map draws (ADR-0038), so
    // the main loop is told to forget its own copies: those it sent the writer before
    // the flush. A generation, taken under the same lock as the send of the flush or
    // of a trace, says which side of the flush each trace's write is on.
    let history_generation = std::sync::Arc::new(std::sync::Mutex::new(0_u64));
    let (history_flushed_tx, mut history_flushed) = mpsc::unbounded_channel::<u64>();
    let flush_history: netwatch_adapter_web::FlushHistory = {
        let writes = writes.clone();
        let history_generation = history_generation.clone();
        std::sync::Arc::new(move || {
            let (done, written) = tokio::sync::oneshot::channel();
            let (tell, result) = tokio::sync::oneshot::channel();
            // Only a counter is guarded, so a poisoned lock still holds a valid one.
            let generation = {
                let mut generation = history_generation
                    .lock()
                    .unwrap_or_else(std::sync::PoisonError::into_inner);
                *generation += 1;
                let _ = writes.send(persist::Write::FlushHistory(done));
                *generation
            };
            let history_flushed = history_flushed_tx.clone();
            tokio::spawn(async move {
                if let Ok(outcome) = written.await {
                    if outcome.is_ok() {
                        let _ = history_flushed.send(generation);
                    }
                    let _ = tell.send(outcome);
                }
            });
            result
        })
    };
    tokio::spawn({
        let live = live.clone();
        async move {
            let daemon = netwatch_adapter_web::Daemon {
                flush_history,
                database,
                live,
                layers,
                targets: names,
                flush,
                current,
                settings,
            };
            if let Err(error) = netwatch_adapter_web::serve(listen, daemon).await {
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
        handle(output, &observations_tx, &recorder);
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

    for (target, destination) in config.targets.iter().filter_map(|t| match t.probe {
        TargetProbe::Echo(address) => Some((t.id, address)),
        _ => None,
    }) {
        tokio::spawn(probe::run_role(
            Role::Target(target),
            destination,
            probe::ProbeKind::Echo(None),
            echo,
            pin.clone(),
            reports.clone(),
            clock,
        ));
    }
    // Hop streams are shared where the hops are the same router (ADR-0026,
    // ADR-0043): separate streams would trip routers' ICMP rate limits and show loss
    // that isn't there.
    let hopped = hopped_targets(&config);
    let hop_streams = sharing::plan(&hopped);
    // Where each target with hops is: for the map to see where a path reaches it, and
    // for path traces.
    let destinations: BTreeMap<TargetId, IpAddr> = hopped.into_iter().collect();
    current_tx.send_modify(|c| c.rows = shared_rows(&hop_streams, &BTreeMap::new()));
    for stream in &hop_streams {
        tokio::spawn(probe::run_role(
            stream.role(),
            stream.destination,
            probe::ProbeKind::Echo(Some(stream.ttl)),
            hop,
            pin.clone(),
            reports.clone(),
            clock,
        ));
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
    let mut nic_watch: Option<(InterfaceIndex, CounterWatch, Throughput)> = None;
    let mut kernel_tick = tokio::time::interval(std::time::Duration::from_secs(1));
    // Every role's recent outcomes, and the layers' latest reports, from which the
    // daemon grades every role each second (ADR-0030).
    let mut recent: HashMap<Role, VecDeque<(WallTime, bool)>> = HashMap::new();
    let mut layer_health: BTreeMap<Role, Health> = BTreeMap::new();
    // Facts about the host for the header and the report (ADR-0039), and the public
    // identity, read every 5 minutes.
    current_tx.send_modify(|c| {
        let read = |path: &str| {
            std::fs::read_to_string(path)
                .ok()
                .map(|t| t.trim().to_owned())
        };
        c.host.hostname = read("/proc/sys/kernel/hostname");
        c.host.kernel = read("/proc/sys/kernel/osrelease");
        c.host.version = env!("CARGO_PKG_VERSION").to_owned();
    });
    let mut public_identities = public::spawn(pin.clone());
    let mut public_identity: Option<netwatch_host_health::facts::PublicIdentity> = None;
    // The radios, read every 5 s, and what each was last attached to (ADR-0036).
    let mut radios = radio::spawn(pin.clone());
    let mut attachments: HashMap<Role, Option<String>> = HashMap::new();
    // Each grading also opens, updates, and closes incidents (ADR-0031).
    let mut incidents = IncidentTracker::new(Time::new::<second>(10.0));
    // Path traces (ADR-0040) of every target with a fixed address, with router names
    // for whoever answers them and for every hop. The first routine traces wait half
    // a minute, for the egress path and the hops to be known.
    let names = names::Names::new(config::system_resolver(), config.asn_lookups);
    names::follow_hops(names.clone(), current_tx.subscribe());
    let mut traces = TraceSchedule::new(
        destinations.keys().copied(),
        Time::new::<second>(600.0),
        Time::new::<second>(6.0 * 3600.0),
        clock.now().after(Time::new::<second>(30.0)),
    );
    // Each target's latest trace, for the network map (ADR-0042), and the flush
    // generation each one's write was sent in.
    let mut trace_generations: BTreeMap<TargetId, u64> = BTreeMap::new();
    let mut latest_traces = persist::latest_traces(&config.database, destinations.keys().copied())
        .unwrap_or_else(|error| {
            tracing::warn!(%error, "cannot read the latest path traces");
            BTreeMap::new()
        });
    let (traced_tx, mut traced) = mpsc::channel::<trace::Finished>(8);
    let tracer = trace::Tracer::new(pin.clone(), names.clone(), clock, traced_tx);
    let start_trace = |request: TraceRequest| {
        if let Some(&destination) = destinations.get(&request.target) {
            tracing::info!(target = request.target.get(), kind = ?request.kind, "path trace");
            tracer.start(request, destination);
        }
    };
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
        recorder.note("source-disabled", Role::Kernel, reason.clone());
        disabled.insert(reason);
    }

    // Drop reasons, counted in the kernel by an eBPF program (ADR-0008). Loading it
    // needs CAP_BPF and CAP_PERFMON; without them the source is disabled, and says so.
    let drop_reasons = match netwatch_adapter_ebpf::DropReasons::start() {
        Ok(drops) => Some(drops),
        Err(error) => {
            let hint = if error.is_permission() {
                " (needs CAP_BPF and CAP_PERFMON)"
            } else {
                ""
            };
            let reason = format!("drop reasons: {error}{hint}");
            tracing::warn!(%error, "drop reasons disabled");
            recorder.note("source-disabled", Role::Kernel, reason.clone());
            disabled.insert(reason);
            None
        }
    };

    loop {
        tokio::select! {
            _ = kernel_tick.tick() => {
                let Ok(mut reading) = tokio::task::spawn_blocking(netwatch_adapter_procfs::read).await else {
                    continue;
                };
                if let Some(drops) = &drop_reasons {
                    reading.counters.extend(drops.read());
                }
                let kernel_report = kernel_watch.observe(reading);
                current_tx.send_modify(|c| {
                    c.host.clock_synchronized = kernel_report.clock.map(|(synchronized, _)| synchronized);
                    c.host.clock_error_us = kernel_report.clock.map(|(_, error)| error);
                });
                layer_health.insert(
                    Role::Kernel,
                    publish_layer(
                        Role::Kernel,
                        &kernel_report,
                        &disabled.iter().cloned().collect::<Vec<_>>(),
                        &[],
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
                    recorder.note("clock-jump", Role::Kernel, format!("wall clock jumped by {jump_s:+.1} s"));
                }
                last_clocks = clocks;
                let sysctls = netwatch_adapter_procfs::sysctls();
                for (name, value) in &sysctls {
                    if let Some(before) = last_sysctls.get(name).filter(|before| *before != value) {
                        tracing::warn!(%name, %before, %value, "network sysctl changed");
                        recorder.note("config-drift", Role::Kernel, format!("{name}: {before} → {value}"));
                    }
                }
                last_sysctls = sysctls;
                let nic = topology.binding(&Role::Nic).and_then(|b| match b.identity {
                    Identity::Interface { index, .. } => Some(index),
                    Identity::Address(_) => None,
                });
                if let Some(index) = nic {
                    if nic_watch.as_ref().is_none_or(|(watched, _, _)| *watched != index) {
                        nic_watch = Some((index, CounterWatch::new(NIC_RULES), Throughput::default()));
                    }
                    let counters = tokio::task::spawn_blocking(move || {
                        netwatch_adapter_netlink::link_counters(index.get())
                    })
                    .await;
                    if let (Ok(Ok(counters)), Some((_, watch, throughput))) = (counters, nic_watch.as_mut()) {
                        // The NIC's throughput is graphed on its row (ADR-0032).
                        let epoch = topology.binding(&Role::Nic).map(|b| b.epoch);
                        if let (Some((rx, tx)), Some(epoch)) = (throughput.observe(&counters, clock.now()), epoch) {
                            // One timestamp for both, so they are drawn as one reading.
                            let at = clock.wall_now();
                            for (gauge, value) in [(Gauge::RxThroughput, rx), (Gauge::TxThroughput, tx)] {
                                recorder.reading(Reading { role: Role::Nic, epoch, at, gauge, value });
                            }
                        }
                        // Link facts (ADR-0039): notes on the row, and carrier changes as a
                        // counter, so a flap grades the NIC.
                        let facts = topology.binding(&Role::Nic).and_then(|b| match &b.identity {
                            Identity::Interface { name, .. } => Some(netwatch_adapter_procfs::link_facts(name.as_str())),
                            Identity::Address(_) => None,
                        });
                        let mut counters = counters;
                        if let Some(changes) = facts.as_ref().and_then(|f| f.carrier_changes) {
                            counters.insert("Link.CarrierChanges".to_owned(), changes);
                        }
                        let notes: Vec<String> = facts.iter().map(|f| f.summary()).filter(|s| !s.is_empty()).collect();
                        current_tx.send_modify(|c| {
                            c.host.link = notes.first().cloned();
                            c.host.link_speed_mbps = facts.as_ref().and_then(|f| f.speed_mbps);
                        });
                        let reading = netwatch_host_health::Reading { counters, ..Default::default() };
                        layer_health.insert(
                            Role::Nic,
                            publish_layer(Role::Nic, &watch.observe(reading), &[], &notes, &live, &layers_tx),
                        );
                    }
                }
                let now = clock.wall_now();
                let grades = publish_current(&recent, &layer_health, &topology, &names.known(), &latest_traces, &hop_streams, &destinations, now, &live, &current_tx);
                for incident in incidents.grade(now, &grades) {
                    tracing::info!(role = ?incident.role, severity = ?incident.severity, cause = ?incident.cause, ended = incident.ended.is_some(), "incident");
                    if let Some(request) = traces.incident(&incident, clock.now()) {
                        start_trace(request);
                    }
                    recorder.incident(incident);
                }
                for request in traces.tick(clock.now()) {
                    start_trace(request);
                }
            }
            Some(observation) = observations.recv() => {
                for output in topology.observe(observation) {
                    handle(output, &observations_tx, &recorder);
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
                for resolution in fan_out(resolution, &hop_streams) {
                    let Some(sample) = attribute(resolution, &mut topology, clock, |output| {
                        handle(output, &observations_tx, &recorder);
                    }) else {
                        continue;
                    };
                    log_sample(&sample);
                    let answered = matches!(sample.measurement, Measurement::RoundTrip(_));
                    incidents.outcome(&sample.role, sample.at, answered);
                    let outcomes = recent.entry(sample.role.clone()).or_default();
                    outcomes.push_back((sample.at, answered));
                    if outcomes.len() > health::WINDOW {
                        outcomes.pop_front();
                    }
                    let _ = live.send(LiveMessage::from(&sample));
                    let _ = writes.send(persist::Write::Sample(sample));
                }
            }
            Some(finished) = traced.recv() => {
                if let Some(trace) = finished.trace {
                    recorder.note("trace", trace.role.clone(), trace::summary(&trace));
                    let generation = history_generation.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
                    trace_generations.insert(finished.target, *generation);
                    latest_traces.insert(finished.target, trace.clone());
                    let _ = writes.send(persist::Write::Trace(trace));
                    drop(generation);
                }
                if let Some(request) = traces.finished(finished.target, clock.now()) {
                    start_trace(request);
                }
            }
            Some(flushed) = history_flushed.recv() => {
                // Traces written before the flush are gone; any written after it stay.
                latest_traces.retain(|target, _| trace_generations.get(target).is_some_and(|&g| g >= flushed));
            }
            Some(reason) = disabled_rx.recv() => {
                disabled.insert(reason);
            }
            Some(identity) = public_identities.recv() => {
                if public_identity.as_ref() != Some(&identity) {
                    // The first one this run compares with the last one before it.
                    let before = public_identity.as_ref().map(|p| p.text()).or_else(|| public_before.take());
                    let now = identity.text();
                    if before.as_ref() != Some(&now) {
                        let detail = match before {
                            Some(before) => format!("{before} → {now}"),
                            None => now,
                        };
                        recorder.note("public-address", Role::LocalInterface, detail);
                    }
                    current_tx.send_modify(|c| {
                        c.host.public = Some(identity.text());
                        c.host.public_address = Some(identity.address.to_string());
                    });
                    public_identity = Some(identity);
                }
            }
            Some(update) = radios.recv() => {
                let (role, identity, report) = match update {
                    radio::Update::Wifi(device) => (
                        Role::Wifi,
                        Some(Identity::Interface { index: device.index, name: device.name }),
                        netwatch_host_health::radio::wifi_report(&device.link),
                    ),
                    radio::Update::Cellular(identity, modem) => (
                        Role::Cellular,
                        identity,
                        netwatch_host_health::radio::cellular_report(&modem.link),
                    ),
                };
                if let Some(identity) = identity {
                    for output in topology.observe(Observation::ProbeIdentified { role: role.clone(), identity }) {
                        handle(output, &observations_tx, &recorder);
                    }
                }
                // A new access point or network is an event on the radio's row.
                let previous = attachments.insert(role.clone(), report.attachment.clone());
                if let (Some(Some(before)), Some(now)) = (&previous, &report.attachment)
                    && before != now
                {
                    recorder.note("radio", role.clone(), format!("{before} → {now}"));
                }
                if let Some(epoch) = topology.binding(&role).map(|b| b.epoch) {
                    let at = clock.wall_now();
                    for &(gauge, value) in &report.readings {
                        recorder.reading(Reading { role: role.clone(), epoch, at, gauge, value });
                    }
                }
                layer_health.insert(role.clone(), radio::publish(&role, &report, &live, &layers_tx));
            }
            Some(message) = kmsg.recv() => {
                if let Some(line) = netwatch_host_health::classify(&message) {
                    tracing::info!(kind = ?line.kind, interface = ?line.interface, "kernel log: {}", line.message);
                    // A line about the interface carrying the NIC or tunnel role
                    // belongs on that row; anything else on the kernel's (ADR-0031).
                    let role = [Role::Nic, Role::Tunnel]
                        .into_iter()
                        .find(|role| {
                            topology.binding(role).is_some_and(|b| match &b.identity {
                                Identity::Interface { name, .. } => {
                                    line.interface.as_deref() == Some(name.as_str())
                                }
                                Identity::Address(_) => false,
                            })
                        })
                        .unwrap_or(Role::Kernel);
                    recorder.note("kernel-log", role, line.message);
                }
            }
            _ = tokio::signal::ctrl_c() => break,
        }
    }
    Ok(())
}

fn handle(output: Output, observations: &mpsc::Sender<Observation>, recorder: &Recorder) {
    match output {
        Output::Event(event) => {
            log_event(&event);
            recorder.topology(event);
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

/// A shared hop stream's resolutions, copied to the hop role of every target it
/// serves (ADR-0026, ADR-0043). Each role then gets its own epoch and sample.
fn fan_out(
    resolution: probe::Resolution,
    streams: &[sharing::HopStream],
) -> Vec<probe::Resolution> {
    match streams.iter().find(|s| s.role() == resolution.role) {
        Some(stream) => stream
            .serves
            .iter()
            .map(|&target| probe::Resolution {
                role: Role::Hop {
                    target,
                    ttl: stream.ttl,
                },
                pin: resolution.pin.clone(),
                responder: resolution.responder,
                measurement: resolution.measurement,
            })
            .collect(),
        None => vec![resolution],
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

/// Records events and incidents, and shows them live. Every event is shown with the
/// roles it concerns, among the roles graded so far (ADR-0031).
struct Recorder {
    writes: std::sync::mpsc::Sender<persist::Write>,
    live: broadcast::Sender<LiveMessage>,
    current: watch::Receiver<netwatch_adapter_web::Current>,
    clock: Clock,
    /// Each role's identity before this start, by role key, until its first epoch.
    before_start: std::sync::Mutex<BTreeMap<String, String>>,
    /// When this run started.
    started: netwatch_shared::Instant,
}

/// How long after a start a role's first epoch can still resume one from before it
/// (ADR-0046). Binding every role takes seconds; an identity that comes back later
/// came back after an outage, which is a change.
const RESUME_WITHIN_S: f64 = 60.0;

impl Recorder {
    fn show(&self, stored: netwatch_adapter_sqlite::StoredEvent) {
        let known = self.current.borrow().known_roles();
        let _ = self.live.send(LiveMessage::Event {
            event: EventPoint::new(stored, &known),
        });
    }

    /// Records a topology event. A role's first epoch this run, soon after the start,
    /// with the identity it had before the start resumes it, and isn't a change
    /// (ADR-0044, ADR-0046).
    fn topology(&self, event: TopologyEvent) {
        let at = self.clock.wall_now();
        let resumed = match &event {
            TopologyEvent::EpochStarted { role, .. } => {
                let before = self
                    .before_start
                    .lock()
                    .ok()
                    .and_then(|mut before| before.remove(&role.key()));
                let identity = netwatch_adapter_sqlite::describe(&event, at, false).detail;
                let soon =
                    self.clock.now().since(self.started) <= Time::new::<second>(RESUME_WITHIN_S);
                soon && before.is_some() && before == identity
            }
            _ => false,
        };
        self.show(netwatch_adapter_sqlite::describe(&event, at, resumed));
        let _ = self.writes.send(persist::Write::Event(event, at, resumed));
    }

    /// Records an event that isn't a topology event: a kernel log line, a clock jump,
    /// config drift, or a disabled source.
    fn note(&self, kind: &str, role: Role, detail: String) {
        let stored = netwatch_adapter_sqlite::StoredEvent {
            at: self.clock.wall_now(),
            kind: kind.to_owned(),
            role: Some(role.key()),
            epoch: None,
            detail: Some(detail),
        };
        self.show(stored.clone());
        let _ = self.writes.send(persist::Write::Note(stored));
    }

    fn reading(&self, reading: Reading) {
        let _ = self.live.send(LiveMessage::Reading {
            role: reading.role.key(),
            reading: ReadingPoint::from(&reading),
        });
        let _ = self.writes.send(persist::Write::Reading(reading));
    }

    fn incident(&self, incident: Incident) {
        let _ = self.live.send(LiveMessage::Incident {
            incident: IncidentView::from(&incident),
        });
        let _ = self.writes.send(persist::Write::Incident(incident));
    }
}

/// Publishes a layer's health report to the UI, and logs it when something got worse.
/// A role's outcomes older than this are too old to grade it by, so it is `Unknown`.
const STALE_AFTER_US: i64 = 30_000_000;

/// Grades every role from its recent outcomes, reads loss along each path, folds in
/// the layer reports, and publishes the result together with every role's current
/// binding, and the network map built from them (ADR-0042). This is the only place
/// statuses are decided (ADR-0030). Returns the grades, from which incidents are
/// tracked.
#[allow(clippy::too_many_arguments)] // Each is one of the daemon's views.
fn publish_current(
    recent: &HashMap<Role, VecDeque<(WallTime, bool)>>,
    layers: &BTreeMap<Role, Health>,
    topology: &Topology,
    names: &HashMap<IpAddr, String>,
    traces: &BTreeMap<TargetId, netwatch_adapter_sqlite::StoredTrace>,
    hop_streams: &[sharing::HopStream],
    destinations: &BTreeMap<TargetId, IpAddr>,
    now: WallTime,
    live: &broadcast::Sender<LiveMessage>,
    current: &watch::Sender<netwatch_adapter_web::Current>,
) -> BTreeMap<Role, Health> {
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
    let grades = health::along_paths(&grades);
    let statuses: BTreeMap<String, String> = grades
        .iter()
        .map(|(role, &health)| (role.key(), status_text(health)))
        .collect();
    let bindings: BTreeMap<String, BindingView> = topology
        .bindings()
        .map(|(role, binding)| {
            let (identity, name) = match &binding.identity {
                Identity::Interface { index, name } => {
                    (format!("{name} (ifindex {})", index.get()), None)
                }
                Identity::Address(address) => (address.to_string(), names.get(address).cloned()),
            };
            (
                role.key(),
                BindingView {
                    identity,
                    name,
                    epoch: binding.epoch.get().get(),
                },
            )
        })
        .collect();
    let map = network_map(&grades, topology, names, traces, destinations, &bindings);
    let rows = shared_rows(hop_streams, &grades);
    let _ = live.send(LiveMessage::Statuses {
        statuses: statuses.clone(),
        rows: rows
            .iter()
            .map(|row| (row.key.clone(), row.status.clone()))
            .collect(),
    });
    let _ = live.send(LiveMessage::Bindings {
        bindings: bindings.clone(),
    });
    current.send_modify(|c| {
        if c.map != map {
            let _ = live.send(LiveMessage::Map { map: map.clone() });
            c.map = map;
        }
        c.statuses = statuses;
        c.rows = rows;
        c.bindings = bindings;
        c.names = names
            .iter()
            .map(|(address, name)| (address.to_string(), name.clone()))
            .collect();
    });
    grades
}

/// The rows several hop roles share, one per stream that serves more than one, with
/// the worst of their statuses (ADR-0045). They come from the plan, not the grades:
/// a row none of whose members is graded yet is still one row, `unknown`, so a UI that
/// loads right after a start draws one row per stream.
fn shared_rows(
    hop_streams: &[sharing::HopStream],
    grades: &BTreeMap<Role, Health>,
) -> Vec<RowView> {
    hop_streams
        .iter()
        .filter(|stream| stream.serves.len() > 1)
        .map(|stream| RowView {
            key: stream.role().key(),
            members: stream
                .serves
                .iter()
                .map(|&target| {
                    Role::Hop {
                        target,
                        ttl: stream.ttl,
                    }
                    .key()
                })
                .collect(),
            status: status_text(sharing::row_health(stream, grades).unwrap_or(Health::Unknown)),
        })
        .collect()
}

/// The network map (ADR-0042), from the grades, the hops' responding addresses, and
/// each target's latest trace. Router names come from the lookups so far, and else
/// from the trace that saw the router.
fn network_map(
    grades: &BTreeMap<Role, Health>,
    topology: &Topology,
    names: &HashMap<IpAddr, String>,
    traces: &BTreeMap<TargetId, netwatch_adapter_sqlite::StoredTrace>,
    destinations: &BTreeMap<TargetId, IpAddr>,
    bindings: &BTreeMap<String, BindingView>,
) -> netwatch_adapter_web::MapView {
    let hops: BTreeMap<Role, IpAddr> = topology
        .bindings()
        .filter_map(|(role, binding)| match (role, &binding.identity) {
            (Role::Hop { .. }, Identity::Address(address)) => Some((role.clone(), *address)),
            _ => None,
        })
        .collect();
    let latest: BTreeMap<TargetId, &netwatch_probing::trace::PathTrace> = traces
        .iter()
        .map(|(&target, stored)| (target, &stored.trace))
        .collect();
    let name = |address: IpAddr| {
        names
            .get(&address)
            .or_else(|| traces.values().find_map(|t| t.names.get(&address)))
            .cloned()
    };
    let traced = traces.iter().map(|(&t, stored)| (t, stored.at)).collect();
    let map = netwatch_probing::map::build(grades, &hops, destinations, &latest);
    netwatch_adapter_web::MapView::new(&map, bindings, &name, &traced)
}

/// Publishes a layer's report, and returns its grade for [`publish_current`].
fn publish_layer(
    role: Role,
    report: &netwatch_host_health::LayerReport,
    disabled: &[String],
    notes: &[String],
    live: &broadcast::Sender<LiveMessage>,
    layers: &watch::Sender<std::collections::BTreeMap<String, LayerReportView>>,
) -> Health {
    use netwatch_host_health::LayerStatus;
    let view = LayerReportView {
        disabled: disabled.to_vec(),
        notes: notes.to_vec(),
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
