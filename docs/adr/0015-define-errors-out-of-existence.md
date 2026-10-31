# 15. Define errors out of existence

Date: 2026-10-31

## Status

Accepted

## Context

In a monitor, failure is data. A probe that times out is a measurement, not an
exception. The alternative considered was a rich error hierarchy for each context, with
lower-level errors wrapped and `?` propagating everything. That would treat expected
outcomes as exceptional, and it would grow wide interfaces, against ADR-0011.

## Decision

- Expected failures are domain outcomes: `Timeout`, `Unreachable { code }`,
  `SendFailed { cause }`. A missing capability is a disabled source plus an event
  (ADR-0007).
- A prober never crashes the daemon.
- Each bounded context has one small `thiserror` enum for truly exceptional conditions.
  `anyhow` appears only in the composition root.
- `panic!`, `unreachable!`, and `expect` are allowed only for invariants that a proof or the
  type system guarantees. The message names that invariant. Outside tests, `unwrap()`
  is not used.

## Consequences

- Error paths are few and meaningful, and the failure modes show up in the data model and the UI.
- Designing outcomes takes more domain modeling up front.
