// -*- mode: Bluespec; -*-
/**
 * # The trace schedule: when each target's path is traced
 *
 * This spec is the design of `netwatch_probing::trace::TraceSchedule`
 * (ADR-0040). Terms are defined in `docs/glossary.md`.
 *
 * ## What is modeled
 *
 * Two traceable targets. Each has three kinds of role an incident can open on:
 * `near` (hop 1, shared by every target, or the host's layers), `hop` (a hop
 * beyond hop 1), and `target` (the target itself). Time advances in whole ticks.
 * Within a tick, any number of these happen, in any order:
 *
 * - **An incident update**: an incident on one of a target's roles opens (or is
 *   updated while open), or closes.
 * - **A tick**: the schedule starts every routine trace that is due.
 * - **A finish**: a target's running trace finishes.
 *
 * Each step returns `started`: the traces it started, in target order.
 *
 * - A routine trace is due at time 0 and every `ROUTINE_EVERY` after the last
 *   one started. A due trace waits while another trace of its target runs.
 * - An incident opening on a `hop` or `target` role starts an episode trace,
 *   unless one started less than `COOLDOWN` ago. If a trace of the target is
 *   running, the episode trace starts when it finishes.
 * - Incidents on `near` roles never start a trace: the problem is before the part
 *   of the path a trace measures.
 *
 * ## How the Rust code is held to this spec
 *
 * `crates/netwatch-probing/tests/trace_conformance.rs` replays every step against
 * the Rust schedule and requires identical `started` lists.
 */
module traces {

  pure val TARGETS = Set(0, 1)
  pure val ROLES = Set("near", "hop", "target")
  pure val MAX_TIME = 4
  /// The shortest time between two episode traces of one target: 10 minutes in
  /// the daemon.
  pure val COOLDOWN = 2
  /// How often routine traces run: every 6 hours in the daemon.
  pure val ROUTINE_EVERY = 3
  /// "No time", for optional times. Times are never negative.
  pure val NONE = -1

  pure def triggers(r: str): bool = r != "near"
  pure def cooledDown(last: int, t: int): bool = last == NONE or t - last >= COOLDOWN

  type Start = { target: int, kind: str }
  type Past = { target: int, kind: str, at: int }

  // ---------------------------------------------------------------------------
  // The schedule.
  var now: int
  /// When each target's next routine trace is due.
  var nextRoutine: int -> int
  /// When each target's latest episode trace started.
  var lastEpisode: int -> int
  /// Whether each target has a trace running.
  var running: int -> bool
  /// Whether an episode trace waits for the running trace to finish.
  var pending: int -> bool
  /// The triggering roles of each target with an incident open.
  var open: int -> Set[str]
  /// What the latest step started.
  var started: List[Start]

  // ---------------------------------------------------------------------------
  // Ghost state, for the invariants only.
  /// Every trace started, oldest first.
  var history: List[Past]
  /// Traces started and not yet finished, by target.
  var inFlight: int -> int
  /// Whether a target has had an incident open with no episode trace started in
  /// the `COOLDOWN` before it, and none started since.
  var owed: int -> bool

  action init = all {
    now' = 0,
    nextRoutine' = TARGETS.mapBy(_ => 0),
    lastEpisode' = TARGETS.mapBy(_ => NONE),
    running' = TARGETS.mapBy(_ => false),
    pending' = TARGETS.mapBy(_ => false),
    open' = TARGETS.mapBy(_ => Set()),
    started' = [],
    history' = [],
    inFlight' = TARGETS.mapBy(_ => 0),
    owed' = TARGETS.mapBy(_ => false),
  }

  action advance = all {
    now < MAX_TIME,
    now' = now + 1,
    started' = [],
    nextRoutine' = nextRoutine, lastEpisode' = lastEpisode, running' = running,
    pending' = pending, open' = open, history' = history, inFlight' = inFlight,
    owed' = owed,
  }

