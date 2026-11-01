# 31. Incidents, and the roles an event concerns

Date: 2026-11-01

## Status

Accepted. Extends ADR-0030.

## Context

Statuses (ADR-0030) say how each role is doing *now*. Looking back, the graphs show
RTT and loss ticks, but nothing says "this was an outage from 14:03 to 14:05, and it
started at hop 1". Every event marker is also drawn on every row, so one hop's
identity change or one kernel log line puts lines across a dozen unrelated graphs.

People asked for the history of outages and degradation, shown only on the graphs it
belongs to, with a tooltip that explains it.

## Decision

**Incidents are a fact the daemon records.** An *incident* is a period during which
one role's status was degraded or failing:
- It **opens** on the first grading (each second) that finds the role degraded or
  failing.
- It **closes** on the first grading that doesn't. Rate-limited, healthy, and unknown
  all close it. Its end is the role's first answered probe after its last loss, so
  the band covers the loss rather than the 12-probe grading window that trails it.
  A role graded without probes (the NIC's counters) ends at the closing grading.
- Its **severity** is *outage* if it was ever failing, otherwise *degradation*.
  Severity only escalates.
- Its **probable cause** is the most upstream role with an incident open at the same
  time, if there is one. Upstream runs kernel → NIC → tunnel → local interface →
  hop 1 → hop 2 → hop 3 → target, along one target's path. A DNS outage while hop 1
  is failing is attributed to hop 1, the way you'd read it in `mtr`.

The tracker (`netwatch_probing::incident`) is a pure state machine, specified in
`specs/incidents.qnt` and held to it by trace conformance. Incidents are stored in
their own table and kept forever, because they are small. An incident left open by a
daemon that stopped is closed at its role's last sample when the daemon starts again.

**Every event names the roles it concerns.** The daemon decides, and the UI draws an
event marker only on those rows:

| Event | Concerns |
|---|---|
| a role's epoch started or ended | that role, and every role downstream of it |
| topology desynchronized or resynchronized | the NIC, tunnel, and local interface |
| a kernel log line about the current NIC or tunnel interface | that role |
| any other kernel log line, a clock jump, config drift, a disabled source | the kernel |

**The UI shows both on hover.** An incident is a translucent band behind its own
role's graph: red for an outage, amber for degradation. Hovering a graph shows a
tooltip with the value at that time, the incidents covering it (severity, duration,
probable cause), and the nearby event markers. Picking out what lies under the
cursor is presentation, and lives in a pure, tested module (`web/src/tooltip.ts`).

## Consequences

- The history of a role's health survives restarts and every range (1 h, 24 h, 30 d)
  shows it, including where raw samples have expired.
- Incidents follow the existing grades, which only use loss. A path that answers
  everything, but slowly, has no incident. A latency criterion needs a baseline,
  and would be a later decision.
- An incident opens up to one second after the loss that caused it.
- A hop that is lossy only while its target is too, then rate-limited, records an
  incident only for the time its loss carried through.
