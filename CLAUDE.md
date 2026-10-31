# CLAUDE.md

**netwatch** is a continuous, mtr-style network monitor delivered as a web app. A Rust
daemon probes this host's path to the network layer by layer (kernel, NIC, tunnel, local
interface, hops 1–3 per target, and ICMP/DNS/HTTP(S) targets), follows every change in
topology, stores samples in SQLite, and serves an embedded TypeScript SPA with graphs.

The design lives in `docs/adr/` ([index](docs/adr/README.md)). Read the relevant ADRs
before working on an area. This file holds the working rules. The reasons for them are in
the ADRs.

## Status (keep this current)

Working end to end: `cargo run -p netwatch`, then open <http://127.0.0.1:8732>.
- **Topology:** NIC, tunnel, and local interface followed via rtnetlink, with
  resync after `ENOBUFS`. Epochs persist across restarts.
- **Probing:** ICMP echo and DNS (`. IN NS`) to targets every 5 s, HTTPS every 10 s. Hops 1–3 per target every 1 s via
  `IP_RECVERR`, with hop 1 shared across targets. Loopback baseline every 1 s.
- **Host health:** kernel (snmp/netstat/softnet, PSI) and NIC (`IFLA_STATS64`) counters, graded every 1 s.
- **History:** raw samples (7 days), plus 1m (90 days) and 1h (forever) rollups merged with the verified algebra.
- **UI:** layered stack with 1 h / 24 h / 30 d ranges, live stream with backfill, epoch markers, and rate-limit-aware
  hop status.

Not yet built, roughly in priority order:
- persisting and graphing HTTPS phase timings, the `colo=` identity, and DNS through the system resolver
- IPv6 egress for the global roles (IPv6 targets are pinned to the IPv4 egress NIC), and IPv6 DNS/HTTPS targets
- ethtool driver stats, Wi-Fi (nl80211), cellular (ModemManager)
- percentile bands on the rollup charts, and pan/zoom into history beyond the selected range
- dynamic stream sharing for hops ≥ 2 (ADR-0026), and the tunnel underlay (ADR-0022)
- eBPF drop reasons, clock health, config drift, and qdisc
  stats (ADR-0008)
- runtime target management through the API (configuration is a TOML file today)
- kernel receive timestamps (`SO_TIMESTAMPING`)

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
- **Privileges** ([0007](docs/adr/0007-privileges-and-probe-sockets.md)):
  - Only `CAP_NET_RAW`, `CAP_SYSLOG`, `CAP_BPF`, and `CAP_PERFMON`, and each one is
    optional at runtime: if it's missing, disable the dependent source and record an
    event. Never fail to start.
  - Never add a capability without an ADR.
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
  they describe.
- **Done** means fmt, clippy, tests (including trace conformance), Quint model checking,
  Kani, and Verus all pass, plus `tsc --noEmit`, Vitest, and ESLint for the web app. For UI
  changes, also look at the graphs in a browser.
- **Never make a change pass by weakening it.** Don't weaken a spec property, delete a
  proof, add `assume`, or `#[ignore]` a test. If something can't pass, raise it.

## Layout

```
docs/{glossary.md, context-map.md, adr/}
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
sudo -E cargo test -- --ignored                    # netns integration tests (by design #[ignore]d)
cargo clippy --all-targets -- -D warnings
cargo fmt --check
cd web && npm ci && npm run build                  # needed before a release build embeds it
cd web && npm run dev                              # proxies /api and /ws to the daemon
cd web && npx tsc --noEmit && npx vitest run && npx eslint .
./scripts/check-specs.sh                           # every spec: typecheck, simulate, exhaustive TLC (~1 min)
quint run specs/<f>.qnt --invariant=<name> --verbosity=3   # print a counterexample trace
cargo kani                                         # all harnesses, ~3 min
cargo verus verify -p netwatch-verified        # Verus proofs; format verified code with `cargo verus fmt`
```
