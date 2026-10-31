// -*- mode: Bluespec; -*-
/**
 * # Topology: identity epochs, pinned probes, and resync
 *
 * This spec is the authoritative design of the topology aggregate in
 * `crates/netwatch-topology`. Terms are defined in `docs/glossary.md`.
 * The decisions it models are recorded in ADR-0003 and ADR-0020.
 *
 * ## What is modeled
 *
 * There are two worlds, and the whole difficulty lies in the gap between them:
 *
 * - **The kernel's actual topology** (`actual`, `present`): which identity each
 *   role is really bound to, and which identities (interfaces) exist. It
 *   changes whenever it likes.
 * - **The aggregate's view** (`agg`): what netwatch believes. It learns about
 *   changes only through *notifications*, which arrive late (`pending` is a
 *   FIFO queue) and can be lost (`lossSignal`, which models `ENOBUFS`).
 *
 * Probes are included only so that the central property has something to say
 * about: *a sample is never attributed to an identity it didn't measure*. A
 * probe takes its *pin* from the view when it is scheduled, but what it
 * measures is decided by the real world when it is sent.
 *
 * ## How the Rust code is held to this spec
 *
 * `crates/netwatch-topology/tests/conformance.rs` generates traces with
 * `quint run --mbt` and replays the aggregate's actions (`deliver`,
 * `reportLoss`, `deliverSnapshot`) against the Rust implementation. After every
 * step it checks the bindings, the synchronization state, and the exact outputs.
 * The pure operators in the "Aggregate" section below are therefore the
 * specification of `Topology::observe`.
 *
 * Roles and identities are small integers here, so that the state space stays
 * finite and roles have a total order (the Rust `Role` type is `Ord` too, and
 * the order must match: see the conformance harness).
 */
module topology {

  // ===========================================================================
  // Types
  // ===========================================================================

  type Role = int
  type Identity = int
  type Epoch = int
  type Generation = int

  /// A role's current identity and the epoch in which it holds it.
  type Binding = { identity: Identity, epoch: Epoch }

  /// What the netlink adapter tells the aggregate about a single role. Each
  /// notification carries the role's complete new state, never a delta. That
  /// is why replaying a notification late is harmless (see `deliverSnapshot`).
  type Observation =
    | IdentityObserved({ role: Role, identity: Identity })
    | RoleVanished(Role)

  /// Everything the aggregate emits. `Resync` is a command to the adapter;
  /// everything else is a topology event.
  type Output =
    | EpochStarted({ epoch: Epoch, role: Role, identity: Identity })
    | EpochEnded({ epoch: Epoch, role: Role })
    | Desynchronized(Generation)
    | Resynchronized(Generation)
    | Resync(Generation)

  /// The full kernel state, captured in answer to `Resync(generation)`.
  type Snapshot = { generation: Generation, state: Role -> Identity }

  type Aggregate = {
    bindings: Role -> Binding,
    // The next epoch to allocate. Epochs start at 1 and only ever grow.
    nextEpoch: Epoch,
    synchronized: bool,
    // Incremented on every notification loss. Only a snapshot of the current
    // generation can restore synchronization.
    generation: Generation,
    // Notifications received while desynchronized, in arrival order. They are
    // replayed on top of the snapshot (see `applySnapshot` and ADR-0021).
    buffered: List[Observation],
  }

  type Transition = { agg: Aggregate, outputs: List[Output] }

  type Probe = { id: int, role: Role, pin: Binding }
  type Sample = { probe: int, role: Role, epoch: Epoch, measured: Identity }

  // ===========================================================================
  // Model bounds
  // ===========================================================================

  pure val NUM_ROLES = 2
  pure val ROLES: Set[Role] = 0.to(NUM_ROLES - 1)
  pure val IDENTITIES: Set[Identity] = 0.to(1)
  pure val MAX_PROBES = 2

  /// Bounds how many times the kernel changes the topology in one execution.
  /// This makes the state space finite, so TLC can explore every interleaving
  /// at every depth: about 1.5 million distinct states, in under a minute.
  /// TLC already finds both of the counterexamples recorded in ADR-0021 with a
  /// bound of 3.
  pure val MAX_WORLD_CHANGES = 4

