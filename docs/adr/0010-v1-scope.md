# 10. v1 scope

Date: 2026-10-31

## Status

Accepted

## Context

Plenty of features are plausible, and building toward several of them at once would
spread effort thin and blur the domain.

## Decision

v1 covers one host, graphs, and the UI on localhost only.

Out of scope for v1:
- alerting and notifications
- Prometheus export
- remote access and authentication
- multi-host agents
- firewall ruleset monitoring

Don't add hooks or abstractions in anticipation of these. Any of them can be brought in
later with a new ADR.

## Consequences

- The domain stays small enough to specify and verify thoroughly.
- Bringing any of these in later may require redesign, which is accepted in exchange
  for a simpler v1.
