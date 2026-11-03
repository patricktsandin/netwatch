# 58. Traces wait for a settled path

Date: 2026-11-02

## Status

Accepted. Supersedes ADR-0057's "with no way out, nothing is asked of the trace
schedule", and refines ADR-0056's trace paths.

## Context

ADR-0057 kept the trace schedule from being asked while there was no way out, and
held incidents back until there was one. Review found it wrong in several ways:
- An incident that ended while held back never reached the schedule, which kept its
  role open for good: that role was never episode-traced again.
- The gate waited for ADR-0055's 5 s, so traces asked for in those 5 s started, and
  failed, and the trace a finished one handed on started with no gate at all.
- When the way out came back, every held-back incident asked for a trace at once,
  each spending its target's cooldown on the path that had just recovered, sometimes
  before the local address was known, recording a path that nothing could match.
- The map kept one trace per target, so coming back to a network didn't show its
  traces again, contrary to ADR-0056.

## Decision

- **The schedule hears everything, as it always did.** Every trace it asks for
  waits in a queue, and runs once the whole path (the egress interface and the local
  address) has been known for 10 s.
- **When it runs, an episode trace whose target has no incident open any more is
  dropped**, and the schedule hears it finished: it was for an outage that is over,
  such as the time without a way out itself.
- **The trace path is a type** (`TracePath`), and there is none until both the egress
  interface and the local address are known.
- **The map keeps each target's latest trace on each path**, so it draws a network's
  traces again when the host comes back to it.

## Consequences

- An episode trace runs at least 10 s after the path settles, which is also when the
  routers the outage disturbed have settled.
- A dropped episode trace still spends its target's cooldown in the schedule, as a
  trace that ran would. Giving the cooldown back would be a new transition in
  `specs/traces.qnt`.
