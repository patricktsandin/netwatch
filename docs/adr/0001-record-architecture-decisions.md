# 1. Record architecture decisions

Date: 2026-10-31

## Status

Accepted

## Context

netwatch is meant to be a model of domain-driven design and readable code. Readers, both
people and coding agents, need to know not only what the design is but why it is that
way, and which alternatives were rejected. If that reasoning lives in chat logs or in a
continuously edited `CLAUDE.md`, it gets lost or quietly rewritten. Agent instructions
are also loaded into every session, so filling them with rationale costs context
without changing behavior.

## Decision

- Record every significant decision as an Architecture Decision Record in
  `docs/adr/NNNN-kebab-title.md`, using Michael Nygard's format: Title, Date, Status,
  Context, Decision, Consequences.
- An accepted ADR is never edited, except to change its status to "Superseded by
  ADR-NNNN". To change a decision, write a new ADR that supersedes the old one.
- `docs/adr/README.md` is the index.
- `CLAUDE.md` holds only the rules an agent needs while working and links to ADRs
  for the reasons.
- The other design documents, kept current alongside the code they describe, are:
  - `docs/glossary.md`, the ubiquitous language (ADR-0013)
  - `docs/context-map.md`, the bounded contexts (ADR-0013)
  - the annotated Quint specs in `specs/` (ADR-0016)
  - rustdoc design notes: every crate's and module's `//!` comment states its
    responsibility, its invariants, and what it hides, so `cargo doc` reads as a
    guided tour of the design

## Consequences

- Rationale survives, and the history of the design can be followed.
- Writing an ADR adds a small cost to each significant decision. That cost is intended.
- `CLAUDE.md` stays short and focused on what to do.
