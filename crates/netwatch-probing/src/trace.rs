//! Path traces (ADR-0040): one-off TTL sweeps toward a target, beyond the hops
//! measured continuously, and the schedule that decides when they run.
//!
//! [`TraceSchedule`] is the design in `specs/traces.qnt`, held to it by trace
//! conformance. It hides the cooldown, the routine interval, and the bookkeeping of
//! which incidents are already open, so the daemon only feeds it incident updates,
//! ticks, and finished traces, and runs whatever it asks for. [`assemble`] turns a
//! sweep's probes into the hops a report shows.

use std::collections::{BTreeMap, BTreeSet};
use std::net::IpAddr;

use netwatch_shared::uom::si::f64::Time;
use netwatch_shared::{HopTtl, Instant, Role, TargetId};

use crate::incident::Incident;

/// Why a trace runs.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum TraceKind {
    /// At startup and every few hours: the path as it normally is (glossary:
    /// *routine trace*).
    Routine,
    /// Because an incident opened beyond hop 1 (glossary: *episode trace*).
    Episode,
}

/// A trace the daemon should run now.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TraceRequest {
    /// Whose path.
    pub target: TargetId,
    /// Why.
    pub kind: TraceKind,
}

/// Decides when each target is traced (glossary: *trace schedule*).
///
/// - Every target gets a routine trace when the schedule starts, and every
///   `routine_every` after the last one started.
/// - An incident opening on a target, or on one of its hops beyond hop 1, gets an
///   episode trace, unless one started less than `cooldown` before. If a trace of
///   that target is running, the episode trace is requested when it finishes.
/// - A target never has two traces running.
#[derive(Debug)]
pub struct TraceSchedule {
    cooldown: Time,
    routine_every: Time,
    targets: BTreeMap<TargetId, Track>,
}

#[derive(Debug)]
struct Track {
    next_routine: Instant,
    last_episode: Option<Instant>,
    running: bool,
    pending: bool,
    open: BTreeSet<Role>,
}

/// The target whose path a trace measures, if an incident on `role` is one a trace
/// can tell more about: the target itself, or a hop beyond hop 1. Hop 1 and the
/// host's own layers are before the part of the path a trace measures.
fn traced_target(role: &Role) -> Option<TargetId> {
    match role {
        Role::Target(target) => Some(*target),
        Role::Hop { target, ttl } if ttl.get().get() > 1 => Some(*target),
        _ => None,
    }
}

impl TraceSchedule {
    /// A schedule for `targets` (the traceable ones), each due a routine trace at
    /// `now`.
    pub fn new(
        targets: impl IntoIterator<Item = TargetId>,
        cooldown: Time,
        routine_every: Time,
        now: Instant,
    ) -> Self {
        Self {
            cooldown,
            routine_every,
            targets: targets
                .into_iter()
                .map(|t| {
                    (
                        t,
                        Track {
                            next_routine: now,
                            last_episode: None,
                            running: false,
                            pending: false,
                            open: BTreeSet::new(),
                        },
                    )
                })
                .collect(),
        }
    }

    /// Takes one incident update, as the incident tracker returned it, at `now`.
    /// Returns the episode trace to run, if this opened an incident that wants one
    /// and nothing of that target is running.
    pub fn incident(&mut self, incident: &Incident, now: Instant) -> Option<TraceRequest> {
        let target = traced_target(&incident.role)?;
        let cooldown = self.cooldown;
        let track = self.targets.get_mut(&target)?;
        if incident.ended.is_some() {
            track.open.remove(&incident.role);
            return None;
        }
        if !track.open.insert(incident.role.clone()) {
            return None;
        }
        let cool = track
            .last_episode
            .is_none_or(|last| now.since(last) >= cooldown);
        if !cool {
            None
        } else if track.running {
            track.pending = true;
            None
        } else {
            track.running = true;
            track.last_episode = Some(now);
            Some(TraceRequest {
                target,
                kind: TraceKind::Episode,
            })
        }
    }

