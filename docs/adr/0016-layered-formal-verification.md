# 16. Layered formal verification

Date: 2026-10-31

## Status

Accepted

## Context

Correctness matters most in a few places: attributing samples to identities across
topology changes (ADR-0003), reply matching and sequence-number wraparound (ADR-0004),
rollup algebra (ADR-0006), and parsing untrusted bytes (ADR-0007). The available tools
differ in strength and cost:
- **TLA+, Quint, or P:** check the design, not the code.
- **Kani:** bounded model checking of real Rust.
- **Verus:** deductive proofs over unbounded inputs, but restricted to a subset of
  Rust.
- **Property and model-based tests:** cheap, but not exhaustive.

Specs drift away from the code unless something ties them together.

## Decision

Use all four, each where it fits best:

1. **Quint specs** in `specs/`, one per core state machine: topology and identity
   epochs, the probe lifecycle and reply matching, and hop rediscovery.
   - Model-checked with Apalache, and with TLC where the spec is finite.
   - Written for people to read: prose explains every state variable, action, and
     property, and why it matters.
   - Example safety property: a sample is never attributed to an identity it didn't
     measure.
   - Example liveness property: every route change eventually triggers hop rediscovery
     for the affected targets.

   Quint was chosen over TLA+ for its typed, readable syntax with the same
   semantics, and over P for its ecosystem and its trace tooling.
2. **Trace conformance:** `quint run --mbt` generates ITF traces. Rust tests replay them
   against the sans-IO state machines (ADR-0012) using the `itf` crate, and check the
   state after every step. A spec change that breaks conformance blocks the change.
3. **Verus**, limited to `netwatch-verified`: a small crate written entirely in
   the Verus subset. It holds reply-to-probe matching, sequence and counter wraparound,
   the merge algebra for rollups and percentiles, and the rules for advancing identity
   epochs. Full proofs, with no `assume`. Any `external_body` needs its own ADR.
4. **Kani and property tests:**
   - Kani harnesses for every parser of untrusted bytes: ICMP/ICMPv6, DNS, `/proc`,
     netlink attributes, kernel-log lines. Properties: no panics, and serialize-then-parse
     round-trips.
   - `proptest` for algebraic laws, and state-machine tests against a reference model.
   - `loom` for any concurrency in the shell.

Never weaken a property, delete a proof, or add `assume` to make a change pass.

## Consequences

- Strong assurance where it matters, without putting the whole codebase in the Verus
  subset.
- More toolchains: Quint (Node), Apalache (JVM), Kani, and Verus. Verification time
  adds to every change.
- The Verus crate's interface is deliberately narrow, with plain integer types (ADR-0014).
