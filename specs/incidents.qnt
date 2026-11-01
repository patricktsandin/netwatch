// -*- mode: Bluespec; -*-
/**
 * # Incidents: a role's degraded and failing periods
 *
 * This spec is the design of `netwatch_probing::incident::IncidentTracker`
 * (ADR-0031). Terms are defined in `docs/glossary.md`.
 *
 * ## What is modeled
 *
 * Two roles on one path: role 0 is upstream of role 1 (think hop 1 and its
 * target). Time advances in whole ticks. Within a tick, any number of these
 * happen, in any order:
 *
 * - **An outcome**: one of a role's probes is answered or lost.
 * - **A grading**: the daemon grades both roles (each second, ADR-0030). The
 *   grades are arbitrary here: the tracker must be right whatever the grading
 *   rule says.
 *
 * The tracker turns gradings into incidents. Each grading returns `updates`:
 * every incident that opened, changed (severity or probable cause), or closed.
 *
 * - An incident opens on a bad grading (degraded or failing). If the role lost
 *   a probe since its last calm grading, the incident starts at that loss;
 *   otherwise (a layer graded from counters) at the grading.
 * - It closes on the first calm grading. If it saw loss, it ends at the first
 *   answered probe after the last loss (or at the grading, if nothing has been
 *   answered since); otherwise at the grading.
 *
 * ## How the Rust code is held to this spec
 *
 * `crates/netwatch-probing/tests/incident_conformance.rs` replays `outcome` and
 * `grade` steps against the Rust tracker and requires identical `updates`, in
 * role order.
 */
module incidents {

  pure val ROLES = [0, 1]
  pure val MAX_TIME = 3
  /// "No time", for optional times. Times are never negative.
  pure val NONE = -1
  /// The tracker treats every calm grade (healthy, rate-limited, unknown) alike, so
  /// one of them stands for all three and keeps the state space small.
  pure val HEALTHS = Set("healthy", "degraded", "failing")

  pure def upstreamOf(u: int, r: int): bool = u < r
  pure def bad(h: str): bool = h == "degraded" or h == "failing"
  pure def severityOf(h: str): str = if (h == "failing") "outage" else "degradation"

  type Incident = { role: int, started: int, ended: int, severity: str, cause: int }

  // ---------------------------------------------------------------------------
  // The tracker.
  var now: int
  /// The time of each role's latest lost probe.
  var lastLoss: int -> int
  /// Each role's first answered probe after its latest loss.
  var recovered: int -> int
  /// Each role's latest grading that was not bad.
  var lastCalm: int -> int
  var isOpen: int -> bool
  /// The open incident of each role (meaningful only while `isOpen`).
  var current: int -> Incident
  /// Whether the open incident has seen loss, which decides how it ends.
  var probed: int -> bool
  /// What the latest step returned.
  var updates: List[Incident]

  /// Whether this tick's grading has happened: the daemon grades once a second.
  var graded: bool

  // ---------------------------------------------------------------------------
  // History, kept only so the properties can refer to the past. An incident's
  // *lifetime* runs from the grading that opened it to the one that closed it;
  // its band (`started` to `ended`) can be shorter, because it covers the loss
  // rather than the grading window that trails it.
  type Lifetime = { inc: Incident, openedAt: int, closedAt: int, gradedFailing: bool }
  var closed: List[Lifetime]
  var openedAt: int -> int
  /// Whether the open incident was ever graded failing.
  var gradedFailing: int -> bool
  /// Each role's latest grade.
  var lastHealth: int -> str

  action init = all {
    now' = 0,
    lastLoss' = ROLES.foldl(Map(), (m, r) => m.put(r, NONE)),
    recovered' = ROLES.foldl(Map(), (m, r) => m.put(r, NONE)),
    lastCalm' = ROLES.foldl(Map(), (m, r) => m.put(r, NONE)),
    isOpen' = ROLES.foldl(Map(), (m, r) => m.put(r, false)),
    current' = ROLES.foldl(Map(), (m, r) => m.put(r, { role: r, started: 0, ended: NONE, severity: "degradation", cause: NONE })),
    probed' = ROLES.foldl(Map(), (m, r) => m.put(r, false)),
    updates' = [],
    graded' = false,
    closed' = [],
    openedAt' = ROLES.foldl(Map(), (m, r) => m.put(r, NONE)),
    gradedFailing' = ROLES.foldl(Map(), (m, r) => m.put(r, false)),
    lastHealth' = ROLES.foldl(Map(), (m, r) => m.put(r, "healthy")),
  }

