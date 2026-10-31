# 11. *A Philosophy of Software Design* settles design conflicts

Date: 2026-10-31

## Status

Accepted

## Context

The classic books disagree. *Clean Code* favors many tiny functions and treats
comments as a failure. *A Philosophy of Software Design* (Ousterhout) favors deep
modules and comments that carry intent. *Domain Modeling Made Functional* (Wlaschin)
favors encoding the design in types. Without a tiebreaker, style drifts from module
to module.

## Decision

Ousterhout settles conflicts:
- **Deep modules:** a small interface hiding a lot of functionality. No shallow
  pass-through layers, wrappers that only rename, or one-line functions that exist only
  to have a name.
- **Information hiding:** each module owns a design decision. If two modules have to
  change together, the boundary is in the wrong place.
- **Comments carry what code can't say:** the why, the invariants, units, and the
  intuition behind non-obvious code. Every module and public item has an interface
  comment. Implementation comments are rare, and explain why, never what.
- **Define errors out of existence** (ADR-0015).
- **Strategic programming:** when a change doesn't fit the design, change the design
  instead of patching around it.

Wlaschin's type-driven modeling (ADR-0014) is used alongside these rules and doesn't
override them.

## Consequences

- Modules are fewer and richer. Reviews ask "what does this hide?" before "is this short?"
- Comments must be maintained along with the code, and a stale comment is a defect.
