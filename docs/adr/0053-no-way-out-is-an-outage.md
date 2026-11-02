# 53. No way out is an outage

Date: 2026-11-02

## Status

Accepted. Extends ADR-0020's pins and ADR-0030's statuses.

## Context

A probe takes the egress binding as its pin (ADR-0020), and its sample carries the
pin's epoch. When the default route goes away (a cable pulled, Wi-Fi disassociated),
the global roles vanish and there is no pin. Nothing can be sent, and with no epoch
there is nothing a sample could truthfully be attributed to, so nothing is recorded.

The grades followed the samples. Without new outcomes, every role became stale after
30 s and was graded unknown, which the incident tracker treats as calm. Every open
incident closed. An outage of hours read as a 30 s blip, and the report's
availability as nearly 100%.

## Decision

- **While there is no egress path, the daemon grades the path failing**: the local
  interface, and every target and hop. It is the daemon's judgement (ADR-0030), from
  Topology's view, not from samples.
- **No samples are made up.** The graphs show a gap with the epoch-ended markers
  that explain it, and the incident bands over it.
- The local interface is upstream of every target, so incidents name it as the
  probable cause (ADR-0034).
- The rule is a pure function, `health::without_egress`, with behaviour tests.

## Consequences

- An outage without a route opens incidents, which stay open until the path is back,
  and the report counts its time as unavailable.
- A host that is deliberately offline, such as a laptop with networking turned off,
  is graded failing too. That is what it is, from where netwatch stands.
