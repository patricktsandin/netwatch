# 66. Only the host vouches for its clock

Date: 2026-11-02

## Status

Accepted. Supersedes ADR-0065's pruning rule, and refines its roll-up reset.

## Context

ADR-0065 trusted the clock for pruning when it read within the raw retention of the
newest sample stored before the start. That sample may have been written by the same
wrong clock in an earlier run of this boot: a clock a year ahead, a restart (a
settings save, the system resolver changing, a crash), and pruning deleted all real
history. Data the clock wrote can't vouch for the clock.

ADR-0065's roll-up reset also rolled up all 7 days again on any small step back, as
after NTP's first step at boot, holding up the writer for minutes; and it reset the
hours only together with the minutes, so a reset that stopped half way left the
hours stuck in the future.

## Decision

- **The evidence is the host's:**
  - synchronized now: trusted, and recorded (`facts`, migration 0015) as the last
    time it was;
  - never synchronized, on any run: a clock nothing steps, so pruning by it is
    consistent, wherever it is;
  - synchronized before: trusted while it reads within the raw retention after then,
    or once the host has been up a day by the boot clock.
- Whether it is trusted is asked once per prune interval, either way.
- **Each tier's progress ahead of now is set back on its own**, the minutes to just
  after the last minute rolled up before now, and the hours to the hour of the first
  minute rolled up again.
- Migration 0015 indexes rollups and reading rollups by tier and time, as rolling up
  and pruning look them up.
- The map's traces are of each target's most recent paths toward its address now.

## Consequences

- A host that was synchronized once, then boots without NTP more than 7 days later,
  doesn't prune until it has been up a day.
