# 18. Feature workflow: spec, types, tests, code, proofs

Date: 2026-10-31

## Status

Accepted

## Context

With a glossary, specs, typed domain models, tests, and proofs all in play, the order
of the work determines which of them leads the design. If the code comes first, the
specs end up describing whatever got built. The alternative considered was code-led
work with specs written afterward.

## Decision

Each feature goes through these steps, each as its own reviewable commit:
1. Update the glossary, the context map, and the ADRs as needed.
2. Update the Quint spec and model-check it.
3. Design the domain types and interfaces, with their interface comments.
4. Write the failing tests, including trace conformance.
5. Implement until everything passes.
6. Add or extend the Verus and Kani proofs.

If a later step shows that an earlier one was wrong, go back and fix it at that step.
Don't patch over the problem downstream.

A change is done only when formatting, clippy, the tests (including trace
conformance), Quint model checking, Kani, and Verus all pass.

## Consequences

- The design leads the code, and the history shows how each design decision was made.
- Small features carry the full process overhead. If that becomes a burden, a new ADR
  can define a lighter path for changes outside the core contexts.
