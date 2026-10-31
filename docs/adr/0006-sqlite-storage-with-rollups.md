# 6. Embedded SQLite with tiered rollups

Date: 2026-10-31

## Status

Accepted

## Context

Samples arrive at about 1 Hz per series, across dozens of series. The options were an
embedded database, an external time-series database (Prometheus, VictoriaMetrics,
InfluxDB), or in-memory ring buffers like mtr's, which lose history on restart. netwatch
runs on a single host (ADR-0010) and should need no other services. No CPU or disk
budget is imposed, but the daemon should stay reasonably lean.

## Decision

- A single SQLite file in WAL mode. All writes go through one dedicated writer task,
  in batched transactions.
- Raw samples roll up into 1-minute and 1-hour aggregates: count, loss, min/avg/max,
  p50/p95/p99, and jitter.
- Retention: raw samples for 7 days, 1-minute aggregates for 90 days, and 1-hour
  aggregates indefinitely. A background task prunes each tier.
- Counters are stored as raw cumulative values, and rates are computed when queried,
  so counter resets after a reboot or module reload are handled in one place.
- Schema changes go through numbered migrations. A migration that has been applied is
  never edited.
- Persisted durations are integer microseconds, and column names carry their units.

## Consequences

- Nothing to run besides the daemon. Storage is roughly hundreds of MB.
- Percentiles can't simply be averaged when rolling up, so rollups need a mergeable
  summary. That merge algebra is a target for formal proof (ADR-0016).
- There is no Prometheus integration in v1 (ADR-0010).