    /// Returns every routine trace due at `now`, in target order.
    pub fn tick(&mut self, now: Instant) -> Vec<TraceRequest> {
        let every = self.routine_every;
        self.targets
            .iter_mut()
            .filter(|(_, track)| !track.running && now >= track.next_routine)
            .map(|(&target, track)| {
                track.running = true;
                track.next_routine = now.after(every);
                TraceRequest {
                    target,
                    kind: TraceKind::Routine,
                }
            })
            .collect()
    }

    /// Whether `target` has an incident open that a trace can tell more about (on the
    /// target, or on a hop beyond hop 1).
    pub fn has_open(&self, target: TargetId) -> bool {
        self.targets
            .get(&target)
            .is_some_and(|track| !track.open.is_empty())
    }

    /// Records that `target`'s running trace finished at `now`. Returns the episode
    /// trace that was waiting for it, if any.
    pub fn finished(&mut self, target: TargetId, now: Instant) -> Option<TraceRequest> {
        let track = self.targets.get_mut(&target)?;
        if !track.running {
            return None;
        }
        let again = std::mem::take(&mut track.pending);
        track.running = again;
        again.then(|| {
            track.last_episode = Some(now);
            TraceRequest {
                target,
                kind: TraceKind::Episode,
            }
        })
    }
}

/// How long the path must have been the same before a trace runs on it (ADR-0058).
pub const SETTLE_S: f64 = 10.0;

/// The traces the schedule asked for, waiting to run, one at a time, on a path that
/// has settled (ADR-0058, ADR-0059). A trace started without the whole path known
/// couldn't run, or would record a path nothing matches; one started as a link
/// recovers would trace the recovery; and one started long after it was asked for
/// may no longer be wanted. So the checks are made when a trace starts.
#[derive(Debug, Default)]
pub struct TraceQueue {
    waiting: std::collections::VecDeque<TraceRequest>,
    /// The current path, and since when it has been the current one.
    path: Option<(TracePath, Instant)>,
    running: bool,
}

/// What to do with the next request in a [`TraceQueue`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Next {
    /// Run it now, on this path.
    Run(TraceRequest, TracePath),
    /// It's an episode trace whose target has nothing open any more: it was for an
    /// outage that is over. Tell the schedule it finished.
    Drop(TraceRequest),
}

impl TraceQueue {
    /// Adds a request the schedule asked for.
    pub fn push(&mut self, request: TraceRequest) {
        self.waiting.push_back(request);
    }

    /// Adds every request given, such as the one [`TraceSchedule::finished`] hands
    /// on, if any.
    pub fn push_all(&mut self, requests: impl IntoIterator<Item = TraceRequest>) {
        self.waiting.extend(requests);
    }

    /// The current path at `now`, if the whole of it is known. Any change, even
    /// straight from one path to another, starts the settling afresh.
    pub fn path(&mut self, path: Option<&TracePath>, now: Instant) {
        match (path, &self.path) {
            (Some(path), Some((current, _))) if path == current => {}
            (Some(path), _) => self.path = Some((path.clone(), now)),
            (None, _) => self.path = None,
        }
    }

    /// The next request at `now`, if none is running and the path has settled.
    /// `open` says whether a target still has an incident open, as
    /// [`TraceSchedule::has_open`] does.
    pub fn next(&mut self, now: Instant, open: impl Fn(TargetId) -> bool) -> Option<Next> {
        if self.running {
            return None;
        }
        let (path, since) = self.path.as_ref()?;
        if now
            .since(*since)
            .get::<netwatch_shared::uom::si::time::second>()
            < SETTLE_S
        {
            return None;
        }
        let request = self.waiting.pop_front()?;
        if request.kind == TraceKind::Episode && !open(request.target) {
            return Some(Next::Drop(request));
        }
        self.running = true;
        Some(Next::Run(request, path.clone()))
    }

    /// The running trace finished, whether or not it could run.
    pub fn finished(&mut self) {
        self.running = false;
    }
}