  /// ADR-0020's central decision. Set this to `false` and the simulator finds
  /// a counterexample to `attributionIsTruthful` within a few steps: a probe
  /// scheduled under the old binding is sent along the new path before the
  /// notification arrives.
  pure val PINNED = true

  // ===========================================================================
  // Aggregate: pure operators that specify `Topology::observe`
  // ===========================================================================

  pure val initialAggregate: Aggregate =
    { bindings: Map(), nextEpoch: 1, synchronized: true, generation: 0, buffered: [] }

  pure def without(m: a -> b, k: a): a -> b =
    m.keys().exclude(Set(k)).mapBy(x => m.get(x))

  /// Ends the role's open epoch, if it has one.
  pure def end(a: Aggregate, role: Role): Transition =
    if (a.bindings.keys().contains(role))
      {
        agg: { ...a, bindings: a.bindings.without(role) },
        outputs: [EpochEnded({ epoch: a.bindings.get(role).epoch, role: role })],
      }
    else
      { agg: a, outputs: [] }

  /// Binds the role to the identity. If it already holds that identity,
  /// nothing happens: a repeated notification must not split an epoch.
  /// Otherwise its old epoch ends first and a new epoch starts.
  pure def bind(a: Aggregate, role: Role, identity: Identity): Transition =
    if (a.bindings.keys().contains(role) and a.bindings.get(role).identity == identity)
      { agg: a, outputs: [] }
    else
      val ended = end(a, role)
      val epoch = ended.agg.nextEpoch
      {
        agg: {
          ...ended.agg,
          bindings: ended.agg.bindings.put(role, { identity: identity, epoch: epoch }),
          nextEpoch: epoch + 1,
        },
        outputs: ended.outputs.append(EpochStarted({ epoch: epoch, role: role, identity: identity })),
      }

  /// A notification received while synchronized takes effect immediately.
  /// One received while desynchronized is buffered instead. Applying it now
  /// would be undone when the snapshot arrives, if the snapshot was taken
  /// before the change (ADR-0021).
  pure def observe(a: Aggregate, o: Observation): Transition =
    if (not(a.synchronized))
      { agg: { ...a, buffered: a.buffered.append(o) }, outputs: [] }
    else match o {
      | IdentityObserved(x) => bind(a, x.role, x.identity)
      | RoleVanished(role) => end(a, role)
    }

  /// Applies an observation to a plain role-to-identity map. This is the
  /// state-only counterpart of `observe`, used to replay buffered
  /// notifications on top of a snapshot.
  pure def overlay(state: Role -> Identity, o: Observation): Role -> Identity =
    match o {
      | IdentityObserved(x) => state.put(x.role, x.identity)
      | RoleVanished(role) => state.without(role)
    }

  /// After a notification loss, the view can no longer be trusted. Every loss
  /// starts a new generation, even one that happens while a resync is already
  /// in progress. A snapshot requested before the loss can't account for
  /// changes made after it was taken.
  ///
  /// The buffer is cleared. Everything in it was received before this loss
  /// was reported, so it is older than the snapshot that will be taken for the
  /// new generation. Replaying it on top of that snapshot could revert a
  /// change whose notification was the one that got lost.
  pure def notificationsLost(a: Aggregate): Transition =
    val g = a.generation + 1
    {
      agg: { ...a, synchronized: false, generation: g, buffered: [] },
      outputs: [Desynchronized(g), Resync(g)],
    }

  /// Reconciles the view with a snapshot. First the buffered notifications are
  /// replayed onto the snapshot's state map, then the bindings are reconciled
  /// with the result in a single pass. Roles are visited in ascending order so
  /// that epoch allocation is deterministic. Because replay works on the map
  /// rather than the bindings, the intermediate states never allocate epochs.
  ///
  /// A snapshot from an earlier generation is ignored entirely, because a loss
  /// happened after it was requested. The buffer is kept for the snapshot that
  /// is still to come.
  pure def applySnapshot(a: Aggregate, s: Snapshot): Transition =
    if (s.generation != a.generation or a.synchronized)
      { agg: a, outputs: [] }
    else
      val target = a.buffered.foldl(s.state, (m, o) => overlay(m, o))
      val reconciled = range(0, NUM_ROLES).foldl({ agg: a, outputs: [] }, (t, role) =>
        val stepped =
          if (target.keys().contains(role)) bind(t.agg, role, target.get(role))
          else end(t.agg, role)
        { agg: stepped.agg, outputs: t.outputs.concat(stepped.outputs) }
      )
      {
        agg: { ...reconciled.agg, synchronized: true, buffered: [] },
        outputs: reconciled.outputs.append(Resynchronized(a.generation)),
      }

