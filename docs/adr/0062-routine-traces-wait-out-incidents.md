# 62. Routine traces wait out incidents

Date: 2026-11-02

## Status

Accepted. Refines ADR-0061.

## Context

ADR-0061 turns a queued routine trace into the episode trace when the schedule
holds one back. In its cooldown, though, the schedule holds none back, and the
routine trace still ran during the outage, to be taken for the normal path. And an
upgraded trace counted as having served the episode trace even when it stored
nothing (the path changed under it), so the episode trace was then dropped.

## Decision

- **A routine trace doesn't start while its target has an incident open.** It
  waits in the trace queue, and the traces behind it go ahead.
- **An upgraded trace serves the episode trace only if it stored a trace.**

## Consequences

- A target in a long incident has its routine trace put off until the incident
  ends; its normal path stays the last one taken before.
