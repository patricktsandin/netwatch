# Architecture Decision Records

Nygard format; see [ADR-0001](0001-record-architecture-decisions.md). Accepted ADRs
are never edited. A new ADR supersedes an old one, and the old one's status changes
to "Superseded".

| # | Decision | Status |
|---|---|---|
| [0001](0001-record-architecture-decisions.md) | Record architecture decisions | Accepted |
| [0002](0002-monitored-layers-as-logical-roles.md) | Monitored layers are logical roles with changing identities | Accepted |
| [0003](0003-follow-topology-via-rtnetlink.md) | Follow topology changes via rtnetlink, with identity epochs | Accepted |
| [0004](0004-measurement-integrity.md) | Measurement integrity rules | Accepted |
| [0005](0005-rust-daemon-with-embedded-spa.md) | Rust daemon serving an embedded TypeScript SPA | Accepted |
| [0006](0006-sqlite-storage-with-rollups.md) | Embedded SQLite with tiered rollups | Accepted |
| [0007](0007-privileges-and-probe-sockets.md) | Least-privilege systemd service; prefer unprivileged ping sockets | Accepted |
| [0008](0008-kernel-layer-sources.md) | Kernel layer sources | Accepted |
| [0009](0009-default-targets-and-cadence.md) | Default targets and probe cadence | Accepted |
| [0010](0010-v1-scope.md) | v1 scope | Accepted |
| [0011](0011-ousterhout-as-design-authority.md) | *A Philosophy of Software Design* settles design conflicts | Accepted |
| [0012](0012-sans-io-core-hexagonal-shell.md) | Sans-IO domain core inside a hexagonal shell | Accepted |
| [0013](0013-domain-driven-design.md) | Strategic DDD, with tactical patterns only where they earn their place | Accepted |
| [0014](0014-type-driven-domain-modeling.md) | Type-driven domain modeling | Accepted |
| [0015](0015-define-errors-out-of-existence.md) | Define errors out of existence | Accepted |
| [0016](0016-layered-formal-verification.md) | Layered formal verification | Accepted |
| [0017](0017-classicist-test-first.md) | Classicist, test-first testing | Accepted |
| [0018](0018-feature-workflow.md) | Feature workflow: spec, types, tests, code, proofs | Accepted |
| [0019](0019-frontend-verification.md) | Frontend verification | Accepted |
| [0020](0020-pinned-probes-and-topology-resync.md) | Pin probes to identity epochs; resynchronize after lost notifications | Accepted; refined by 0021 |
| [0021](0021-buffer-and-drain-during-resync.md) | Buffer notifications while desynchronized; drain the socket on loss | Accepted |
| [0022](0022-global-roles-from-reference-egress.md) | Global roles follow the egress path to a reference destination | Accepted |
| [0023](0023-probe-matching-tokens-and-sequence-window.md) | Probe matching: tokens where possible, a sequence window otherwise | Accepted |
| [0024](0024-echo-probes-over-pinned-ping-sockets.md) | Echo probes over pinned ping sockets | Accepted |
| [0025](0025-hops-identified-by-probing.md) | Hops are identified by Probing, through ICMP errors | Accepted |
| [0026](0026-shared-first-hop-and-rate-limit-display.md) | Share the first hop across targets; show hop-only loss as rate limiting | Accepted |
| [0027](0027-dns-probes.md) | DNS probes: `. IN NS` over a pinned UDP socket | Accepted |
| [0028](0028-https-probes-with-phase-timings.md) | HTTPS probes with phase timings | Accepted |
| [0029](0029-flush-barrier-before-backfill.md) | Flush the writer before every backfill | Accepted |
| [0030](0030-the-daemon-decides-the-ui-displays.md) | The daemon decides; the UI displays | Accepted |
| [0031](0031-incidents-and-event-relevance.md) | Incidents, and the roles an event concerns | Accepted |
| [0032](0032-gauge-readings.md) | Gauge readings: graphs for what isn't a probe | Accepted |
| [0033](0033-incident-reports.md) | Incident reports for an ISP | Accepted |
| [0034](0034-causes-come-first.md) | A probable cause comes first, and episodes follow causes | Accepted |
| [0035](0035-configuration-from-the-ui.md) | Configuring targets from the UI | Accepted |
| [0036](0036-radio-roles.md) | Radio roles: Wi-Fi and cellular, carrying traffic or standing by | Accepted |
| [0037](0037-drop-reasons-by-reason-only.md) | Drop reasons are counted by reason only | Accepted |
| [0038](0038-flushing-history.md) | Flushing all history from the UI | Accepted |
