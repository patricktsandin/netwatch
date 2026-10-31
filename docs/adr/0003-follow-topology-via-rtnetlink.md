# 3. Follow topology changes via rtnetlink, with identity epochs

Date: 2026-10-31

## Status

Accepted

## Context

Following the NIC and every address through routing and network changes is a core
requirement (ADR-0002). There are several ways to find out about changes:
- Poll `ip` output or `nmcli`. This is slow, depends on formats meant for people, and
  misses changes that happen between polls.
- Read the default route and assume every target uses it. This is wrong under policy
  routing, VPNs, and multiple defaults.
- Subscribe to the kernel's own notifications.

If hop discovery runs only on a timer, a reroute goes unnoticed until the next cycle,
and during that time samples are attributed to the old path.

## Decision

- rtnetlink multicast groups are the source of truth: link, IPv4/IPv6 address,
  IPv4/IPv6 route, and neighbor. Never parse `ip` or `nmcli` output.
- On any relevant change, compute each target's egress path with `RTM_GETROUTE`,
  which asks the kernel to resolve the route exactly as it would for a real packet,
  then rebind the probes.
- A route or gateway change triggers hop rediscovery immediately for the affected targets.
- When the identity behind a role changes, the role starts a new **identity epoch**.
  Every sample carries its epoch. Aggregates such as percentiles are never computed
  across epochs.
- Every topology change is written to an events table and drawn as an annotation on
  all graphs.

## Consequences

- Changes are seen within milliseconds, and even complex routing is handled correctly.
- The topology aggregate becomes the most important core state machine, and the one
  most worth specifying formally (ADR-0016). Its key invariant is that a sample is never
  attributed to an identity it didn't measure.
- Users can match a change in latency to its cause on the graph.
