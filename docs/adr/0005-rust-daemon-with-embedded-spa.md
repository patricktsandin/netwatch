# 5. Rust daemon serving an embedded TypeScript SPA

Date: 2026-10-31

## Status

Accepted

## Context

The collector needs raw and datagram ICMP sockets, `AF_PACKET`, netlink, eBPF, precise
timing, and many concurrent probes. The candidates were:
- **Go:** good netlink and ICMP libraries, and quick to iterate on.
- **Rust:** `rtnetlink`, `socket2`, `aya`, and `tokio`, with tighter latency jitter and
  a type system that suits the type-driven design in ADR-0014.
- **Python:** already installed on the development host, but its timer jitter blurs
  sub-millisecond measurements.

For the UI, the candidates were an embedded SPA, server-rendered HTML with htmx, or a
separately deployed frontend.

## Decision

- **Backend:** Rust stable, edition 2024. `tokio` and `axum` (HTTP and WebSocket);
  `rtnetlink` / `netlink-packet-route` with ethtool netlink and nl80211; `socket2` and
  `nix` for probe sockets; `hickory-proto` for building DNS queries; `hyper` and
  `rustls` directly for HTTP probes, because high-level clients hide phase timings;
  `rusqlite` with bundled SQLite; `zbus` for ModemManager; `aya` for eBPF; `tracing`,
  `serde`, `thiserror`, and `uom`.
- **Frontend:** TypeScript, Vite, and uPlot, built to static assets and embedded in the
  binary with `rust-embed`. Live data comes over WebSocket; history over a JSON REST API.
- **Main screen:** a stack with one row per layer, from kernel down to targets. Each
  row shows a status, a sparkline, and current loss/latency. All rows share one time
  axis, with topology events drawn as vertical markers through every row, and
  synchronized zoom and pan. Clicking a row opens its detailed graphs.
- Config is a TOML file. Targets changed at runtime through the API are persisted.

## Consequences

- One binary to deploy. The type system carries a lot of the domain modeling.
- Iteration is slower than in Go or Python, which is acceptable given the emphasis on
  correctness.
- Two toolchains (Rust and Node). The SPA must be built before a release build embeds it.
- uPlot keeps rendering cheap with long histories and many rows.
