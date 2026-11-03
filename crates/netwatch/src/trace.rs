//! Running path traces (ADR-0040): the sweep's probes through the sockets adapter,
//! then the router names of whoever answered. The trace schedule decides when.

use std::net::IpAddr;
use std::num::NonZeroU8;
use std::time::Duration;

use netwatch_adapter_sockets::ProbeSocket;
use netwatch_adapter_sqlite::StoredTrace;
use netwatch_probing::trace::{TracePath, TraceProbe, TraceRequest, assemble};
use netwatch_probing::{ProbeIndex, ProbeSequence, ReplyKind};
use netwatch_shared::{HopTtl, Identity, Role, TargetId};
use netwatch_topology::Binding;
use tokio::sync::mpsc;

use crate::clock::Clock;
use crate::names::Names;

/// The TTLs swept. Hops 1–3 have their own series (ADR-0025), and probing them
/// again would share the gateway's ICMP rate limit with those (ADR-0026).
const FIRST_TTL: u8 = 4;
const LAST_TTL: u8 = 30;
const ROUNDS: u16 = 3;
const ROUND_EVERY: Duration = Duration::from_secs(1);
const TIMEOUT: Duration = Duration::from_secs(2);

/// A trace that has finished: the stored result, or `None` if it couldn't run.
#[derive(Debug)]
pub struct Finished {
    /// Whose path was traced.
    pub target: TargetId,
    /// What it found, or `None` if it couldn't run (no interface to pin to, or a
    /// socket that wouldn't open) or was dropped.
    pub trace: Option<StoredTrace>,
}

/// Runs path traces. One runs at a time, which the trace queue sees to (ADR-0059):
/// probes of concurrent traces would delay one another's answers in this process and
/// inflate their RTTs, and would reach the same routers in bursts that their ICMP rate
/// limits drop.
#[derive(Clone)]
pub struct Tracer {
    names: Names,
    clock: Clock,
    done: mpsc::Sender<Finished>,
}

impl Tracer {
    /// Names responders with `names`, and reports every finished trace to `done`.
    pub fn new(names: Names, clock: Clock, done: mpsc::Sender<Finished>) -> Self {
        Self { names, clock, done }
    }

    /// Runs `request` toward `destination` now, over `pin`, recording `path` as the
    /// path it ran on (ADR-0056), and reports to `done` whether or not it could run.
    pub fn start(&self, request: TraceRequest, destination: IpAddr, pin: Binding, path: TracePath) {
        let tracer = self.clone();
        tokio::spawn(async move {
            let trace = run(request, destination, pin, path, &tracer.names, tracer.clock).await;
            let _ = tracer
                .done
                .send(Finished {
                    target: request.target,
                    trace,
                })
                .await;
        });
    }
}

async fn run(
    request: TraceRequest,
    destination: IpAddr,
    pin: Binding,
    path: TracePath,
    names: &Names,
    clock: Clock,
) -> Option<StoredTrace> {
    let Identity::Interface { index, .. } = pin.identity else {
        return None;
    };
    let at = clock.wall_now();
    // Every socket opens before any sweep starts, and the sweeps are dropped, and so
    // aborted, with this trace if it gives up: no probe of it outlives it, to overlap
    // the next trace (ADR-0059).
    let mut sockets = Vec::new();
    for raw in FIRST_TTL..=LAST_TTL {
        let ttl = HopTtl::new(NonZeroU8::new(raw)?);
        match ProbeSocket::open(destination, index.get(), Some(ttl)) {
            Ok(socket) => sockets.push((socket, ttl)),
            Err(error) => {
                tracing::warn!(%error, %destination, "cannot open a path trace socket");
                return None;
            }
        }
    }
    let mut sweeps = tokio::task::JoinSet::new();
    for (socket, ttl) in sockets {
        sweeps.spawn(sweep(socket, ttl, clock));
    }
    let mut probes = Vec::new();
    while let Some(swept) = sweeps.join_next().await {
        probes.extend(swept.ok()?);
    }
    let trace = assemble(&probes);
    // Every responder is named at once: one after another, a trace through a
    // dozen routers with an unanswering resolver would hold the queue for minutes.
    let mut lookups = tokio::task::JoinSet::new();
    let responders: std::collections::BTreeSet<IpAddr> = trace
        .hops
        .iter()
        .flat_map(|h| h.responders.iter().copied())
        .collect();
    for address in responders {
        let names = names.clone();
        lookups.spawn(async move { (address, names.name(address).await) });
    }
    let mut named = std::collections::BTreeMap::new();
    while let Some(looked_up) = lookups.join_next().await {
        if let Ok((address, Some(name))) = looked_up {
            named.insert(address, name);
        }
    }
    Some(StoredTrace {
        role: Role::Target(request.target),
        at,
        kind: request.kind,
        destination,
        trace,
        names: named,
        path: Some(path),
    })
}

