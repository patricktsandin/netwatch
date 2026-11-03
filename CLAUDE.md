# CLAUDE.md

**netwatch** is a continuous, mtr-style network monitor delivered as a web app. A Rust
daemon probes this host's path to the network layer by layer (kernel, NIC, tunnel, local
interface, hops 1–3 per target, and ICMP/DNS/HTTP(S) targets), follows every change in
topology, stores samples in SQLite, and serves an embedded TypeScript SPA with graphs.

The design lives in `docs/adr/` ([index](docs/adr/README.md)). Read the relevant ADRs
before working on an area. This file holds the working rules. The reasons for them are in
the ADRs.

## Status (keep this current)

Working end to end: `NETWATCH_LISTEN=127.0.0.1:8733 cargo run -p netwatch`, then open
<http://127.0.0.1:8733> (8732 is the installed service's).
Configuration: the file `NETWATCH_CONFIG` names (the service's unit sets
`/etc/netwatch/netwatch.toml`), or else the built-in `deploy/netwatch.toml`. A dev build
never finds the service's file or database by itself.
- **Topology:** NIC, tunnel, and local interface followed via rtnetlink, with resync
  after `ENOBUFS`. Epochs persist across restarts.
- **Probing:**
  - ICMP echo (IPv4 and IPv6) and DNS (`. IN NS`) to targets every 5 s, HTTPS every
    10 s.
  - Hops 1–3 per ICMP and DNS target every 1 s via `IP_RECVERR`, as ICMP echoes.
    Hop 1 is shared per address family, hops 2–3 per destination address
    (`netwatch_probing::sharing`, ADR-0043).
  - Loopback baseline every 1 s.
- **Host health:**
  - Kernel: snmp/netstat/softnet counters, PSI, conntrack, and neighbor tables.
  - NIC: `IFLA_STATS64` counters.
  - Both graded every 1 s.
  - A restart starts new epochs, but a role back with the same identity is
    `epoch-resumed`, which no row marks; one `started` marker goes on the kernel row
    (ADR-0044).
  - Kernel log lines (with `CAP_SYSLOG`), wall-clock jumps, and network sysctl
    changes become event markers. Clock sync state comes from `adjtimex`.
- **History:** raw samples and gauge readings (7 days), plus 1m (90 days) and 1h
  (forever) rollups, merged with the verified algebra. NIC throughput is a gauge.
- **Incidents** (ADR-0031, ADR-0034): every grading opens, escalates, and closes
  incidents, with a probable cause that began first. They are stored forever, and
  `specs/incidents.qnt` holds the tracker.
- **Incident reports** (ADR-0033, ADR-0039, ADR-0040): `/report`, `/report.txt`, and
  `/report.csv`. They cover:
  - ticket details from the dialog (never stored)
  - the host, clock, connection, first public hop, public identity, and a summary
    with availability
  - episodes along cause links, with path evidence and router names, baselines, loss
    reasons, loss bursts and jitter, link load, path traces beside the routine trace,
    and timelines
  - loss by hour, the time without a way out, normal paths, events, and the method
- **Path traces** (ADR-0040): TTLs 4–30 toward every ICMP and DNS target, as a
  routine trace every 6 h and an episode trace when an incident opens beyond hop 1
  (10-minute cooldown), stored forever. `specs/traces.qnt` holds the schedule.
- **Router names** (ADR-0040): PTR and Team Cymru origin AS for every hop and trace
  responder, from the system resolver, cached for 1 h, shown in hop row titles.
  `asn_lookups = false` turns off the Cymru queries.
- **Host facts** (ADR-0039): the public identity from Cloudflare's trace every 5 min
  (in the header), and the NIC's link facts and carrier flaps.
- **Settings** (ADR-0035): targets are editable in the UI. The daemon rewrites its
  config file in place, then re-executes itself.
- **UI:**
  - A layered stack with 1 h / 24 h / 30 d ranges, an RTT chart per role, and a
    gauge chart per unit.
  - A live stream with backfill behind a flush barrier.
  - Incident bands, event markers only on the rows they concern, hover tooltips, and
    rate-limit-aware hop status.
  - One row per shared hop stream (ADR-0045), from `/api/rows`, with the worst of
    its members' statuses.
  - Report and Settings dialogs. Settings can also flush all history (ADR-0038).
  - A Map tab (ADR-0042): the network map, a DAG of every path with routers shared
    across targets merged, each node's status, and trace hops beyond hop 3. The
    daemon builds it (`netwatch_probing::map`) and sends it when it changes.
  - An Auto / Light / Dark theme switch, remembered per browser, which the report
    page follows too.
- **Specs:** `topology.qnt`, `probing.qnt`, `live_backfill.qnt`, `incidents.qnt`,
  and `traces.qnt`, all TLC-exhaustive and conformance-tested.

- **Packaging** (ADR-0041): the RPM (`packaging/build-rpm.sh`), an install script
  for other systemd distros (`packaging/install.sh`), and a container image
  (`packaging/build-container.sh`, run with `--network host`). The RPM and image
  build from HEAD.

