# 20. Pin probes to identity epochs; resynchronize after lost notifications

Date: 2026-10-31

## Status

Accepted

## Context

ADR-0003 requires that a sample is never attributed to an identity it didn't measure.
Two properties of netlink make that hard.

**Notification lag.** The kernel applies a route change first and tells us afterward.
Suppose a probe is sent in the gap between the two. It leaves through the new path, but
our view still holds the old binding, so the probe would be attributed to the old
identity. The failover from `eth0` to `wlp0s20f3` on the development host is exactly
this case. No amount of fast event handling closes the gap; it can only make it
smaller.

**Notification loss.** A netlink multicast socket whose receive buffer overflows drops
messages, and the next read fails with `ENOBUFS`. Nothing says which messages were
lost, so from then on our view may silently disagree with the kernel.

Alternatives considered for the lag:
- Discard every sample from probes in flight across a change. This loses data in
  exactly the moments users care most about, and it still doesn't help with probes
  sent before the notification arrived.
- Look up the route again right before every send. That only narrows the race.
- **Pin each probe** to the egress interface and source address of its epoch, so the
  kernel can't route it anywhere else.

## Decision

1. **Identity epochs are globally unique and strictly increasing.**
   - Each epoch is a `u64`, allocated only by the topology aggregate.
   - An epoch never changes once allocated: one role, one identity, one start.
   - It ends exactly once, either when its role is bound to a different identity or
     when the role vanishes.
2. **Probes are pinned.**
   - When a probe is scheduled, it takes the role's current binding as its pin:
     the epoch and identity, including the egress interface and source address.
   - The sockets adapter enforces the pin with `IP_UNICAST_IF` / `IPV6_UNICAST_IF` and
     a bound source address. Both are unprivileged.
   - A pinned probe either travels through its pinned identity or fails to send, in
     which case the outcome is `SendFailed`. Its sample always carries the epoch of its
     pin.
   - Hop identities are the exception. A hop's identity is the address that answered,
     and Probing reports that address to Topology as an observation.
3. **Lost notifications trigger a resynchronization.**
   - When the adapter detects that notifications were lost, the aggregate becomes
     *desynchronized*, records a topology event, and emits a `Resync` command. That
     command asks the adapter for a full snapshot of the kernel's current state.
   - Until the snapshot arrives, existing bindings and pins stay in effect. Pinning keeps
     samples truthful even while the view is stale.
   - The aggregate reconciles the snapshot with its view. A changed role starts a new
     epoch, a vanished role ends its epoch, and then the aggregate is *synchronized*
     again.
4. **Core invariant:** once the aggregate is synchronized and every pending
   notification has been delivered, its bindings equal the kernel's actual state.

## Consequences

- Attribution of samples is correct by construction, and does not depend on reacting
  quickly. The spec (`specs/topology.qnt`) checks this against a model in which the
  real topology and our view of it diverge.
- Probes are sent with interface and source pins, which complicates the sockets
  adapter. A probe pinned to an interface that has disappeared is reported as
  `SendFailed`. That is correct: it really couldn't be sent there.
- Resynchronizing costs one full netlink dump, and only happens when notifications are
  actually lost.
- The netlink adapter must turn `ENOBUFS` into a "notifications lost" observation. It
  must never drop that error silently.
