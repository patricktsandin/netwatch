//! Probe streams for targets and their hops: pins from Topology, sends and replies
//! through the sockets adapter, and resolutions back to the main loop, which turns
//! them into History samples.

use std::collections::{HashMap, VecDeque};
use std::net::IpAddr;

use netwatch_adapter_sockets::{DnsSocket, ProbeSocket, Received};
use netwatch_history::{Loss, Measurement};
use netwatch_probing::{Output, ProbeOutcome, ProbeStream, StreamConfig};
use netwatch_shared::{HopTtl, Identity, Role};
use netwatch_topology::Binding;
use tokio::sync::{mpsc, oneshot, watch};

use crate::clock::Clock;

/// One probe's outcome, with the pin it traveled under and who answered it.
#[derive(Debug)]
pub struct Resolution {
    pub role: Role,
    pub pin: Binding,
    pub responder: Option<IpAddr>,
    pub measurement: Measurement,
}

/// What a stream sends.
#[derive(Debug, Clone, Copy)]
pub enum ProbeKind {
    /// ICMP echo; with a TTL, a hop probe.
    Echo(Option<HopTtl>),
    /// A DNS query for `. IN NS`.
    Dns,
}

/// A stream's socket, whichever kind it is.
enum Socket {
    Icmp(ProbeSocket),
    Dns(DnsSocket),
}

impl Socket {
    fn open(
        kind: ProbeKind,
        destination: IpAddr,
        pin: std::num::NonZeroU32,
    ) -> std::io::Result<Self> {
        Ok(match kind {
            ProbeKind::Echo(ttl) => Socket::Icmp(ProbeSocket::open(destination, pin, ttl)?),
            ProbeKind::Dns => Socket::Dns(DnsSocket::open(destination, pin)?),
        })
    }

    fn send(
        &self,
        sequence: netwatch_probing::ProbeSequence,
        index: netwatch_probing::ProbeIndex,
    ) -> Result<(), netwatch_probing::SendFailure> {
        match self {
            Socket::Icmp(socket) => socket.send(sequence, index),
            Socket::Dns(socket) => socket.send(sequence),
        }
    }

    async fn recv(&self) -> std::io::Result<Received> {
        match self {
            Socket::Icmp(socket) => socket.recv().await,
            Socket::Dns(socket) => socket.recv().await,
        }
    }
}

/// Where a probe task reports: every probe's resolution, and any source it finds it
/// can't run.
#[derive(Debug, Clone)]
pub struct Reports {
    pub resolutions: mpsc::Sender<Resolution>,
    pub disabled: mpsc::UnboundedSender<String>,
}

/// Keeps one role probed (a target, or one hop toward it when `ttl` is set) through
/// whatever the current egress pin is. When the pin changes, a new stream starts on
/// a socket pinned to the new interface. The old stream is retired, and lives on
/// until all of its probes have their outcomes (ADR-0020).
pub async fn run_role(
    role: Role,
    destination: IpAddr,
    kind: ProbeKind,
    config: StreamConfig,
    mut pins: watch::Receiver<Option<Binding>>,
    reports: Reports,
    clock: Clock,
) {
    let mut retire_current: Option<oneshot::Sender<()>> = None;
    loop {
        let pin = pins.borrow_and_update().clone();
        if let Some(retire) = retire_current.take() {
            let _ = retire.send(());
        }
        if let Some(pin) = pin
            && let Identity::Interface { index, name } = &pin.identity
        {
            match Socket::open(kind, destination, index.get()) {
                Ok(socket) => {
                    let (retire, retired) = oneshot::channel();
                    retire_current = Some(retire);
                    let stream = ProbeStream::new(config, clock.now());
                    tokio::spawn(run_stream(
                        role.clone(),
                        pin.clone(),
                        socket,
                        stream,
                        retired,
                        reports.resolutions.clone(),
                        clock,
                    ));
                }
                Err(error) => {
                    tracing::warn!(%error, ?role, interface = %name, "cannot open probe socket");
                    // A probe kind that can't run is a disabled source, shown in the UI,
                    // never just a missing row (ADR-0007). One message per kind and
                    // cause, however many roles hit it.
                    let _ = reports.disabled.send(disabled_reason(kind, &error));
                }
            }
        }
        if pins.changed().await.is_err() {
            return;
        }
    }
}

async fn run_stream(
    role: Role,
    pin: Binding,
    socket: Socket,
    mut stream: ProbeStream,
    mut retired: oneshot::Receiver<()>,
    resolutions: mpsc::Sender<Resolution>,
    clock: Clock,
) {
    let mut retiring = false;
    // Who answered each probe, kept until its resolution is reported.
    let mut responders = HashMap::new();
    while !stream.is_idle() {
        let outputs = tokio::select! {
            () = tokio::time::sleep_until(clock.deadline(stream.next_wake())) => stream.tick(clock.now()),
            received = socket.recv() => match received {
                Ok(received) => {
                    let outputs = stream.reply(received.reply, clock.now());
                    for output in &outputs {
                        if let Output::Resolved { index, .. } = output {
                            responders.insert(*index, received.responder);
                        }
                    }
                    outputs
                }
                Err(error) => {
                    tracing::warn!(%error, ?role, "probe socket receive failed");
                    Vec::new()
                }
            },
            _ = &mut retired, if !retiring => {
                retiring = true;
                stream.retire();
                Vec::new()
            }
        };
        // Outputs must be handled in order, and a send failure appends its own.
        let mut pending = VecDeque::from(outputs);
        while let Some(output) = pending.pop_front() {
            match output {
                Output::Send { index, sequence } => {
                    if let Err(cause) = socket.send(sequence, index) {
                        pending.extend(stream.send_failed(index, cause));
                    }
                }
                Output::Resolved { index, outcome } => {
                    let resolution = Resolution {
                        role: role.clone(),
                        pin: pin.clone(),
                        responder: responders.remove(&index).flatten(),
                        measurement: measurement(outcome),
                    };
                    if resolutions.send(resolution).await.is_err() {
                        return;
                    }
                }
            }
        }
    }
}

