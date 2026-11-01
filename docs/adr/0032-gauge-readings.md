# 32. Gauge readings: graphs for what isn't a probe

Date: 2026-11-01

## Status

Accepted. Refines ADR-0006 and the glossary's *rate*.

## Context

The NIC row has never shown a graph. Its health comes from counters (ADR-0008), which
produce a status but no samples, and samples were the only thing History stored. Wi-Fi
signal and bitrate, cellular signal quality, and eBPF drop counts are next, and none
of them are probe outcomes either.

The glossary planned for counter readings to be stored raw, with rates computed at
query time. That makes every query do arithmetic over counter resets, and makes
rollups depend on the counter's history instead of on the interval alone.

## Decision

- **A reading is one gauge value** of one role at one wall time, in one identity
  epoch, like a sample. A **gauge** is a named quantity with a fixed unit: receive
  and transmit throughput (bit/s) first, then Wi-Fi and cellular values. The `Gauge`
  type lives in the shared crate, with its key, label, and unit.
- **Rates are computed when the counter is read**, once a second, by
  `netwatch_host_health::Throughput`, which uses the verified `counter_increase`
  so that a counter reset is a gap, never negative traffic. Counter readings are no
  longer stored.
- **History stores readings beside samples**, with the same retention, and rolls them
  up into the same tiers. A reading rollup keeps count, sum, min, and max. Its merge
  is addition and min/max, so it is associative and commutative by construction,
  and property-tested rather than proven.
- **The API serves readings and their rollups by role**, lists every gauge with its
  label and unit, and pushes readings live. A row draws one chart per unit, one line
  per gauge. The units say how to format values, so formatting stays presentation.

## Consequences

- The NIC row graphs throughput now, and Wi-Fi and cellular gauges plug into the same
  path.
- Readings add two rows a second for the NIC, about 1.2 million rows over the 7 days
  of raw retention.
- A rate is fixed at the 1 s interval it was read over. A query can't recompute it
  over a different interval, but a rollup's mean is the same average rate.