  action advance = all {
    now < MAX_TIME,
    now' = now + 1,
    updates' = [],
    graded' = false,
    lastLoss' = lastLoss, recovered' = recovered, lastCalm' = lastCalm,
    isOpen' = isOpen, current' = current, probed' = probed,
    closed' = closed, openedAt' = openedAt, gradedFailing' = gradedFailing, lastHealth' = lastHealth,
  }

  /// One of a role's probes resolves now.
  action outcome = {
    nondet r = Set(0, 1).oneOf()
    nondet answered = Bool.oneOf()
    all {
      lastLoss' = if (answered) lastLoss else lastLoss.set(r, now),
      recovered' =
        if (not(answered)) recovered.set(r, NONE)
        else if (recovered.get(r) == NONE and lastLoss.get(r) != NONE) recovered.set(r, now)
        else recovered,
      probed' = if (not(answered) and isOpen.get(r)) probed.set(r, true) else probed,
      updates' = [],
      now' = now, lastCalm' = lastCalm, isOpen' = isOpen, current' = current,
      graded' = graded, closed' = closed, openedAt' = openedAt,
      gradedFailing' = gradedFailing, lastHealth' = lastHealth,
    }
  }

  /// One role's part of a grading, before probable causes are assigned.
  type RoleStep = { open: bool, inc: Incident, probed: bool, calm: int, update: bool, closes: bool }

  pure def roleStep(
    r: int, h: str, t: int, wasOpen: bool, inc: Incident, wasProbed: bool,
    loss: int, rec: int, calm: int,
  ): RoleStep =
    if (bad(h) and not(wasOpen))
      // Opens. A loss since the last calm grading is where it started.
      val sawLoss = loss != NONE and loss > calm
      { open: true, probed: sawLoss, calm: calm, update: true, closes: false,
        inc: { role: r, started: if (sawLoss) loss else t, ended: NONE, severity: severityOf(h), cause: NONE } }
    else if (bad(h))
      // Stays open; severity only escalates.
      val severity = if (inc.severity == "outage") "outage" else severityOf(h)
      { open: true, probed: wasProbed, calm: calm, update: severity != inc.severity, closes: false,
        inc: inc.with("severity", severity) }
    else if (wasOpen)
      // Closes.
      val ended = if (wasProbed and rec != NONE) rec else t
      { open: false, probed: false, calm: t, update: true, closes: true,
        inc: inc.with("ended", ended) }
    else
      { open: false, probed: false, calm: t, update: false, closes: false, inc: inc }

  /// The most upstream role with an incident open after this grading, upstream
  /// of `r`, if any.
  pure def mostUpstream(r: int, openAfter: int -> bool): int =
    ROLES.foldl(NONE, (best, u) =>
      if (best == NONE and upstreamOf(u, r) and openAfter.get(u)) u else best)

