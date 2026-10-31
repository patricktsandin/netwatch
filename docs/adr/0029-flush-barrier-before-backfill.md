# 29. Flush the writer before every backfill

Date: 2026-10-31

## Status

Accepted

## Context

The UI combines two sources (ADR-0019): live samples over a WebSocket, and a
backfill from the database whenever the socket (re)opens. The database lags behind
the live stream, because the writer batches samples for up to a second (ADR-0006).

Writing the spec `specs/live_backfill.qnt` showed what that lag does. TLC finds the
following in under a second:
1. The socket drops. A sample is produced and broadcast, but nobody receives it.
2. The socket reopens, and the client asks for everything since its latest sample.
3. The backfill query runs before the writer has flushed that sample.

The sample is in neither source, and nothing will ever fetch it again: a permanent
gap in the graph.

## Decision

- **Every API read is preceded by a flush barrier.** It sends `Flush` down the
  writer's channel and waits for the acknowledgement. The channel is FIFO, so every
  sample broadcast before the request has been written by the time the query runs.
- **The client subscribes first** (the socket opens) **and backfills second**, so every
  sample produced after the backfill query arrives live. Together with the barrier,
  that leaves no window in which a sample can fall through.
- The spec's client side is held to the TypeScript code by trace conformance in
  Vitest (`src/conformance.test.ts`), which replays generated traces against
  `merge` and `latest`.

## Consequences

- TLC verifies the convergence invariant over the spec's whole state space with the
  barrier, and finds the gap without it.
- A read waits for at most one batch write. Reads measured at about 10 ms.
- ESLint allows casts in a second boundary file: the conformance test's ITF decoder.
