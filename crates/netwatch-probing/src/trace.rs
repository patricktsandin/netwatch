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
use netwatch_shared::{HopTtl, Identity, Instant, Role, TargetId};

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
pub fn traced_target(role: &Role) -> Option<TargetId> {
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

    /// Whether `target` has an episode trace held back until its running (or
    /// queued) trace finishes.
    pub fn has_pending(&self, target: TargetId) -> bool {
        self.targets.get(&target).is_some_and(|track| track.pending)
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
pub fn settle() -> Time {
    Time::new::<netwatch_shared::uom::si::time::second>(10.0)
}

/// The trace schedule, with the queue where the traces it asks for wait to run, one at
/// a time, on a path that has settled (ADR-0058, ADR-0059, ADR-0063). The daemon gives
/// it every incident and tick and the current path, starts what [`next`] hands out,
/// and says when each has [`finished`]; the rules live here:
/// - a trace starts only once the path has been the same for [`settle`], and only
///   one at a time
/// - an episode trace whose target has no incident open by then is dropped, and a
///   routine trace waits while its target has one open, letting others go ahead: run
///   during an outage, it would be taken for the normal path
/// - an episode trace goes ahead of waiting routine traces
/// - the schedule counts a queued trace as running, so it holds back the episode
///   trace of an incident on a target whose routine trace is only queued; that one
///   becomes the episode trace instead, and the one the schedule hands on once it
///   finishes is dropped, if it stored a trace
///
/// [`next`]: Self::next
/// [`finished`]: Self::finished
#[derive(Debug)]
pub struct TraceQueue {
    schedule: TraceSchedule,
    waiting: std::collections::VecDeque<TraceRequest>,
    /// The current path, and since when it has been the current one.
    path: Option<(TracePath, Instant)>,
    /// The trace running now, and whether it stood in for a held-back episode trace.
    running: Option<(TraceRequest, bool)>,
    /// Targets whose waiting routine trace became their episode trace, and those
    /// whose stand-in stored a trace, so the held-back one has already run.
    upgraded: BTreeSet<TargetId>,
    served: BTreeSet<TargetId>,
    /// Incidents open on copies of shared hops (ADR-0043): they ask for no trace,
    /// since the stream's owner's do, but their targets have incidents open.
    copies: BTreeMap<Role, TargetId>,
}

/// A trace to run now: what, and on which path.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Start {
    /// The trace.
    pub request: TraceRequest,
    /// The path it runs on, which it records.
    pub path: TracePath,
}

impl TraceQueue {
    /// A queue over `schedule`, with nothing waiting and no path yet.
    pub fn new(schedule: TraceSchedule) -> Self {
        Self {
            schedule,
            waiting: std::collections::VecDeque::new(),
            path: None,
            running: None,
            upgraded: BTreeSet::new(),
            served: BTreeSet::new(),
            copies: BTreeMap::new(),
        }
    }

    /// Takes one incident update at `now`. `copy` says it is a shared hop's copy for
    /// a target other than the stream's owner.
    pub fn incident(&mut self, incident: &Incident, copy: bool, now: Instant) {
        if copy {
            match traced_target(&incident.role) {
                Some(target) if incident.ended.is_none() => {
                    self.copies.insert(incident.role.clone(), target);
                }
                _ => {
                    self.copies.remove(&incident.role);
                }
            }
            return;
        }
        match self.schedule.incident(incident, now) {
            Some(request) => self.push(request),
            None => {
                if let Some(target) = traced_target(&incident.role)
                    && self.schedule.has_pending(target)
                {
                    self.upgrade(target);
                }
            }
        }
    }

    /// Queues every routine trace due at `now`.
    pub fn tick(&mut self, now: Instant) {
        for request in self.schedule.tick(now) {
            self.push(request);
        }
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

    /// The trace to start at `now`, if one may. The daemon must say when it has
    /// [`finished`](Self::finished), even if it couldn't start it.
    pub fn next(&mut self, now: Instant) -> Option<Start> {
        if self.running.is_some() {
            return None;
        }
        let (path, since) = self.path.clone()?;
        if now.since(since) < settle() {
            return None;
        }
        loop {
            let at = self
                .waiting
                .iter()
                .position(|r| r.kind == TraceKind::Episode || !self.open(r.target))?;
            let request = self.waiting.remove(at)?;
            let episode = request.kind == TraceKind::Episode;
            if episode && (self.served.remove(&request.target) || !self.open(request.target)) {
                self.upgraded.remove(&request.target);
                self.end(request.target, now);
                continue;
            }
            let stand_in = self.upgraded.remove(&request.target);
            self.running = Some((request, stand_in));
            return Some(Start { request, path });
        }
    }

    /// The running trace finished at `now`: `stored` if it produced a trace, not if
    /// it couldn't start, couldn't run, or was dropped.
    pub fn finished(&mut self, stored: bool, now: Instant) {
        if let Some((request, stand_in)) = self.running.take() {
            if stand_in && stored {
                self.served.insert(request.target);
            }
            self.end(request.target, now);
        }
    }

    /// Whether `target` has an incident open a trace can tell more about.
    fn open(&self, target: TargetId) -> bool {
        self.schedule.has_open(target) || self.copies.values().any(|&t| t == target)
    }

    /// Tells the schedule `target`'s trace finished, and queues what it hands on.
    fn end(&mut self, target: TargetId, now: Instant) {
        if let Some(request) = self.schedule.finished(target, now) {
            self.push(request);
        }
    }

    /// Queues a request: an episode trace ahead of the routine traces waiting.
    fn push(&mut self, request: TraceRequest) {
        let at = match request.kind {
            TraceKind::Episode => self
                .waiting
                .iter()
                .position(|waiting| waiting.kind == TraceKind::Routine)
                .unwrap_or(self.waiting.len()),
            TraceKind::Routine => self.waiting.len(),
        };
        self.waiting.insert(at, request);
    }

    /// Turns `target`'s waiting routine trace into its episode trace, ahead.
    fn upgrade(&mut self, target: TargetId) {
        let Some(at) = self
            .waiting
            .iter()
            .position(|r| r.target == target && r.kind == TraceKind::Routine)
        else {
            return;
        };
        if let Some(mut request) = self.waiting.remove(at) {
            request.kind = TraceKind::Episode;
            self.upgraded.insert(target);
            self.push(request);
        }
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
    /// The path out by the egress `egress` (an interface, by name), from the local
    /// address `local`.
    pub fn new(egress: &Identity, local: IpAddr) -> Self {
        match egress {
            Identity::Interface { name, .. } => Self(format!("{name} from {local}")),
            Identity::Address(address) => Self(format!("{address} from {local}")),
        }
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