  /// An incident on target `t`'s role `r` is open after this update (`isOpen`),
  /// or has closed.
  action incident = {
    nondet t = TARGETS.oneOf()
    nondet r = ROLES.oneOf()
    nondet isOpen = Bool.oneOf()
    val opening = triggers(r) and isOpen and not(open.get(t).contains(r))
    val cool = cooledDown(lastEpisode.get(t), now)
    val startNow = opening and cool and not(running.get(t))
    val recent = history.select(p =>
      p.target == t and p.kind == "episode" and now - p.at < COOLDOWN).length() > 0
    all {
      open' = if (not(triggers(r))) open
        else if (isOpen) open.set(t, open.get(t).union(Set(r)))
        else open.set(t, open.get(t).exclude(Set(r))),
      running' = if (startNow) running.set(t, true) else running,
      lastEpisode' = if (startNow) lastEpisode.set(t, now) else lastEpisode,
      pending' = if (opening and cool and running.get(t)) pending.set(t, true) else pending,
      started' = if (startNow) [{ target: t, kind: "episode" }] else [],
      history' = if (startNow) history.append({ target: t, kind: "episode", at: now }) else history,
      inFlight' = if (startNow) inFlight.set(t, inFlight.get(t) + 1) else inFlight,
      owed' = if (startNow) owed.set(t, false)
        else if (opening and not(recent)) owed.set(t, true)
        else owed,
      now' = now, nextRoutine' = nextRoutine,
    }
  }

  /// Starts every routine trace that is due.
  action tick = {
    val due = TARGETS.filter(t => not(running.get(t)) and now >= nextRoutine.get(t))
    val inOrder = range(0, 2).select(t => due.contains(t))
    all {
      running' = TARGETS.mapBy(t => running.get(t) or due.contains(t)),
      nextRoutine' = TARGETS.mapBy(t =>
        if (due.contains(t)) now + ROUTINE_EVERY else nextRoutine.get(t)),
      started' = inOrder.foldl([], (acc, t) => acc.append({ target: t, kind: "routine" })),
      history' = inOrder.foldl(history, (acc, t) =>
        acc.append({ target: t, kind: "routine", at: now })),
      inFlight' = TARGETS.mapBy(t => inFlight.get(t) + (if (due.contains(t)) 1 else 0)),
      now' = now, lastEpisode' = lastEpisode, pending' = pending, open' = open,
      owed' = owed,
    }
  }

  /// Target `t`'s running trace finishes; a waiting episode trace starts.
  action finish = {
    nondet t = TARGETS.oneOf()
    val again = pending.get(t)
    all {
      running.get(t),
      running' = running.set(t, again),
      pending' = pending.set(t, false),
      lastEpisode' = if (again) lastEpisode.set(t, now) else lastEpisode,
      started' = if (again) [{ target: t, kind: "episode" }] else [],
      history' = if (again) history.append({ target: t, kind: "episode", at: now }) else history,
      inFlight' = inFlight.set(t, inFlight.get(t) - 1 + (if (again) 1 else 0)),
      owed' = if (again) owed.set(t, false) else owed,
      now' = now, nextRoutine' = nextRoutine, open' = open,
    }
  }

  action step = any { advance, incident, tick, finish }

  // ---------------------------------------------------------------------------
  // Invariants.

  /// A target never has two traces running at once.
  val oneAtATime: bool =
    TARGETS.forall(t => inFlight.get(t) <= 1 and ((inFlight.get(t) == 1) == running.get(t)))

  pure def spaced(h: List[Past], kind: str, gap: int): bool =
    h.indices().forall(i => h.indices().forall(j =>
      (i < j and h[i].target == h[j].target and h[i].kind == kind and h[j].kind == kind)
        implies h[j].at - h[i].at >= gap))

  /// Episode traces of a target start at least `COOLDOWN` apart.
  val episodesSpaced: bool = spaced(history, "episode", COOLDOWN)

  /// Routine traces of a target start at least `ROUTINE_EVERY` apart.
  val routinesSpaced: bool = spaced(history, "routine", ROUTINE_EVERY)

  /// An incident that opened outside the cooldown always gets its episode trace:
  /// immediately, or once the trace running then has finished.
  val episodesNotMissed: bool =
    TARGETS.forall(t => owed.get(t) implies inFlight.get(t) == 1)

  val invariants: bool =
    oneAtATime and episodesSpaced and routinesSpaced and episodesNotMissed
}
