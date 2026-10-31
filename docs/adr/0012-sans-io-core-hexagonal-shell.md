# 12. Sans-IO domain core inside a hexagonal shell

Date: 2026-10-31

## Status

Accepted

## Context

The options considered were:
- **Classic hexagonal:** domain services call async port traits. Familiar, but I/O and
  time get mixed into the logic, and that logic is hard to model-check.
- **Actor per bounded context:** clear runtime boundaries, but it says nothing about
  purity.
- **Sans-IO:** the domain is a set of pure state machines, and I/O lives outside them.
  This is the approach of `quinn-proto` and `str0m`.

Formal verification (ADR-0016), trace conformance, and fake-free testing (ADR-0017) all
need domain logic that is deterministic.

## Decision

- **Domain crates are pure.** Deterministic state machines take events (OS data already
  translated into domain terms, received packets, clock readings passed in as values)
  and return commands (send this probe, re-run hop discovery) and facts (samples,
  topology events). They contain no `tokio`, `libc`, sockets, filesystem access,
  clocks, randomness, or `async`.
- **Adapters** do all I/O and translate between OS representations and domain types,
  as anti-corruption layers. Raw kernel structures never reach the domain.
- The **composition root** (the `netwatch` binary) wires adapters to the domain, owns
  the runtime, and is the only place `anyhow` is allowed.
- **Dependency rule:** domain crates depend only on `netwatch-shared`,
  `netwatch-verified`, and pure libraries such as `uom`. A workspace test checks
  `cargo metadata` to enforce this.

## Consequences

- Domain logic can be replayed, model-checked, and tested without fakes.
- Adapters need a careful event loop to drive the state machines, including time:
  timers are expressed as "wake me at instant T" commands.
- There are more crates, and the boundaries between them are explicit and enforced.
