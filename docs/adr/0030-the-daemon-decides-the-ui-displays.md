# 30. The daemon decides; the UI displays

Date: 2026-10-31

## Status

Accepted. Narrows ADR-0005 and ADR-0019.

## Context

A review asked whether all the logic lives in the backend. It didn't. The frontend
was making domain decisions of its own:
- grading each row as healthy, degraded, failing, or unknown from its recent samples
- applying ADR-0004's rule that hop-only loss is rate limiting, not loss, a rule that
  existed nowhere in Rust
- combining a layer's probes with its health report
- reconstructing every role's current identity by replaying epoch events, although
  the topology aggregate already holds its bindings
- listing disabled sources by replaying events, which showed one entry per daemon
  restart instead of the current state

Any other consumer of the API (alerting, an export) would have seen different answers,
and none of these rules were specified or tested where the data is.

## Decision

- **Grading moves to Probing** (`netwatch_probing::health`, pure and tested):
  - `grade` judges a role from its last 12 outcomes.
  - `along_paths` applies the rate-limit rule across every target's path.
  - `Health::worse` combines a role's probes with its layer report.
- **The composition root decides.** It keeps each role's recent outcomes, grades every
  role once a second, and publishes the statuses (`/api/status`, live `statuses`)
  together with Topology's bindings (`/api/bindings`, live `bindings`). Disabled
  sources are part of the layer report.
- **The frontend only displays.** It may merge transport (live stream and backfill),
  lay out the time axis, and draw. It must not grade, interpret, or reconstruct state
  that the daemon owns. `status.ts` is gone.

## Consequences

- One answer for every client, and the rate-limit rule now has Rust tests.
- A role is `unknown` when its newest outcome is more than 30 s old. The UI no longer
  needs a per-range staleness rule, because status is always "now".
- The UI's statuses update once a second, rather than on every sample.
