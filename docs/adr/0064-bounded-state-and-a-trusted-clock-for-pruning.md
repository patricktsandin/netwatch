# 64. Bounded state, and a trusted clock for pruning

Date: 2026-11-02

## Status

Accepted. Refines ADR-0006's retention and ADR-0056's trace paths.

## Context

Review of the daemon over weeks of uptime found state that only grew: router names
for every router ever seen, and a trace per target for every path the host was ever
on. It also found that pruning trusted the wall clock: a host that boots with its
clock a year ahead, before NTP corrects it, would delete every raw sample, and roll
up nothing for a year after the correction.

## Decision

- **A router name not looked up again within an hour after it expires is dropped.**
- **The map keeps each target's traces on its 4 most recently traced paths.**
- **Pruning waits until the kernel says the clock is synchronized**, or the run has
  been up a day (a host without NTP).
- **Roll-up progress never runs ahead of now**: progress recorded by a clock that
  was stepped forward and back resumes from now.
- Migration 0014 indexes samples by time, which rolling up and pruning look them up
  by; the list of roles is read with one index seek per role.
- Slow reads in the daemon's tick (link speed and duplex, which go to the driver,
  and the sysctls) run off the runtime that times probes, as the counters do.

## Consequences

- On a host whose clock is unsynchronized, raw samples outlive 7 days by up to a day
  after each start.