  action grade = {
    nondet h0 = HEALTHS.oneOf()
    nondet h1 = HEALTHS.oneOf()
    val health = Map(0 -> h0, 1 -> h1)
    val steps = ROLES.foldl(Map(), (m, r) => m.put(r,
      roleStep(r, health.get(r), now, isOpen.get(r), current.get(r), probed.get(r),
               lastLoss.get(r), recovered.get(r), lastCalm.get(r))))
    val openAfter = ROLES.foldl(Map(), (m, r) => m.put(r, steps.get(r).open))
    // Probable causes, for incidents still open.
    val final = ROLES.foldl(Map(), (m, r) =>
      val s = steps.get(r)
      val candidate = mostUpstream(r, openAfter)
      val better = s.open and candidate != NONE
        and (s.inc.cause == NONE or upstreamOf(candidate, s.inc.cause))
      m.put(r, if (better) s.with("inc", s.inc.with("cause", candidate)).with("update", true) else s))
    all {
      not(graded),
      graded' = true,
      isOpen' = openAfter,
      current' = ROLES.foldl(Map(), (m, r) => m.put(r, final.get(r).inc)),
      probed' = ROLES.foldl(Map(), (m, r) => m.put(r, final.get(r).probed)),
      lastCalm' = ROLES.foldl(Map(), (m, r) => m.put(r, final.get(r).calm)),
      updates' = ROLES.foldl([], (l, r) => if (final.get(r).update) l.append(final.get(r).inc) else l),
      closed' = ROLES.foldl(closed, (l, r) =>
        if (final.get(r).closes)
          l.append({ inc: final.get(r).inc, openedAt: openedAt.get(r), closedAt: now, gradedFailing: gradedFailing.get(r) })
        else l),
      gradedFailing' = ROLES.foldl(Map(), (m, r) =>
        m.put(r, openAfter.get(r) and (health.get(r) == "failing" or (isOpen.get(r) and gradedFailing.get(r))))),
      lastHealth' = health,
      openedAt' = ROLES.foldl(Map(), (m, r) =>
        m.put(r, if (openAfter.get(r) and not(isOpen.get(r))) now else openedAt.get(r))),
      now' = now, lastLoss' = lastLoss, recovered' = recovered,
    }
  }

  action step = any { advance, outcome, grade }

  // ---------------------------------------------------------------------------
  // Properties.

  /// Every incident that is or was open, with its lifetime (an open incident's
  /// lifetime hasn't ended: `closedAt` is NONE).
  val lifetimes: Set[Lifetime] =
    indices(closed).map(i => closed[i]).union(
      Set(0, 1).filter(r => isOpen.get(r)).map(r =>
        { inc: current.get(r), openedAt: openedAt.get(r), closedAt: NONE, gradedFailing: gradedFailing.get(r) }))

  pure def during(l: Lifetime, t: int): bool =
    l.openedAt <= t and (l.closedAt == NONE or t < l.closedAt)

  /// A band starts no later than the grading that opened it, and ends no earlier
  /// than it starts, and no later than the grading that closed it.
  val bandWithinReason: bool =
    lifetimes.forall(l =>
      l.inc.started <= l.openedAt
        and (l.closedAt != NONE implies (l.inc.started <= l.inc.ended and l.inc.ended <= l.closedAt))
        and ((l.closedAt == NONE) == (l.inc.ended == NONE)))

  /// A role's bands never overlap, and its lifetimes follow one another.
  val noOverlap: bool =
    lifetimes.forall(a => lifetimes.forall(b =>
      (a.inc.role == b.inc.role and a.openedAt < b.openedAt) implies
        (a.closedAt != NONE and a.closedAt <= b.openedAt and a.inc.ended <= b.inc.started)))

  /// The latest grading of a role, if bad, is within the lifetime of an incident.
  /// (Lifetimes change only at gradings, so this covers every bad grading.)
  val badGradingsAreCovered: bool =
    Set(0, 1).forall(r => bad(lastHealth.get(r)) implies isOpen.get(r))

  /// An incident is an outage exactly when it was graded failing during its lifetime.
  val outageMeansFailing: bool =
    lifetimes.forall(l => (l.inc.severity == "outage") == l.gradedFailing)

  /// A probable cause is upstream, and had an incident whose lifetime overlapped.
  val causeIsUpstreamAndOverlaps: bool =
    lifetimes.forall(l => l.inc.cause != NONE implies (
      upstreamOf(l.inc.cause, l.inc.role) and lifetimes.exists(o =>
        o.inc.role == l.inc.cause and (during(o, l.openedAt) or during(l, o.openedAt)))))

  val invariants: bool =
    bandWithinReason and noOverlap and badGradingsAreCovered and outageMeansFailing
      and causeIsUpstreamAndOverlaps
}
