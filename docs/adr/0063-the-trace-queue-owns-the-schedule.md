# 63. The trace queue owns the schedule

Date: 2026-11-02

## Status

Accepted. Refines ADR-0059, ADR-0061, and ADR-0062.

## Context

ADR-0061's upgrade worked only if the daemon followed a protocol by hand: after the
schedule returned no trace, ask `has_pending`, then tell the queue to upgrade. And
incidents on a shared hop's copies (ADR-0043) never reached the schedule, so a
member target looked calm, and its routine trace ran during the outage, against
ADR-0062.

## Decision

- **`TraceQueue` owns the `TraceSchedule`.** The daemon gives it every incident (with
  whether it is a shared hop's copy), every tick, and the current path, starts what
  `next` hands out, and says when each has `finished`. Every rule of ADR-0058 to
  ADR-0062 lives inside it, tested through that interface.
- **A copy's incident asks for no trace, but its target has an incident open.**
- **Trace conformance also checks `has_pending` and `has_open`** against the spec's
  `pending` and `open` after every step, since the queue steers by them.

## Consequences

- The schedule itself, and so `specs/traces.qnt`, is unchanged.
