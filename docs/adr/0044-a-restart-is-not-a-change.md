# 44. A restart is not a change

Date: 2026-11-02

## Status

Accepted. Refines ADR-0003's epochs and ADR-0031's event markers.

## Context

Every time the daemon starts, Topology binds every role afresh and starts a new
identity epoch for each. Each start is recorded as an `epoch-started` event, which
the graphs draw as "NIC is now eth0", even when eth0 was the NIC before the restart
too. The public address is noted again on every start for the same reason. On the
development host, a day of reinstalls left 11 identical markers on every row.

The new epochs themselves are right. The daemon wasn't measuring while it was down,
so nothing it measures afterwards may be aggregated with what came before as if
the measurements were continuous (ADR-0004). Calling them changes is wrong, though.
Markers are for things a person should look at (ADR-0031).

## Decision

- **An epoch that resumes is not a change.** When a role's first epoch after a start
  has the same identity as its last epoch before the stop, the event is recorded as
  `epoch-resumed`, not `epoch-started`. The `epochs` table is unchanged, and the
  epoch is still new.
- **`epoch-resumed` concerns no role**, so no graph marks it. Reports leave it out
  of their events.
- **A start is one event, `started`, on the kernel row,** which says which version
  started. It explains the gap in every graph that follows it.
- **The public address is noted only when it differs** from the last one recorded,
  across restarts too.
- **The daemon decides** (ADR-0030). On start it reads each role's last identity and
  the last public address from the store. Before that read, the history must already
  be in the store, which is the case when the daemon starts.
- **Migration 0008 fixes history**: an `epoch-started` event becomes `epoch-resumed`
  when its role's previous epoch had the same identity and no `epoch-ended` event. Only
  a restart ends an epoch without one. A first `public-address` note of a run that
  repeats the address before it is deleted, because it says nothing.

## Consequences

- Identity changes across a restart, such as a laptop that was moved to another
  network while the daemon was stopped, are still marked, because the identity
  differs.
- Within a run, an identity that ends and comes back the same (a link going down and
  up) is still marked twice. That is a change worth seeing.
- Routine path traces still run at every start (ADR-0040). Each one is a real
  measurement, and its event says what it found.
