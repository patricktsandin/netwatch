# 13. Strategic DDD, with tactical patterns only where they earn their place

Date: 2026-10-31

## Status

Accepted

## Context

The codebase is meant to be a model of domain-driven design. The options considered
were:
- **Full Evans/Vernon:** every tactical pattern (repositories, factories, domain
  services, application services, specifications) used consistently.
- **Strategic plus selective tactical:** the shared vocabulary, a context map, and
  bounded contexts, with tactical patterns used only where they help.
- **Light:** a glossary plus newtypes.

Using every pattern everywhere produces shallow modules, which conflicts with
ADR-0011.

The word "kernel" is overloaded here: the Linux kernel, the DDD "shared kernel"
pattern, and a "verified kernel" of proven code.

## Decision

- **Ubiquitous language:** `docs/glossary.md` is the authority. Code, tests, specs, UI
  text, and commit messages use its terms exactly as written. A new concept goes into the
  glossary first, and a renamed concept is renamed everywhere in the same change.
- **"Kernel" means the Linux kernel only.** The shared-kernel pattern is called the
  *shared crate* (`netwatch-shared`), and the Verus crate is the *verified crate*
  (`netwatch-verified`).
- **Context map:** `docs/context-map.md` classifies each context and its relationships
  (customer–supplier, anti-corruption layer, published language), with a diagram.
  The initial contexts are:
  - **Topology** (core): links, addresses, routes, and neighbors → egress paths,
    identities, identity epochs, topology events.
  - **Probing** (core): probe schedules, the probe lifecycle, reply matching, per-target
    hop discovery, and deduplication of probe streams.
  - **Host health** (supporting): kernel and NIC telemetry, including counters,
    pressure, clock health, drop reasons, and kernel-log events.
  - **History** (supporting): samples, rollups, and the retention policy.
  - **Presentation** (generic): the API and UI read models.
- Each context has its own domain crate. The crate layout follows the context map and
  changes when it changes.
- **Tactical patterns:**
  - Aggregates enforce invariants; for example, only the Topology aggregate assigns
    identity epochs.
  - Value objects for every domain value.
  - Domain events for anything another context reacts to.
  - No repositories, factories, or "services" added just to complete the pattern set.

## Consequences

- Names in the code mean exactly one thing, and the code reads like the domain.
- Keeping the glossary up to date is a real cost that every change shares.
- The workspace has many crates. That is accepted, because the boundaries are the point.