  // ===========================================================================
  // State
  // ===========================================================================

  // The kernel's actual topology.
  var actual: Role -> Identity
  var present: Set[Identity]
  var worldChanges: int

  // The notification channel between the kernel and the aggregate.
  var pending: List[Observation]
  var lossSignal: bool

  // The adapter's side of a resync.
  var requested: Generation
  var lastTaken: Generation
  var snapshots: List[Snapshot]

  // The aggregate, and what it emitted in the most recent step.
  var agg: Aggregate
  var outputs: List[Output]

  // Ghost state, used only to state properties: every epoch ever allocated.
  var allocated: Epoch -> { role: Role, identity: Identity }

  // Probing, reduced to what attribution needs.
  var inFlight: Set[Probe]
  var samples: Set[Sample]
  var nextProbe: int

  // ===========================================================================
  // Actions
  // ===========================================================================

  pure def recordAllocations(
    alloc: Epoch -> { role: Role, identity: Identity },
    outs: List[Output],
  ): Epoch -> { role: Role, identity: Identity } =
    outs.foldl(alloc, (m, o) => match o {
      | EpochStarted(x) => m.put(x.epoch, { role: x.role, identity: x.identity })
      | _ => m
    })

  action init = all {
    actual' = Map(),
    present' = IDENTITIES,
    worldChanges' = 0,
    pending' = [],
    lossSignal' = false,
    requested' = 0,
    lastTaken' = 0,
    snapshots' = [],
    agg' = initialAggregate,
    outputs' = [],
    allocated' = Map(),
    inFlight' = Set(),
    samples' = Set(),
    nextProbe' = 0,
  }

  /// The aggregate takes a step. `outputs` holds exactly what this step emitted.
  action aggregateSteps(t: Transition): bool = all {
    agg' = t.agg,
    outputs' = t.outputs,
    allocated' = recordAllocations(allocated, t.outputs),
  }

  action aggregateIdle: bool = all {
    agg' = agg,
    outputs' = [],
    allocated' = allocated,
  }

  action probesIdle: bool = all {
    inFlight' = inFlight,
    samples' = samples,
    nextProbe' = nextProbe,
  }

  action resyncIdle: bool = all {
    requested' = requested,
    lastTaken' = lastTaken,
    snapshots' = snapshots,
  }

  /// The kernel sends a notification. If `dropped`, it is lost to buffer
  /// overflow, and only the loss itself will be reported (later).
  action notify(o: Observation, dropped: bool): bool =
    if (dropped) all { pending' = pending, lossSignal' = true }
    else all { pending' = pending.append(o), lossSignal' = lossSignal }

  // --- The kernel changes the topology ---------------------------------------

  action worldBind(role: Role, identity: Identity, dropped: bool): bool = all {
    worldChanges < MAX_WORLD_CHANGES,
    worldChanges' = worldChanges + 1,
    present.contains(identity),
    not(actual.keys().contains(role) and actual.get(role) == identity),
    actual' = actual.put(role, identity),
    present' = present,
    notify(IdentityObserved({ role: role, identity: identity }), dropped),
    resyncIdle, aggregateIdle, probesIdle,
  }

  action worldUnbind(role: Role, dropped: bool): bool = all {
    worldChanges < MAX_WORLD_CHANGES,
    worldChanges' = worldChanges + 1,
    actual.keys().contains(role),
    actual' = actual.without(role),
    present' = present,
    notify(RoleVanished(role), dropped),
    resyncIdle, aggregateIdle, probesIdle,
  }

