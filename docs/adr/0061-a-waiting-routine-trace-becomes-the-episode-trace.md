# 61. A waiting routine trace becomes the episode trace

Date: 2026-11-02

## Status

Accepted. Refines ADR-0058, ADR-0059, and ADR-0060.

## Context

The trace schedule counts a trace as running from when it asks for it, so a routine
trace still waiting in the trace queue holds back an episode trace of its target
(`pending`) until it finishes. The routine trace then ran during the outage, was
stored as routine, and became the "normal path" that later reports compare
episodes with, while the episode trace came after it.

Review also found that an episode overlapping a later one lost its traces to the
later one's start, that naming a trace's routers one at a time held the queue for
minutes when the resolver didn't answer, and that `TracePath` took a bare string.

## Decision

- **When an incident's episode trace is held back behind a routine trace that is
  only queued, the queued trace becomes the episode trace** (`TraceQueue::upgrade`)
  and goes ahead. The episode trace the schedule hands on once it finishes has
  already run, and is dropped. The schedule says when it holds one back
  (`has_pending`).
- An episode's late-trace grace stops at a later episode's start only when that one
  begins after this one ends.
- A trace's routers are named concurrently.
- `TracePath` is built from the egress identity and the local address.

## Consequences

- A routine trace that was upgraded doesn't run as routine: the target's next
  routine trace is 6 h later, as if it had.
