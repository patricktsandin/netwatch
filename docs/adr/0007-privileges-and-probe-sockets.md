# 7. Least-privilege systemd service; prefer unprivileged ping sockets

Date: 2026-10-31

## Status

Accepted

## Context

Network probing traditionally runs as root. Linux offers finer-grained options:
- ICMP datagram sockets (`SOCK_DGRAM, IPPROTO_ICMP`) need no privileges when
  `net.ipv4.ping_group_range` allows it; on Fedora it is `0 2147483647`.
- `IP_RECVERR` / `IPV6_RECVERR` deliver TTL-exceeded errors on those sockets, which is
  how `tracepath` works without privileges.
- Netlink monitoring and reading ethtool statistics are unprivileged.

Some sources do need capabilities:
- `AF_PACKET`, for ARP/NDP probes of the gateway (ADR-0002), needs `CAP_NET_RAW`.
- `/dev/kmsg` needs `CAP_SYSLOG`, because `kernel.dmesg_restrict=1` on Fedora.
- The eBPF drop-reason program needs `CAP_BPF` and `CAP_PERFMON` (ADR-0008).

The deployment options were a systemd service with capabilities, a container, or root.

## Decision

- Run as a systemd service under a dedicated `netwatch` user, granting
  `CAP_NET_RAW`, `CAP_SYSLOG`, `CAP_BPF`, and `CAP_PERFMON` through
  `AmbientCapabilities=` (`setcap` for development builds). Nothing more.
- ICMP and hop probes prefer unprivileged datagram sockets with `IP_RECVERR`, and use
  raw sockets only where a datagram socket can't do the job.
- `CAP_NET_ADMIN` is not granted. Adding any capability requires a new ADR.
- Each capability is optional at runtime. If one is missing, the sources that need it
  are disabled, an event is recorded, and the UI shows those sources as disabled. The
  daemon never fails to start because a capability is missing.
- The UI binds to `127.0.0.1` by default.

## Consequences

- A small attack surface for a daemon that parses untrusted packets. Kani proofs for
  the parsers back this up (ADR-0016).
- At startup, the daemon must detect which socket modes and capabilities are available,
  and log what it found.
- Firewall ruleset monitoring would need `CAP_NET_ADMIN`, so it is out of scope (ADR-0010).
