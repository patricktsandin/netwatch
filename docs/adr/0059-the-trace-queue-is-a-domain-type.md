# 59. The trace queue is a domain type

Date: 2026-11-02

## Status

Accepted. Refines ADR-0058.

## Context

ADR-0058's queue was written into the composition root's select loop, where no test
reached it, and review found it wrong:
- A switch straight from one path to another didn't restart the settling.
- Traces were released together, but the tracer ran them one at a time, so the later
  ones started tens of seconds after the checks that let them through.
- The tracer read the pin and the path from two channels at different moments, and
  ran with no path at all, recording a trace that matched every path.
- It kept its own set of open incidents, which disagreed with the trace schedule's.

## Decision

- **`TraceQueue`** in `netwatch_probing::trace` is the queue: pure, with behaviour
  tests. It holds the current path and since when it has been current (any change
  restarts the settling), hands out one request at a time, and checks each when it
  would start: on a settled path, and, for an episode trace, with an incident open.
- **The schedule says which targets have an incident open** (`has_open`), by its own
  rule of which incidents a trace can tell more about.
- **The tracer runs what it is given**, with the pin and the path the queue handed
  out together, and nothing else.
- In the report, each episode trace's baseline is of its own path, and an episode's
  traces include those that started up to 2 min after it ended.
- HTTPS connects as clients do (RFC 8305): each resolved address gets 250 ms before
  the next is tried alongside it, so a silently broken address costs a probe at most
  that delay.

## Consequences

- The queue is not in `specs/traces.qnt`: it orders and drops requests, while the
  spec's schedule decides them. Its rules are few enough for behaviour tests.
