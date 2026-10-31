// -*- mode: Bluespec; -*-
/**
 * # Probing: probe lifecycle and reply matching on one probe stream
 *
 * The authoritative design of `ProbeStream` in `crates/netwatch-probing`. Terms
 * are defined in `docs/glossary.md`. The decisions are recorded in ADR-0004 and
 * ADR-0023.
 *
 * ## What is modeled
 *
 * - One *probe stream*, sending on a fixed *cadence* in discrete time, using
 *   sequence numbers that wrap modulo `SEQ_SPACE`. In reality the space is
 *   2^16; here it is tiny, so the model checker meets wraparound constantly.
 * - **The network**, adversarially: it may lose a reply, duplicate it, or delay
 *   it by up to `MAX_DELAY` ticks, which is longer than the probe timeout. So
 *   late replies happen, and so do replies whose sequence number has since been
 *   reused.
 * - Each reply either carries the probe's 64-bit *token* or doesn't. Echo
 *   replies echo the payload, so they carry it. ICMP errors often quote only 8
 *   bytes of the original packet, which is the sequence number and nothing more.
 *
 * Ghost state records which probe really caused each reply, so that matching
 * can be judged.
 *
 * ## How the Rust code is held to this spec
 *
 * `crates/netwatch-probing/tests/conformance.rs` replays `tick`, `deliverReply`,
 * and send failures against `ProbeStream`, comparing the outputs and the
 * in-flight set after every step.
 */
module probing {

  // ===========================================================================
  // Types
  // ===========================================================================

  type Index = int      // The probe's position in its stream: also its token.
  type Seq = int        // The on-the-wire sequence number: index mod SEQ_SPACE.

  type ReplyKind = Echo | Unreachable

  /// What arrives from the network. `cause` is ghost: the probe that really
  /// caused this reply, which the stream can't see.
  type Reply = { seq: Seq, token: Index, hasToken: bool, kind: ReplyKind, arrivesAt: int, cause: Index }

  type Outcome =
    | Answered(int)     // round-trip time in ticks
    | TimedOut
    | Unreachable_
    | SendFailed

  type Output =
    | Send({ index: Index, seq: Seq })
    | Resolved({ index: Index, outcome: Outcome })

  type InFlight = { index: Index, sentAt: int }

  type Stream = {
    nextIndex: Index,
    nextDue: int,
    inFlight: Seq -> InFlight,
  }

  type Transition = { stream: Stream, outputs: List[Output] }

  // ===========================================================================
  // Model bounds
  // ===========================================================================

  pure val SEQ_SPACE = 4
  pure val CADENCE = 1
  /// A probe with no answer after this many ticks is `TimedOut`.
  pure val TIMEOUT = 2
  pure val MAX_DELAY = 3
  /// A reply that carries its token may arrive this late: beyond the reuse
  /// window, after its sequence number belongs to a newer probe. Only the token
  /// keeps such a reply from being misattributed.
  pure val LONG_DELAY = SEQ_SPACE * CADENCE + 1
  pure val MAX_PROBES = 6
  /// Time stops once every probe could have been sent, answered, and timed out.
  /// That makes the state space finite, for TLC.
  pure val HORIZON = MAX_PROBES * CADENCE + TIMEOUT + LONG_DELAY + 1

  /// The condition for matching by sequence alone (no token) to be safe: a
  /// reply must arrive before its sequence number can be reused. In reality,
  /// 2^16 probes at a 1 s cadence is about 18 hours, against reply delays of
  /// seconds. Here the bound is checked, not assumed: `MAX_DELAY` < `SEQ_SPACE`
  /// × `CADENCE`.
  pure val SEQ_REUSE_IS_SLOWER_THAN_ANY_REPLY = MAX_DELAY < SEQ_SPACE * CADENCE

  // ===========================================================================
  // Stream: pure operators that specify `ProbeStream`
  // ===========================================================================

  pure val initialStream: Stream = { nextIndex: 0, nextDue: 0, inFlight: Map() }

  pure def without(m: a -> b, k: a): a -> b =
    m.keys().exclude(Set(k)).mapBy(x => m.get(x))

