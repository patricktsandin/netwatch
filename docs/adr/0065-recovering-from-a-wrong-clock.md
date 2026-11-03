# 65. Recovering from a wrong clock

Date: 2026-11-02

## Status

Accepted. Supersedes ADR-0064's pruning and roll-up rules.

## Context

ADR-0064 waited to prune until the clock was synchronized or the run had been up a
day, and clamped roll-up progress to now. Neither worked:
- The clamp never stored the clamped progress, so progress recorded in the future
  still held back every roll-up until the clock caught up with it.
- A run's uptime starts again at every restart, and stops during a suspend, so a
  host whose clock the kernel never reports as synchronized (a VM, a container
  host without NTP) that restarts daily would never prune.

## Decision

- **Roll-up progress found ahead of now is set back** to the start of what is kept
  raw, and everything since is rolled up again: what was rolled up meanwhile was
  rolled up by a wrong clock, and a rollup replaces the one before it.
- **Pruning trusts the clock** when the kernel says it is synchronized, when the host
  has been up a day by the boot clock (which neither restarts nor suspends reset), or
  when now is within the raw retention of the newest sample stored before this
  start. The danger is a clock far ahead of the data, and that is what is checked.

- **Verification that review found short:** `specs/incidents.qnt` is checked with
  three roles too (over one tick and the next, beside two roles over three), with
  an invariant that a probable cause is the most upstream one, which two roles
  couldn't tell apart; and the loops of the DNS lookup's response parser are proven
  by Kani, each on its own, where a harness over the whole parser was intractable.

## Consequences

- A clock that is ahead by less than the retention prunes that much too early.
- Rolling up 7 days again takes a few minutes of the writer's time, once, after a
  clock was wrong.