/// What came back for one probe of a sweep.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct TraceProbe {
    /// The TTL it was sent with.
    pub ttl: HopTtl,
    /// Who answered, if anyone did.
    pub responder: Option<IpAddr>,
    /// Its round trip, if it was answered.
    pub rtt: Option<Time>,
    /// Whether the answer ends the path: the destination replied, or a router said
    /// the destination is unreachable.
    pub ends_path: bool,
}

/// One TTL of a trace.
#[derive(Debug, Clone, PartialEq)]
pub struct TraceHop {
    /// The TTL.
    pub ttl: HopTtl,
    /// Every address that answered at this TTL, in order of first answer. More than
    /// one means load balancing, or a route change during the trace.
    pub responders: Vec<IpAddr>,
    /// Probes sent at this TTL.
    pub sent: u32,
    /// Of those, answered.
    pub answered: u32,
    /// The fastest answer.
    pub best: Option<Time>,
    /// The median answer.
    pub median: Option<Time>,
}

/// The path a trace ran on (glossary: *trace path*): the egress interface and the
/// local address, such as `wlan0 from 192.168.1.20` (ADR-0056). Another network is
/// another path, even on the same interface. Its text is what is stored, and two
/// paths are the same exactly when their texts are.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct TracePath(String);

impl TracePath {
    /// The path out by `interface`, from the local address `local`.
    pub fn new(interface: &str, local: IpAddr) -> Self {
        Self(format!("{interface} from {local}"))
    }

    /// A path as it was stored.
    pub fn from_stored(text: String) -> Self {
        Self(text)
    }

    /// The path's text, as it is stored and shown.
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

/// A trace's hops, in TTL order (glossary: *path trace*).
#[derive(Debug, Clone, PartialEq)]
pub struct PathTrace {
    /// Every TTL up to the one that ended the path, or up to the last one anyone
    /// answered at.
    pub hops: Vec<TraceHop>,
    /// Whether the destination (or a router refusing the way to it) answered.
    pub reached: bool,
    /// The first TTL from which no hop answered, if the path didn't end.
    pub silent_from: Option<HopTtl>,
}

/// Turns a sweep's probes, in any order, into a [`PathTrace`].
pub fn assemble(probes: &[TraceProbe]) -> PathTrace {
    let mut by_ttl: BTreeMap<HopTtl, Vec<&TraceProbe>> = BTreeMap::new();
    for probe in probes {
        by_ttl.entry(probe.ttl).or_default().push(probe);
    }
    let end = by_ttl
        .iter()
        .find(|(_, probes)| probes.iter().any(|p| p.ends_path))
        .map(|(&ttl, _)| ttl);
    let last_answered = by_ttl
        .iter()
        .rev()
        .find(|(_, probes)| probes.iter().any(|p| p.responder.is_some()))
        .map(|(&ttl, _)| ttl);
    let keep_up_to = end.or(last_answered);
    let silent_from = match (end, by_ttl.keys().next_back()) {
        (None, Some(&highest)) if keep_up_to != Some(highest) => by_ttl
            .keys()
            .find(|&&ttl| keep_up_to.is_none_or(|kept| ttl > kept))
            .copied(),
        _ => None,
    };
    let hops = by_ttl
        .into_iter()
        .filter(|(ttl, _)| keep_up_to.is_some_and(|kept| *ttl <= kept))
        .map(|(ttl, probes)| {
            let mut responders = Vec::new();
            for address in probes.iter().filter_map(|p| p.responder) {
                if !responders.contains(&address) {
                    responders.push(address);
                }
            }
            let mut rtts: Vec<Time> = probes.iter().filter_map(|p| p.rtt).collect();
            rtts.sort_by(|a, b| a.value.total_cmp(&b.value));
            TraceHop {
                ttl,
                responders,
                sent: u32::try_from(probes.len()).unwrap_or(u32::MAX),
                answered: u32::try_from(rtts.len()).unwrap_or(u32::MAX),
                best: rtts.first().copied(),
                median: rtts.get(rtts.len() / 2).copied(),
            }
        })
        .collect();
    PathTrace {
        hops,
        reached: end.is_some(),
        silent_from,
    }
}