  /// Interfaces come and go independently of the roles. An interface can only
  /// disappear once no role is bound to it, which matches how the kernel
  /// removes routes before it removes the link.
  action worldAppear(identity: Identity): bool = all {
    worldChanges < MAX_WORLD_CHANGES,
    worldChanges' = worldChanges + 1,
    not(present.contains(identity)),
    present' = present.union(Set(identity)),
    actual' = actual, pending' = pending, lossSignal' = lossSignal,
    resyncIdle, aggregateIdle, probesIdle,
  }

  action worldDisappear(identity: Identity): bool = all {
    worldChanges < MAX_WORLD_CHANGES,
    worldChanges' = worldChanges + 1,
    present.contains(identity),
    actual.keys().forall(r => actual.get(r) != identity),
    present' = present.exclude(Set(identity)),
    actual' = actual, pending' = pending, lossSignal' = lossSignal,
    resyncIdle, aggregateIdle, probesIdle,
  }

  // --- The aggregate reacts (these are the actions the Rust code replays) ---

  action deliver: bool = all {
    worldChanges' = worldChanges,
    pending.length() > 0,
    aggregateSteps(observe(agg, pending.head())),
    pending' = pending.tail(),
    actual' = actual, present' = present, lossSignal' = lossSignal,
    resyncIdle, probesIdle,
  }