  /// Time advances to `now`. First every probe whose timeout has passed is
  /// resolved as `TimedOut`, in sequence order. Then, if a probe is due, it is
  /// sent. Expiring first means a sequence number is free again before it can
  /// be reused.
  pure def tick(s: Stream, now: int): Transition =
    val expired = range(0, SEQ_SPACE).foldl({ stream: s, outputs: [] }, (t, seq) =>
      if (t.stream.inFlight.keys().contains(seq) and t.stream.inFlight.get(seq).sentAt + TIMEOUT <= now)
        {
          stream: { ...t.stream, inFlight: t.stream.inFlight.without(seq) },
          outputs: t.outputs.append(Resolved({ index: t.stream.inFlight.get(seq).index, outcome: TimedOut })),
        }
      else t
    )
    val e = expired.stream
    if (e.nextDue <= now and e.nextIndex < MAX_PROBES)
      val seq = e.nextIndex % SEQ_SPACE
      {
        stream: {
          nextIndex: e.nextIndex + 1,
          nextDue: e.nextDue + CADENCE,
          inFlight: e.inFlight.put(seq, { index: e.nextIndex, sentAt: now }),
        },
        outputs: expired.outputs.append(Send({ index: e.nextIndex, seq: seq })),
      }
    else expired

  /// The adapter couldn't send the probe it was just asked to send.
  pure def sendFailed(s: Stream, index: Index): Transition =
    val seq = index % SEQ_SPACE
    if (s.inFlight.keys().contains(seq) and s.inFlight.get(seq).index == index)
      {
        stream: { ...s, inFlight: s.inFlight.without(seq) },
        outputs: [Resolved({ index: index, outcome: SendFailed })],
      }
    else { stream: s, outputs: [] }

  /// A reply matches the in-flight probe with its sequence number, and, if the
  /// reply carries a token, only when the token is that probe's index. Anything
  /// else is a late, duplicate, or foreign reply, and it is ignored.
  pure def reply(s: Stream, r: Reply, now: int): Transition =
    if (s.inFlight.keys().contains(r.seq)
        and (not(r.hasToken) or s.inFlight.get(r.seq).index == r.token))
      val p = s.inFlight.get(r.seq)
      val outcome = match r.kind { | Echo => Answered(now - p.sentAt) | Unreachable => Unreachable_ }
      {
        stream: { ...s, inFlight: s.inFlight.without(r.seq) },
        outputs: [Resolved({ index: p.index, outcome: outcome })],
      }
    else { stream: s, outputs: [] }

  // ===========================================================================
  // State
  // ===========================================================================

  var now: int
  var stream: Stream
  var outputs: List[Output]
  var network: Set[Reply]
  // Ghost: every probe sent, and how each was resolved.
  var sent: Set[Index]
  var resolved: Index -> Outcome
  var doubleResolution: bool
  var misattributed: bool

  // ===========================================================================
  // Actions
  // ===========================================================================

