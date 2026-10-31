# 26. Share the first hop across targets; show hop-only loss as rate limiting

Date: 2026-10-31

## Status

Accepted. A first step toward ADR-0002's stream sharing.

## Context

On the development host, with hops 1–3 probed separately toward each of three
targets, two of the three hop-1 rows showed 100% loss, while hop 2 and hop 3 toward the
same targets were healthy. The gateway answers three streams × 1 probe/s with ICMP time
exceeded, which trips its ICMP rate limit: it answered one stream and ignored the
others. No test caught this; looking at the UI did.

Two different rules apply:
- ADR-0004: loss at an intermediate hop that doesn't carry through to later hops is not
  real loss, and must not be alarmed on.
- ADR-0002: targets whose hops resolve identically should share one probe stream.

## Decision

- **Share hop 1.** Every target pinned to the same interface leaves through the same
  gateway, so hop 1 is the same router for all of them. The daemon runs one hop-1
  stream per pin, and fans each resolution out to every target's hop-1 role. Each
  role still gets its own epoch and its own sample.
- **Show hop-only loss as `rate-limited?`.** When a hop row is degraded or failing but
  a role further along the same path (a later hop, or the target) is healthy, the
  UI shows a dashed `rate-limited?` status, with an explanation, instead of failing.
  This is a pure, tested function (`pathAware`).

## Consequences

- The false loss disappeared: 0 losses across every hop role over 45 seconds,
  compared with 100% on two hop-1 rows before.
- Hops 2 and 3 are still probed per target. Sharing them requires knowing that paths
  coincide, which needs the dynamic responder-based sharing ADR-0002 describes. Still
  to do.
- The hop-1 assumption fails under policy routing that sends different targets via
  different gateways on the same interface. The per-target egress paths of ADR-0022's
  limitations would detect that, and are needed before generalizing.