  /// The adapter sees `ENOBUFS`, drains the socket and discards everything
  /// queued on it, and only then reports the loss. The `Resync` command
  /// reaches the adapter in the same step, so `requested` advances along with
  /// it.
  ///
  /// The drain is essential. Linux reports `ENOBUFS` *before* the older
  /// messages still queued on the socket. Without the drain, one of those
  /// stale messages can be delivered after the resync and revert the snapshot,
  /// and the notification that would have corrected it is exactly the one
  /// that was lost. Model checking found this (ADR-0021). Every drained
  /// message was generated before the snapshot request, so the snapshot
  /// already covers it.
  action reportLoss: bool = all {
    worldChanges' = worldChanges,
    lossSignal,
    val t = notificationsLost(agg)
    all { aggregateSteps(t), requested' = t.agg.generation },
    lossSignal' = false,
    pending' = [],
    lastTaken' = lastTaken, snapshots' = snapshots,
    actual' = actual, present' = present,
    probesIdle,
  }

  /// The adapter dumps the kernel's state for the latest requested generation.
  /// The dump is atomic with respect to the kernel. Notifications already
  /// queued stay queued, and will be replayed after the snapshot.
  action takeSnapshot: bool = all {
    worldChanges' = worldChanges,
    requested > lastTaken,
    snapshots' = snapshots.append({ generation: requested, state: actual }),
    lastTaken' = requested,
    requested' = requested,
    actual' = actual, present' = present, pending' = pending, lossSignal' = lossSignal,
    aggregateIdle, probesIdle,
  }

  action deliverSnapshot: bool = all {
    worldChanges' = worldChanges,
    snapshots.length() > 0,
    aggregateSteps(applySnapshot(agg, snapshots.head())),
    snapshots' = snapshots.tail(),
    requested' = requested, lastTaken' = lastTaken,
    actual' = actual, present' = present, pending' = pending, lossSignal' = lossSignal,
    probesIdle,
  }

  // --- Probing, reduced to attribution ---------------------------------------

  /// A probe takes the role's current binding from the view as its pin.
  action scheduleProbe(role: Role): bool = all {
    worldChanges' = worldChanges,
    agg.bindings.keys().contains(role),
    nextProbe < MAX_PROBES,
    inFlight' = inFlight.union(Set({ id: nextProbe, role: role, pin: agg.bindings.get(role) })),
    nextProbe' = nextProbe + 1,
    samples' = samples,
    actual' = actual, present' = present, pending' = pending, lossSignal' = lossSignal,
    resyncIdle, aggregateIdle,
  }

  /// The probe is sent and resolved. The real world decides what it measures.
  /// A pinned probe can only go through its pinned identity; if that identity
  /// is gone, the send fails and no sample is attributed. An unpinned probe
  /// follows whatever the kernel's routing does at that moment.
  action sendProbe(p: Probe): bool = all {
    worldChanges' = worldChanges,
    inFlight.contains(p),
    val measured =
      if (PINNED) (if (present.contains(p.pin.identity)) Set(p.pin.identity) else Set())
      else (if (actual.keys().contains(p.role)) Set(actual.get(p.role)) else Set())
    samples' = samples.union(measured.map(i =>
      { probe: p.id, role: p.role, epoch: p.pin.epoch, measured: i }
    )),
    inFlight' = inFlight.exclude(Set(p)),
    nextProbe' = nextProbe,
    actual' = actual, present' = present, pending' = pending, lossSignal' = lossSignal,
    resyncIdle, aggregateIdle,
  }

  action step = {
    nondet role = ROLES.oneOf()
    nondet identity = IDENTITIES.oneOf()
    nondet dropped = Set(true, false).oneOf()
    any {
      worldBind(role, identity, dropped),
      worldUnbind(role, dropped),
      worldAppear(identity),
      worldDisappear(identity),
      deliver,
      reportLoss,
      takeSnapshot,
      deliverSnapshot,
      scheduleProbe(role),
      nondet p = inFlight.oneOf()
      sendProbe(p),
    }
  }

  // ===========================================================================
  // Properties
  // ===========================================================================

  /// **The central safety property (ADR-0003).** Every sample's epoch names
  /// exactly the role and identity that the probe actually measured.
  val attributionIsTruthful: bool =
    samples.forall(s =>
      allocated.keys().contains(s.epoch)
        and allocated.get(s.epoch) == { role: s.role, identity: s.measured }
    )

  /// Epochs are allocated densely from 1 and never reused, so an epoch number
  /// identifies a role and identity forever.
  val epochsAreDenseAndUnique: bool = and {
    allocated.keys().forall(e => 1 <= e and e < agg.nextEpoch),
    allocated.keys().size() == agg.nextEpoch - 1,
  }

  /// Every open binding refers to its own allocation. Bindings and history
  /// never disagree.
  val bindingsMatchAllocations: bool =
    agg.bindings.keys().forall(r =>
      val b = agg.bindings.get(r)
      allocated.keys().contains(b.epoch)
        and allocated.get(b.epoch) == { role: r, identity: b.identity }
    )

  /// No two roles share an open epoch.
  val openEpochsAreDistinct: bool =
    agg.bindings.keys().forall(r1 => agg.bindings.keys().forall(r2 =>
      r1 == r2 or agg.bindings.get(r1).epoch != agg.bindings.get(r2).epoch
    ))

  /// **The convergence property (ADR-0020).** Once every pending notification
  /// has been delivered, no loss is unreported, no snapshot is in flight, and
  /// the aggregate is synchronized, the view equals the kernel's actual
  /// topology. Every unfinished resync and every lost notification shows up
  /// in one of these conditions, so this is how "the view eventually becomes
  /// correct" appears as a safety invariant.
  ///
  /// Model checking found a counterexample to an earlier design without
  /// buffering: a notification newer than the snapshot was delivered before
  /// the snapshot, and the snapshot then reverted it (ADR-0021).
  ///
  /// A notification older than the snapshot can still be replayed after it,
  /// and make the view briefly regress until the later notification for that
  /// role arrives. Every later change produces a notification that is either
  /// delivered or reported lost, so the regression always corrects itself.
  /// Pinning keeps samples truthful in the meantime.
  val quiescentViewIsAccurate: bool =
    (agg.synchronized and pending.length() == 0 and not(lossSignal) and snapshots.length() == 0)
      implies agg.bindings.keys().mapBy(r => agg.bindings.get(r).identity) == actual

  /// While desynchronized, a resync for the current generation has always been
  /// requested.
  val desynchronizedImpliesResyncRequested: bool =
    not(agg.synchronized) implies requested == agg.generation

  val invariants: bool = and {
    attributionIsTruthful,
    epochsAreDenseAndUnique,
    bindingsMatchAllocations,
    openEpochsAreDistinct,
    quiescentViewIsAccurate,
    desynchronizedImpliesResyncRequested,
  }
}
