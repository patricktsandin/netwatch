# 25. Hops are identified by Probing, through ICMP errors

Date: 2026-10-31

## Status

Accepted

## Context

A hop's identity is the router that answers at that TTL. That isn't something
netlink knows: only a probe can discover it. The topology aggregate (ADR-0020,
ADR-0021) treats its observations as kernel notifications, which are buffered while
desynchronized and reconciled against netlink snapshots. A snapshot lists only the
global roles. Treating hop identities the same way would have every resync end every
hop's epoch, and would hold hop observations back during resyncs that have nothing
to do with them.

Getting the router's answer requires `IP_RECVERR`: time-exceeded errors for a ping
socket arrive on its error queue, as our own original packet together with a
`sock_extended_err` that names the offender.

## Decision

- **A new observation, `ProbeIdentified { role, identity }`**, which always takes
  effect immediately: it is never buffered, and resyncs never touch it. Resync
  reconciles only the global roles (`Role::is_global`: NIC, tunnel, local interface).
- **Every probe socket enables `IP_RECVERR`.**
  - An error is matched by the sequence number and token in our own returned
    packet, so it is exact (ADR-0023's tokenless case doesn't come up on this path).
  - Time exceeded answers a hop probe (`ReplyKind::TimeExceeded`, RTT measured).
    Destination unreachable is the `Unreachable` outcome.
  - The control-message parser is pure, and Kani proves it never panics.
- **Hop probes** are echo requests with TTL 1, 2, and 3, to each target, every
  1 s with a 0.9 s timeout, pinned like their target.
- **Attribution:**
  - The responder of each answered hop probe is reported to Topology first, and the
    sample carries the hop's resulting epoch. So a reroute that changes the router
    at a hop starts a new epoch, exactly like a NIC failover.
  - A loss at a hop that has never answered isn't recorded, because there is no
    identity to attribute it to.

## Consequences

- The development host's hops match `mtr` exactly (192.168.1.1, 100.65.192.1,
  100.127.3.193), with comparable RTTs.
- The Quint topology spec models only kernel-sourced roles. `ProbeIdentified` is
  covered by behaviour tests, but not yet by the spec.
- Hops aren't shared between targets yet (stream sharing, ADR-0002), so identical
  paths are probed once per target.
- Loss at an intermediate hop isn't yet labeled as suspected rate limiting in the UI
  (ADR-0004).
