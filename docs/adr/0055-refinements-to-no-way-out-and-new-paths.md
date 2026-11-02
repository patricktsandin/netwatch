# 55. Refinements to no way out, new paths, and the system resolver

Date: 2026-11-02

## Status

Accepted. Refines ADR-0053, ADR-0054, and ADR-0009's `dns = "system"`.

## Context

Review of ADR-0053 and ADR-0054 in use found gaps:
- At a start, and between the observations of one change (a VPN coming up ends the
  NIC's binding before the tunnel's starts), there is briefly no egress path. Graded
  at once, each of those moments opened a one-second outage, stored forever.
- The report's availability counts probes. With no way out none are sent, so the
  time is in no figure: ADR-0053's promise that the report counts it was not kept.
- A role's grade came from its last 12 outcomes, whatever epoch they were in, so
  after a failover the new router was graded by the old one's losses.
- The map kept drawing the routers that path traces found beyond hop 3 on the old
  path, for up to 6 h.
- `dns = "system"` was resolved once, when the configuration was read. Offline, the
  configuration was refused and the daemon didn't start. On another network, it kept
  probing the old network's resolver.

## Decision

- **No way out counts once it has lasted 5 s.**
- **The report states the time without a way out** (outages the local interface
  caused) beside the outage time beyond the gateway, and says that availability
  can't include it.
- **A role's outcome window holds one epoch's outcomes**: a new epoch starts it
  afresh.
- **A different egress identity from the last one known drops the map's path
  traces**, until a trace runs on the new path. The same interface coming back,
  after a time with no route, keeps them.
- **`dns = "system"` follows the system's resolver.** A `system` target without
  one is left out of the run, as a disabled source, rather than refusing the
  configuration. Every 30 s the daemon reads the resolver again, and when it has
  changed (or appeared), it restarts itself, as a settings save does (ADR-0035). A
  restart resumes every epoch it can (ADR-0044).

## Consequences

- An outage shorter than 5 s with no route at all isn't graded down. Its gap and
  epoch markers still show.
- Moving to another network restarts the daemon once, with a `started` marker.
- No path trace is requested on a path change: that would be a new transition in
  `specs/traces.qnt`. The next routine or episode trace draws the new path.