/// Why a probe kind can't run, for the kernel layer's list of disabled sources.
fn disabled_reason(kind: ProbeKind, error: &std::io::Error) -> String {
    match kind {
        ProbeKind::Echo(_) if error.kind() == std::io::ErrorKind::PermissionDenied => {
            "ICMP probes: permission denied (the service's group must be within \
             net.ipv4.ping_group_range)"
                .to_owned()
        }
        ProbeKind::Echo(_) => format!("ICMP probes: {error}"),
        ProbeKind::Dns => format!("DNS probes: {error}"),
    }
}

fn measurement(outcome: ProbeOutcome) -> Measurement {
    match outcome {
        ProbeOutcome::Answered { rtt } => Measurement::RoundTrip(rtt),
        ProbeOutcome::TimedOut => Measurement::Lost(Loss::TimedOut),
        ProbeOutcome::Unreachable { code } => Measurement::Lost(Loss::Unreachable(code.0)),
        ProbeOutcome::TimeExceeded => Measurement::Lost(Loss::TimeExceeded),
        ProbeOutcome::Refused { code } => Measurement::Lost(Loss::Refused(code)),
        ProbeOutcome::UnexpectedResponse { code } => {
            Measurement::Lost(Loss::UnexpectedResponse(code))
        }
        ProbeOutcome::SendFailed { .. } => Measurement::Lost(Loss::SendFailed),
    }
}

/// Keeps one HTTPS target probed through the current egress pin. Each probe is a
/// request, run as its own task. The stream still owns scheduling and timeouts, and a
/// request that is still running when its probe times out is aborted (ADR-0028).
pub async fn run_https_role(
    role: Role,
    target: netwatch_adapter_sockets::http::HttpsTarget,
    config: StreamConfig,
    mut pins: watch::Receiver<Option<Binding>>,
    resolutions: mpsc::Sender<Resolution>,
    clock: Clock,
) {
    let mut retire_current: Option<oneshot::Sender<()>> = None;
    loop {
        let pin = pins.borrow_and_update().clone();
        if let Some(retire) = retire_current.take() {
            let _ = retire.send(());
        }
        if let Some(pin) = pin
            && let Identity::Interface { name, .. } = &pin.identity
        {
            let (retire, retired) = oneshot::channel();
            retire_current = Some(retire);
            tokio::spawn(run_https_stream(
                role.clone(),
                target.clone(),
                *name,
                pin.clone(),
                ProbeStream::new(config, clock.now()),
                retired,
                resolutions.clone(),
                clock,
            ));
        }
        if pins.changed().await.is_err() {
            return;
        }
    }
}

#[allow(clippy::too_many_arguments)] // One task's whole context; a struct would only rename it.
async fn run_https_stream(
    role: Role,
    target: netwatch_adapter_sockets::http::HttpsTarget,
    interface: netwatch_shared::InterfaceName,
    pin: Binding,
    mut stream: ProbeStream,
    mut retired: oneshot::Receiver<()>,
    resolutions: mpsc::Sender<Resolution>,
    clock: Clock,
) {
    use netwatch_adapter_sockets::http::{self, HttpsResult};
    use netwatch_probing::{ProbeIndex, ProbeSequence, Reply, ReplyKind, SendFailure};

    let (done_tx, mut done) = mpsc::channel::<(ProbeIndex, ProbeSequence, HttpsResult)>(16);
    let mut running: HashMap<ProbeIndex, tokio::task::JoinHandle<()>> = HashMap::new();
    let mut retiring = false;
    while !stream.is_idle() {
        let outputs = tokio::select! {
            () = tokio::time::sleep_until(clock.deadline(stream.next_wake())) => stream.tick(clock.now()),
            Some((index, sequence, result)) = done.recv() => {
                running.remove(&index);
                match result {
                    HttpsResult::Answered(phases) => {
                        tracing::debug!(?role, ?phases, "https phases");
                        let reply = Reply { sequence, token: Some(index), kind: ReplyKind::Echo };
                        stream.reply(reply, clock.now())
                    }
                    HttpsResult::Unexpected { status } => {
                        let reply = Reply { sequence, token: Some(index), kind: ReplyKind::Unexpected(status) };
                        stream.reply(reply, clock.now())
                    }
                    HttpsResult::Failed(error) => {
                        tracing::debug!(%error, ?role, "https probe failed");
                        let cause = error.raw_os_error().map_or(SendFailure::Other(0), SendFailure::Other);
                        stream.send_failed(index, cause)
                    }
                }
            }
            _ = &mut retired, if !retiring => {
                retiring = true;
                stream.retire();
                Vec::new()
            }
        };
        for output in outputs {
            match output {
                netwatch_probing::Output::Send { index, sequence } => {
                    let done_tx = done_tx.clone();
                    let target = target.clone();
                    running.insert(
                        index,
                        tokio::spawn(async move {
                            let result = http::probe(target, interface).await;
                            let _ = done_tx.send((index, sequence, result)).await;
                        }),
                    );
                }
                netwatch_probing::Output::Resolved { index, outcome } => {
                    if let Some(request) = running.remove(&index) {
                        request.abort();
                    }
                    let resolution = Resolution {
                        role: role.clone(),
                        pin: pin.clone(),
                        responder: None,
                        measurement: measurement(outcome),
                    };
                    if resolutions.send(resolution).await.is_err() {
                        return;
                    }
                }
            }
        }
    }
}
