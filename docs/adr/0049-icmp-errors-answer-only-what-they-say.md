# 49. ICMP errors answer only what they say

Date: 2026-11-02

## Status

Accepted. Refines ADR-0004's distinct outcomes, ADR-0025's hop answers, and
ADR-0027's DNS probes.

## Context

ADR-0004 makes every ICMP error a distinct outcome, never a timeout and never an
answer. Two kinds of probe broke that rule:

- **Time exceeded** answers a hop probe (ADR-0025): the router at that TTL says it
  got the probe. `ProbeStream` treated it as an answer for every stream. A routing
  loop toward a target makes some router send time exceeded for the target's echo
  request, and the target was graded healthy, with plausible RTTs, while it could
  not be reached.
- **DNS probes** use a connected UDP socket without `IP_RECVERR`. An ICMP error
  for a query, such as port unreachable from a host whose resolver is down, came
  back from `read()` as `ECONNREFUSED`, which carries no query ID. The adapter only
  logged it, and the probe was stored as a timeout.

## Decision

- **Time exceeded answers a probe only on a stream that limits its TTL.** A
  stream's configuration says whether it does (hop streams do). On any other stream,
  time exceeded resolves the probe as a new loss, `TimeExceeded` (stored as
  `time-exceeded`): the probe expired in transit, which means a routing loop or a
  path longer than its TTL.
- **DNS sockets receive ICMP errors through the error queue**, like echo sockets
  (ADR-0024). The kernel quotes the original query, so each error is matched to its
  probe by the query's ID, never by arrival order (ADR-0023). Unreachable becomes
  `Unreachable { code }`, and time exceeded becomes `TimeExceeded`, by the rule
  above.
- **Migration 0011** widens the samples table's `loss` check to the new reason.

## Consequences

- The distinction is a classification of a reply that was already matched, not a
  matching rule, so the probing spec (`specs/probing.qnt`), which models matching,
  is unchanged. Behaviour tests cover it, as they cover `Refused`.
- Time exceeded toward a target counts as loss, and grades the target, as a
  routing loop should.
- A DNS server that is down on a host that is up shows as unreachable, not as
  timed out.
