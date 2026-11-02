# 54. A new way out is a new path

Date: 2026-11-02

## Status

Accepted. Refines ADR-0025's hop identities and ADR-0051.

## Context

A hop's identity is the router that last answered for it (ADR-0025), and a hop
that falls silent keeps it: silence is loss, not a change of router. When the egress
pin changes, though, such as a failover from Ethernet to Wi-Fi, the hops are on
another path. Until a router on the new path answered, every loss there was recorded
against the old path's router, and the map and the report named the old gateway for
the new path.

## Decision

- **When the egress pin changes, every hop's epoch ends** (`ProbeVanished`, ADR-0051).
  Each hop is identified again by the first router on the new path that answers.
- **A hop resolution pinned to an earlier egress is dropped**, because it measured a
  path whose hops' epochs have ended. A stream retired by the change can still
  resolve probes it sent before (ADR-0020).
- Between the change and the first answer, a hop has no identity, so its losses
  aren't recorded (ADR-0025). The targets' losses are, under the new pin.

## Consequences

- A failover shows on every hop row as an epoch change, which is what it is.
- A link that goes down and comes back starts a new egress epoch, and so new hop
  epochs too, even when the routers are the same.
