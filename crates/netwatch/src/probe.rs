//! Probe streams for targets and their hops: pins from Topology, sends and replies
//! through the sockets adapter, and resolutions back to the main loop, which turns
//! them into History samples.

use std::collections::{HashMap, VecDeque};
use std::net::IpAddr;

use netwatch_adapter_sockets::ProbeSocket;
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

/// Keeps one role probed (a target, or one hop toward it when `ttl` is set) through
/// whatever the current egress pin is. When the pin changes, a new stream starts on
/// a socket pinned to the new interface. The old stream is retired, and lives on
/// until all of its probes have their outcomes (ADR-0020).
pub async fn run_role(
    role: Role,
    destination: IpAddr,
    ttl: Option<HopTtl>,
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
            && let Identity::Interface { index, name } = &pin.identity
        {
            match ProbeSocket::open(destination, index.get(), ttl) {
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
                        resolutions.clone(),
                        clock,
                    ));
                }
                Err(error) => {
                    tracing::warn!(%error, ?role, interface = %name, "cannot open probe socket");
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
    socket: ProbeSocket,
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

fn measurement(outcome: ProbeOutcome) -> Measurement {
    match outcome {
        ProbeOutcome::Answered { rtt } => Measurement::RoundTrip(rtt),
        ProbeOutcome::TimedOut => Measurement::Lost(Loss::TimedOut),
        ProbeOutcome::Unreachable { code } => Measurement::Lost(Loss::Unreachable(code.0)),
        ProbeOutcome::SendFailed { .. } => Measurement::Lost(Loss::SendFailed),
    }
}