- **Radios** (ADR-0036): `wifi` and `cellular` roles, off the path, graded every 5 s
  from nl80211 and ModemManager, with gauges and attachment-change events.
- **Drop reasons** (ADR-0037): an eBPF program on `tp_btf/kfree_skb`, counted by
  reason and named from BTF, feeding the kernel layer. Build it with nightly and
  `bpf-linker` (aya-build runs from `netwatch-adapter-ebpf`'s build script; it is the
  one non-default workspace member). The capability test runs with `--ignored`.

Deliberately left unbuilt for now:
- gateway link-layer probes (ARP/NDP), which would need `CAP_NET_RAW` again (ADR-0067)
- persisting and graphing HTTPS phase timings, and the `colo=` identity
- IPv6 egress for the global roles (IPv6 targets are pinned to the IPv4 egress NIC)
- ethtool driver stats and qdisc stats
- percentile bands on the rollup charts, and pan/zoom beyond the selected range
- sharing hops 2–3 by responding address rather than destination (ADR-0002,
  ADR-0043), and the tunnel underlay (ADR-0022)
- kernel receive timestamps (`SO_TIMESTAMPING`)
- path traces of HTTPS targets, large-packet (MTU) probes, and TCP retransmissions
  in the report

Two daemons on one host (say, the service and a dev build) share the gateway's ICMP
rate limit, about one reply per second, so one of them sees hop 1 go silent.

## Non-negotiables

- **Measurement integrity** ([0004](docs/adr/0004-measurement-integrity.md)):
  - Use monotonic clocks for durations. Use wall-clock time only for timestamps.
  - Probes run on a fixed cadence.
  - Timeouts, ICMP errors, and send failures are distinct outcomes, never a NULL RTT.
  - Match replies by identifier, sequence, and embedded header, never by arrival order.
  - Rate limiting at a hop is not loss.
- **Identities** ([0002](docs/adr/0002-monitored-layers-as-logical-roles.md),
  [0003](docs/adr/0003-follow-topology-via-rtnetlink.md)):
  - Series are keyed by role. Every sample carries its identity and identity epoch.
  - Never aggregate across epochs.
  - rtnetlink is the source of truth. Never parse `ip` or `nmcli` output.
- **Privileges** ([0007](docs/adr/0007-privileges-and-probe-sockets.md),
  [0067](docs/adr/0067-no-cap-net-raw.md)):
  - Only `CAP_SYSLOG`, `CAP_BPF`, and `CAP_PERFMON`, and each one is
    optional at runtime: if it's missing, disable the dependent source and record an
    event. Never fail to start.
  - Never add a capability without an ADR.
- **The daemon decides, the UI displays** ([0030](docs/adr/0030-the-daemon-decides-the-ui-displays.md)):
  - Statuses, interpretations (such as rate limiting), current identities, and
    disabled sources come from the daemon.
  - The frontend only merges transport and draws.
- **Scope** ([0010](docs/adr/0010-v1-scope.md)): don't build toward alerting, Prometheus
  export, remote access, multi-host, or firewall monitoring.

## Code rules

- **Design** ([0011](docs/adr/0011-ousterhout-as-design-authority.md)):
  - Ousterhout settles design conflicts. Prefer deep modules, with no pass-through
    layers or rename-only wrappers.
  - Every module and public item gets an interface comment (the why, the invariants,
    what it hides).
  - Implementation comments explain why, never what.
- **Architecture** ([0012](docs/adr/0012-sans-io-core-hexagonal-shell.md)):
  - Domain crates are pure state machines: events in, commands and facts out. No
    `tokio`, `libc`, `async`, clocks, randomness, or I/O.
  - Adapters translate OS data into domain types. Raw kernel structures never reach the
    domain.
- **Language** ([0013](docs/adr/0013-domain-driven-design.md)):
  - Use `docs/glossary.md` terms exactly, everywhere. New concepts go in the glossary
    first.
  - "Kernel" means only the Linux kernel. Say *shared crate* and *verified crate* for
    the other two.
  - The crate layout follows `docs/context-map.md`.
- **Types** ([0014](docs/adr/0014-type-driven-domain-modeling.md)):
  - Parse, don't validate. A newtype for every domain value; no bare integers or
    `String`s across module boundaries.
  - `uom` for quantities. Unit-suffixed integers (`rtt_us`) only at the persistence and
    wire boundaries.
  - Typestate for lifecycles.
- **Errors** ([0015](docs/adr/0015-define-errors-out-of-existence.md)):
  - A failed probe is an outcome, not an `Err`. One small `thiserror` enum per context,
    and `anyhow` only in the binary.
  - Panic only on proven invariants, with a message that names the invariant. No
    `unwrap()` outside tests.
- **Tests** ([0017](docs/adr/0017-classicist-test-first.md)):
  - Write the failing test first, and test behavior through public interfaces.
  - Fakes only at the process boundary; never mock inside the domain.
  - Test names state behavior in glossary terms.