/// Every round's probe at one TTL. Rounds are sent on their schedule whatever is
/// answered (a fixed cadence, ADR-0004), and each has [`TIMEOUT`] from its own send
/// to be answered. Each round's probe has the round as its sequence number, so a
/// late answer to an earlier round is never taken for another's (ADR-0023).
async fn sweep(socket: ProbeSocket, ttl: HopTtl, clock: Clock) -> Vec<TraceProbe> {
    let start = tokio::time::Instant::now();
    let unanswered = || TraceProbe {
        ttl,
        responder: None,
        rtt: None,
        ends_path: false,
    };
    let mut probes: Vec<TraceProbe> = (0..ROUNDS).map(|_| unanswered()).collect();
    let mut sent = Vec::with_capacity(usize::from(ROUNDS));
    // The last round's answer may come until TIMEOUT after it was really sent,
    // however late the runtime sent it.
    let mut last_sent = start;
    let mut receiving = true;
    loop {
        let next = u16::try_from(sent.len()).unwrap_or(ROUNDS);
        let wake = if next < ROUNDS {
            start + ROUND_EVERY * u32::from(next)
        } else {
            last_sent + TIMEOUT
        };
        tokio::select! {
            () = tokio::time::sleep_until(wake) => {
                if next == ROUNDS {
                    break;
                }
                // A send that fails leaves its round unanswered.
                last_sent = tokio::time::Instant::now();
                let ok = socket.send(ProbeSequence::new(next), ProbeIndex::new(u64::from(next))).is_ok();
                sent.push(ok.then(|| clock.now()));
            }
            received = socket.recv(), if receiving => match received {
                Ok(received) => {
                    let round = usize::from(received.reply.sequence.get());
                    let at = sent.get(round).copied().flatten();
                    if let (Some(at), Some(probe)) = (at, probes.get_mut(round))
                        && probe.rtt.is_none()
                        && clock.now().since(at).get::<netwatch_shared::uom::si::time::second>()
                            <= TIMEOUT.as_secs_f64()
                    {
                        probe.responder = received.responder;
                        probe.rtt = Some(clock.now().since(at));
                        probe.ends_path = matches!(
                            received.reply.kind,
                            ReplyKind::Echo | ReplyKind::Unreachable(_)
                        );
                    }
                }
                // The socket can't be read: the rounds left go unanswered.
                Err(_) => receiving = false,
            },
        }
        if sent.len() == usize::from(ROUNDS) && probes.iter().all(|p| p.rtt.is_some()) {
            break;
        }
    }
    probes
}

/// A trace's event detail, for the target's row: `routine path trace: 11 hops to
/// 1.1.1.1` or `episode path trace: no answer from hop 9 on`.
pub fn summary(trace: &StoredTrace) -> String {
    let kind = match trace.kind {
        netwatch_probing::trace::TraceKind::Routine => "routine",
        netwatch_probing::trace::TraceKind::Episode => "episode",
    };
    let end = match (
        trace.trace.reached,
        trace.trace.hops.last(),
        trace.trace.silent_from,
    ) {
        // Answered at the first TTL traced, it may be nearer: hops before it aren't swept.
        (true, Some(last), _) if last.ttl.get().get() == FIRST_TTL => {
            format!("{} within {FIRST_TTL} hops", trace.destination)
        }
        (true, Some(last), _) => format!("{} hops to {}", last.ttl.get(), trace.destination),
        (_, _, Some(from)) => format!("no answer from hop {} on", from.get()),
        _ => format!("toward {}", trace.destination),
    };
    format!("{kind} path trace: {end}")
}
