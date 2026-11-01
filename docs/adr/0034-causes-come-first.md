# 34. A probable cause comes first, and episodes follow causes

Date: 2026-11-01

## Status

Accepted. Supersedes the probable-cause rule of ADR-0031 and the episode rule of
ADR-0033.

## Context

The first real report showed two faults.

- **A cause could come after its effect.** ADR-0031 named as probable cause any
  upstream role with an incident open *at the same time*. A target that had been
  down for an hour was attributed to a NIC incident that began 40 minutes into it.
- **Overlap isn't causation.** ADR-0033 merged every overlapping incident into one
  episode, so one permanently unreachable target swallowed every other incident in
  the span into a single episode, caused by whichever role was most upstream.

The same report showed the NIC graded *failing* for minutes while every hop answered:
`rx_dropped` grows by one now and then, because the kernel counts frames for
protocols it doesn't handle there. That isn't loss.

## Decision

- **A probable cause must begin first.** An upstream incident can be the probable
  cause of another only if it opened no later than `CAUSE_GRACE` (10 s) after it.
  The grace covers cadence: a target probed every 5 s notices an outage a few
  seconds after its hops, which are probed every second. In `specs/incidents.qnt`
  the grace is one tick.
- **Episodes follow causes.** Two incidents are in one episode when one is the other's
  probable cause, and both overlap. An episode is a connected group under that
  relation. Its probable cause is the root of its earliest incident: that incident's
  own cause, or the incident itself.
- **`Link.RxDropped` no longer grades the NIC.** Missed and FIFO errors stay as the
  signals of real receive overflow, and `Link.TxDropped` degrades rather than fails.

## Consequences

- Unrelated problems that happen to overlap are separate episodes, each with its own
  cause.
- Two targets failing together with no failing hop are separate episodes, because
  nothing measured links them.
