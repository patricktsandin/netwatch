# 67. No CAP_NET_RAW

Date: 2026-11-03

## Status

Accepted. Supersedes ADR-0007's grant of `CAP_NET_RAW`.

## Context

ADR-0007 granted `CAP_NET_RAW` for one source: ARP and neighbor-solicitation probes
of the gateway over `AF_PACKET` (ADR-0002's link-layer probe). That source was never
built. Every probe netwatch sends uses an unprivileged socket: ICMP datagram sockets
(allowed by `net.ipv4.ping_group_range`), UDP for DNS, and TCP for HTTPS. So the
service held a capability, and the `AF_PACKET` address family, that nothing used. A
review found the grant; the user chose to remove it.

## Decision

- **The service gets `CAP_SYSLOG`, `CAP_BPF`, and `CAP_PERFMON` only**, each still
  optional at runtime (ADR-0007), and `AF_PACKET` is no longer among its allowed
  address families. The container instructions drop it too.
- Gateway link-layer probes stay unbuilt. Building them needs a new ADR that grants
  `CAP_NET_RAW` again, as adding any capability does.

## Consequences

- A smaller grant for a daemon that parses untrusted packets.
- Nothing measured changes.