  /// Records what a step emitted, and judges it against the ghost truth.
  /// `cause` is the probe that really caused the reply being processed (or -1).
  action streamSteps(t: Transition, cause: Index): bool = all {
    stream' = t.stream,
    outputs' = t.outputs,
    val res = t.outputs.foldl(resolved, (m, o) => match o {
      | Resolved(x) => m.put(x.index, x.outcome)
      | _ => m
    })
    val newlyResolved = t.outputs.foldl(Set(), (acc, o) => match o {
      | Resolved(x) => acc.union(Set(x.index))
      | _ => acc
    })
    all {
      resolved' = res,
      doubleResolution' = (doubleResolution or newlyResolved.exists(i => resolved.keys().contains(i))),
      misattributed' = (misattributed or t.outputs.select(o => match o {
        | Resolved(x) => match x.outcome {
          | Answered(_) => x.index != cause
          | Unreachable_ => x.index != cause
          | _ => false
        }
        | _ => false
      }).length() > 0),
    },
  }

  action init = all {
    now' = 0,
    stream' = initialStream,
    outputs' = [],
    network' = Set(),
    sent' = Set(),
    resolved' = Map(),
    doubleResolution' = false,
    misattributed' = false,
  }

  /// Time advances. If a probe is sent, the network nondeterministically
  /// produces zero, one, or two replies to it, each with its own delay and
  /// kind, and with or without the token.
  ///
  /// Time can't advance past a reply that has already arrived. Without this, a
  /// reply could wait in the model forever, and the delays wouldn't bound its
  /// age. (The first simulation found exactly that hole.)
  action advance(fate: Set[{ delay: int, kind: ReplyKind, hasToken: bool }]): bool =
    val t = tick(stream, now + 1)
    val sends = t.outputs.select(o => match o { | Send(_) => true | _ => false })
    all {
      network.forall(r => r.arrivesAt > now),
      now < HORIZON,
      now' = now + 1,
      streamSteps(t, -1),
      network' = network.union(
        sends.foldl(Set(), (acc, o) => match o {
          | Send(x) => acc.union(fate.map(f => {
              seq: x.seq, token: x.index, hasToken: f.hasToken, kind: f.kind,
              arrivesAt: now + 1 + f.delay, cause: x.index,
            }))
          | _ => acc
        })
      ),
      sent' = sent.union(sends.foldl(Set(), (acc, o) => match o {
        | Send(x) => acc.union(Set(x.index)) | _ => acc
      })),
    }

  /// The adapter reports that the most recent send failed.
  /// The index of the probe sent in the previous step, or -1 if none was.
  pure def lastSent(outs: List[Output]): Index =
    outs.foldl(-1, (acc, o) => match o { | Send(x) => x.index | _ => acc })

  action failLastSend: bool = all {
    val index = lastSent(outputs)
    all {
      index >= 0,
      streamSteps(sendFailed(stream, index), -1),
      network' = network.filter(r => r.cause != index),
    },
    now' = now, sent' = sent,
  }

  action deliverReply(r: Reply): bool = all {
    network.contains(r),
    r.arrivesAt <= now,
    streamSteps(reply(stream, r, now), r.cause),
    network' = network.exclude(Set(r)),
    now' = now, sent' = sent,
  }

  /// Tokenless replies stay within `MAX_DELAY`, the reuse window. Replies that
  /// carry a token can also arrive after `LONG_DELAY`, beyond it.
  pure val FATES = Set(
    { delay: 1, kind: Echo, hasToken: true },
    { delay: MAX_DELAY, kind: Echo, hasToken: true },
    { delay: LONG_DELAY, kind: Echo, hasToken: true },
    { delay: 1, kind: Echo, hasToken: false },
    { delay: MAX_DELAY, kind: Unreachable, hasToken: false },
  )

  action step = any {
    nondet fate = FATES.powerset().filter(f => f.size() <= 2).oneOf()
    advance(fate),
    failLastSend,
    nondet r = network.oneOf()
    deliverReply(r),
  }

  // ===========================================================================
  // Properties
  // ===========================================================================

  /// Every probe gets at most one outcome (ADR-0004).
  val atMostOneOutcome: bool = not(doubleResolution)

  /// A reply is only ever attributed to the probe that caused it.
  val repliesAreNeverMisattributed: bool = not(misattributed)

  /// No probe stays in flight past its timeout, so every probe gets an outcome
  /// one tick after its deadline at the latest.
  val nothingOutlivesItsTimeout: bool =
    stream.inFlight.keys().forall(seq => stream.inFlight.get(seq).sentAt + TIMEOUT > now)

  /// The in-flight map is keyed by sequence number, so a send must never land
  /// on a sequence that is still in flight. This holds because `TIMEOUT` ≤
  /// `SEQ_SPACE` × `CADENCE` and expiry runs before sending.
  val inFlightIndicesAgreeWithSeqs: bool =
    stream.inFlight.keys().forall(seq => stream.inFlight.get(seq).index % SEQ_SPACE == seq)

  /// Every probe sent is either in flight or resolved, never lost track of.
  val everyProbeIsAccountedFor: bool =
    sent.forall(i =>
      resolved.keys().contains(i)
        or stream.inFlight.keys().exists(seq => stream.inFlight.get(seq).index == i)
    )

  /// RTTs are never negative and never exceed the timeout.
  val rttsAreBounded: bool =
    resolved.keys().forall(i => match resolved.get(i) {
      | Answered(rtt) => 0 <= rtt and rtt < TIMEOUT
      | _ => true
    })

  val invariants: bool = and {
    SEQ_REUSE_IS_SLOWER_THAN_ANY_REPLY,
    atMostOneOutcome,
    repliesAreNeverMisattributed,
    nothingOutlivesItsTimeout,
    inFlightIndicesAgreeWithSeqs,
    everyProbeIsAccountedFor,
    rttsAreBounded,
  }
}