- **Verification** ([0016](docs/adr/0016-layered-formal-verification.md)):
  - A Quint spec plus ITF trace conformance for each core state machine.
  - Verus proofs in `netwatch-verified`.
  - Kani harnesses for every parser of untrusted bytes; `proptest` and `loom` where they apply.
- **Frontend** ([0019](docs/adr/0019-frontend-verification.md)):
  - UI logic lives in TypeScript modules that do no I/O, with Quint specs and ITF
    replay in Vitest.
  - API types are generated by `ts-rs`; never hand-edit them. Payloads are parsed once,
    at the boundary.
  - Strict `tsconfig`: no `any`, no non-null assertions (`!`), and no `as` outside the
    boundary parser. Branded types and exhaustive `never` checks.
  - `fast-check` for properties and model-based tests.
- Every probe type supports IPv4 and IPv6.

## Workflow

Each feature goes through these steps, each as its own commit
([0018](docs/adr/0018-feature-workflow.md)):

**glossary/ADR → Quint spec → types → failing tests → code → proofs**

If a later step shows an earlier one was wrong, go back and fix it there.

- **Significant decisions** get a new ADR ([0001](docs/adr/0001-record-architecture-decisions.md)).
  Accepted ADRs are never edited; a new ADR supersedes the old one.
- **Docs:** update the glossary, context map, and specs in the same commit as the code
  they describe. The same goes for `docs/diagrams/` (schema, crates, data flow) and the
  user manual (anything a user sees). Diagrams are Graphviz sources rendered by
  `npm run diagrams` in `web/`; commit the SVGs with them.
- **Done** means fmt, clippy, tests (including trace conformance), Quint model checking,
  Kani, and Verus all pass, plus `tsc --noEmit`, Vitest, and ESLint for the web app. For UI
  changes, also look at the graphs in a browser.
- **Never make a change pass by weakening it.** Don't weaken a spec property, delete a
  proof, add `assume`, or `#[ignore]` a test. If something can't pass, raise it.

## Layout

```
docs/{glossary.md, context-map.md, adr/}
docs/{overview.md, user-manual.md, diagrams/, images/}   # for readers outside the code
specs/                       # annotated Quint specs
crates/
  netwatch-shared/           # domain: shared value objects and quantities
  netwatch-verified/         # domain: Verus-verified algorithms
  netwatch-topology/         # domain: Topology context
  netwatch-probing/          # domain: Probing context
  netwatch-host-health/      # domain: Host health context
  netwatch-history/          # domain: History context
  netwatch-adapter-{netlink,sockets,procfs,ebpf,modem,sqlite,web}/
  netwatch-ebpf/             # eBPF programs (no_std, nightly)
  netwatch-ebpf-common/      # types shared by the eBPF programs and their loader
  netwatch/                  # composition root binary
web/                         # Vite + TypeScript + uPlot SPA
deploy/                      # systemd unit, sysusers.d, example config
```

## Commands

Toolchains are installed on the dev host, user-local, in `~/.cargo/bin` and `~/.local/bin`:
- **Rust:** `rustup`, stable plus nightly with `rust-src` for the eBPF crate.
- **Node:** 22, from dnf.
- **Quint:** `npm -g` with the prefix set to `~/.local`. Apalache downloads automatically
  on the first `quint verify` and runs on the system JDK.
- **Kani:** installed with `cargo install`, then `cargo kani setup`.
- **bpf-linker:** the prebuilt musl release, unpacked in `~/.local/opt`. `cargo install`
  fails because it needs a system LLVM.
- **Verus:** the release zip, unpacked in `~/.local/opt`. Each Verus release pins an
  exact rustup toolchain, and `verus --version` prints the `rustup install` command for
  it. Install that toolchain whenever you upgrade Verus.

```sh
./scripts/check.sh                                 # everything below, failing fast (~6 min)
cargo build
cargo test                                         # unprivileged tests
cargo test -p netwatch --test netns_failover -- --ignored   # netns tests; they sudo -n themselves
CARGO_TARGET_X86_64_UNKNOWN_LINUX_GNU_RUNNER='sudo -E' \
  cargo test -p netwatch-adapter-ebpf --test privileged -- --ignored   # built as you, run as root
cargo clippy --all-targets -- -D warnings
cargo fmt --check
cd web && npm ci && npm run build                  # needed before a release build embeds it
cd web && npm run dev                              # proxies /api to a dev daemon on 8733
cd web && npm run diagrams                         # docs/diagrams/*.dot → *.svg
cd web && npx tsc --noEmit && npx vitest run && npx eslint .
./scripts/check-specs.sh                           # every spec: typecheck, simulate, exhaustive TLC (~1 min)
quint run specs/<f>.qnt --invariant=<name> --verbosity=3   # print a counterexample trace
cargo kani                                         # all harnesses, ~3 min
cargo verus verify -p netwatch-verified        # Verus proofs; format verified code with `cargo verus fmt`
```
