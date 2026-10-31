# 21. Buffer notifications while desynchronized; drain the socket on loss

Date: 2026-10-31

## Status

Accepted. Refines [ADR-0020](0020-pinned-probes-and-topology-resync.md), whose
decisions all still stand.

## Context

ADR-0020 said that after a notification loss, the aggregate requests a snapshot and
reconciles its view with it. When that protocol was written as a Quint spec
(`specs/topology.qnt`), the simulator found two counterexamples to the convergence
invariant `quiescentViewIsAccurate`. In each, the view settled permanently on a wrong
state.

**Counterexample 1: a notification newer than the snapshot is reverted by it.**
1. A notification is lost, so a resync is requested.
2. The adapter takes the snapshot. Role 0 is unbound in it.
3. The kernel binds role 0. Its notification is delivered *before* the snapshot
   arrives, so the aggregate applies it.
4. The snapshot is applied and unbinds role 0 again.

No further notification will arrive to correct this.

**Counterexample 2: a notification older than the snapshot is replayed after it.**
Linux reports `ENOBUFS` on the next `recvmsg` *before* the messages still queued on the
socket:
1. Notification m₁ ("role 1 bound to identity 0") is queued.
2. The kernel unbinds role 1, and that notification is lost.
3. `ENOBUFS` is reported, and then the snapshot ("role 1 unbound") is applied.
4. m₁ is delivered and rebinds role 1.

The notification that would have corrected this is exactly the one that was lost.

## Decision

1. **Buffer while desynchronized.** A notification received while desynchronized is
   appended to a buffer instead of being applied.
2. **Apply the snapshot and the buffer together.** When the snapshot for the current
   generation arrives, the buffered notifications are replayed onto the snapshot's
   *state map*. The bindings are then reconciled with the result in a single pass,
   visiting roles in ascending order. Because replay works on the map, the
   intermediate states never allocate epochs.
3. **Clear the buffer on every loss.** Everything in it predates the snapshot that the
   new generation will take.
4. **Drain on `ENOBUFS`.** The netlink adapter reads and discards everything queued on
   the socket until `EAGAIN`, then reports the loss, and only then requests the
   dump. Every drained message was generated before the dump request, so the
   snapshot covers it.

A `NotificationsLost` observation therefore means "the socket has been drained", and
the adapter must not report it any earlier.

## Consequences

- With these changes, the simulator finds no violations in 200,000 random 40-step
  traces. Apalache verifies all executions up to 6 steps, and TLC explores every
  interleaving with up to `MAX_WORLD_CHANGES` changes to the topology.
- The view can still regress briefly. A notification older than the snapshot can
  arrive after the drain and be replayed on top of it. This always corrects itself,
  because every later change produces a notification that is either delivered or
  reported lost. Pinning (ADR-0020) keeps samples truthful in the meantime.
- While desynchronized, bindings stay frozen until the snapshot arrives, so a resync
  delays the view by about one netlink dump.
- This is a worked example of why ADR-0018 puts the spec before the code. Both bugs
  would have been intermittent field failures that are very hard to reproduce.
