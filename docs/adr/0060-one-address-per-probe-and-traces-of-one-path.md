# 60. One address per probe, and traces of one path

Date: 2026-11-02

## Status

Accepted. Supersedes ADR-0059's "HTTPS connects as clients do (RFC 8305)".

## Context

- ADR-0059 raced an HTTPS target's resolved addresses, 250 ms apart. Two things
  followed. An address that failed at once still held the next one back 250 ms, and
  that delay was in every probe's RTT. And whichever family won a race measured that
  probe, so one target's series mixed IPv4 and IPv6 from probe to probe, with nothing
  saying which.
- A trace that was running when the path changed measured part of each path,
  through sockets pinned to an interface no longer used, and was stored as the
  latest trace of the path it started on.
- Every target's routine trace comes due at once, and an episode trace waited behind
  them all.

## Decision

- **HTTPS tries addresses in the resolver's order, one at a time**, moving on only
  when one fails. Its samples all measure the address the system prefers. One that
  drops the handshake silently makes the probe time out: that is what it does to
  anything on this host that uses it.
- **A trace that finishes on a path no longer current is dropped**, as one that
  couldn't run.
- **Episode traces go ahead of waiting routine traces** in the trace queue.
- A trace stops all of its sweeps when it gives up, so none overlaps the next.
- A destination answered at the first TTL traced (4) is "within 4 hops": the TTLs
  before it aren't swept.
- Migration 0013 indexes traces by role and time.

## Consequences

- On a host whose preferred family is broken silently, HTTPS targets are lost,
  where a browser would fall back. The ICMP and DNS targets show the other family
  if they use it.
