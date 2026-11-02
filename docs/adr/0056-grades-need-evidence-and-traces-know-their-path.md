# 56. Grades need evidence, and traces know their path

Date: 2026-11-02

## Status

Accepted. Refines ADR-0055, and supersedes its "a different egress identity drops
the map's path traces".

## Context

ADR-0055's changes, reviewed in use:
- A role's window now starts afresh with each epoch. The first probe on a new path,
  lost, was 1 of 1: failing, which opened an outage. A retired stream's late outcome
  of the old epoch emptied the new window again.
- Dropping the map's traces when the egress identity changed missed three cases: a
  trace that was running across the change finished afterwards and was drawn; traces
  loaded at a start were never checked; and two Wi-Fi networks on one interface are
  one egress identity.
- The report's time without a way out added up episodes caused by the local
  interface. Its episode often has another cause, such as a degraded kernel that was
  open first, and then the time read 0 s.

## Decision

- **Failing needs at least 3 outcomes.** With fewer, any loss is degraded.
- **A role's outcomes are those of its newest epoch.** A newer epoch starts the
  window afresh. An outcome of an older one is left out of the grade, though its
  sample is still stored.
- Both rules, and ADR-0055's 5 s wait for no way out, are pure types in
  `netwatch_probing::health` (`Outcomes`, `EgressWatch`), with behaviour tests.
- **Every trace records its path**: the egress interface and the local address,
  such as `wlan0 from 192.168.1.20` (migration 0012). The map draws a target's latest
  trace only if it ran on the current path. A trace from before paths were recorded
  is taken to be on it.
- **The time without a way out is the local interface's own outage incidents**,
  whatever episodes they are in.
- Router names ask the system's resolver of the moment, so they follow it too.

## Consequences

- An outage now needs three probes' evidence: at the targets' 5 s cadence, about
  15 s at worst, where it was 5 s.
- The no-way-out incident starts when it is graded, 5 s after the route went, so
  each such outage is stated 5 s short. Starting it earlier would change the
  incident tracker (`specs/incidents.qnt`), for 5 s.
- Moving between networks clears the map beyond hop 3 until a trace runs on the new
  path; coming back shows the old traces again.
