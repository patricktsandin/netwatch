//! Echo probe streams for targets: pins from Topology, sends and replies through
//! the sockets adapter, outcomes as History samples.

use std::collections::VecDeque;
use std::net::IpAddr;

use netwatch_adapter_sockets::EchoSocket;
use netwatch_history::{Loss, Measurement, Sample};
use netwatch_probing::{Output, ProbeOutcome, ProbeStream, StreamConfig};
use netwatch_shared::{Identity, Role};
use netwatch_topology::Binding;
use tokio::sync::{mpsc, oneshot, watch};

use crate::clock::Clock;

/// Keeps one target probed through whatever the current egress pin is. When the pin
/// changes, a new stream starts on a socket pinned to the new interface. The old
/// stream is retired, and lives on until all of its probes have their outcomes
/// (ADR-0020).
pub async fn run_target(
    role: Role,
    destination: IpAddr,
    config: StreamConfig,
    mut pins: watch::Receiver<Option<Binding>>,
    samples: mpsc::Sender<Sample>,
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
            match EchoSocket::open(destination, index.get()) {
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
                        samples.clone(),
                        clock,
                    ));
                }
                Err(error) => {
                    tracing::warn!(%error, ?role, interface = %name, "cannot open echo socket");
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
    socket: EchoSocket,
    mut stream: ProbeStream,
    mut retired: oneshot::Receiver<()>,
    samples: mpsc::Sender<Sample>,
    clock: Clock,
) {
    let mut retiring = false;
    while !stream.is_idle() {
        let outputs = tokio::select! {
            () = tokio::time::sleep_until(clock.deadline(stream.next_wake())) => stream.tick(clock.now()),
            reply = socket.recv() => match reply {
                Ok(reply) => stream.reply(reply, clock.now()),
                Err(error) => {
                    tracing::warn!(%error, ?role, "echo socket receive failed");
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
                Output::Resolved { outcome, .. } => {
                    let sample = Sample {
                        role: role.clone(),
                        epoch: pin.epoch,
                        at: clock.wall_now(),
                        measurement: measurement(outcome),
                    };
                    if samples.send(sample).await.is_err() {
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
