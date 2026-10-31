# 19. Frontend verification

Date: 2026-10-31

## Status

Accepted

## Context

ADR-0016 covers the Rust side. The TypeScript SPA (ADR-0005) has correctness risks of its
own:
- merging live WebSocket samples with fetched history without gaps or duplicates
- recovering after a reconnect by filling in missed data
- keeping zoom and pan synchronized across rows
- drawing topology-event markers at the right identity epochs
- trusting the shape of data that crosses the API boundary

No mature tool does deductive verification of TypeScript. The options considered:
1. Quint specs of the UI state machines, with ITF trace conformance in TypeScript tests.
2. TypeScript types generated from the Rust API types, plus validation at the boundary.
3. Strict compiler settings and branded types as lightweight proof.
4. Property-based and model-based testing with `fast-check`.
5. Compiling `netwatch-verified` to WebAssembly so the browser runs the same proven
   algorithms.
6. Dafny compiled to JavaScript: real proofs, but another language.
7. XState statecharts with model-based test generation, which overlaps with (1).

## Decision

Adopt options 1–4:

1. **Quint specs** in `specs/` for the frontend state machines: the history-and-live
   merge, reconnect and backfill, time-axis synchronization, and event-marker placement.
   - The core logic lives in plain TypeScript modules that do no I/O (the same sans-IO
     shape as ADR-0012).
   - Vitest replays the ITF traces (JSON) against those modules and checks the state
     after every step.
2. **A single source for the API contract:**
   - TypeScript types are generated from the Rust adapter's API types with `ts-rs`, and
     generated files are never edited by hand.
   - Every WebSocket or REST payload is parsed once at the boundary into domain types
     ("parse, don't validate"). Code past the boundary never sees unchecked data.
3. **Strict TypeScript:**
   - `strict`, `noUncheckedIndexedAccess`, `exactOptionalPropertyTypes`, and
     `noImplicitOverride`.
   - No `any`, no non-null assertions (`!`), no `as` casts outside the boundary parser.
   - Branded types for domain values, and exhaustive `never` checks on every sum type,
     such as probe outcomes.
4. **`fast-check`:** property tests for algebraic rules (merge is idempotent, the
   result is ordered by time, no duplicates), and model-based tests of the state
   machines against a reference model.

Option 5 is **deferred**: it's worth revisiting, with a new ADR, if the frontend starts
doing aggregation that needs proof. Options 6 and 7 are **rejected**. Dafny adds a
language for little gain once 1–4 are in place, and XState duplicates the Quint specs.

## Consequences

- The frontend gets the same spec-to-code conformance as the backend, with no new
  toolchain, since Quint already runs on Node.
- `ts-rs` creates a build-order dependency: Rust generates the types before
  `npm run build`.
- The rules forbidding casts and non-null assertions make some UI code more verbose.
- "Done" (ADR-0018) now also requires `tsc --noEmit`, the Vitest suite including trace
  conformance, and ESLint to pass.
