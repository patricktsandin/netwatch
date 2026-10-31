# 24. Echo probes over pinned ping sockets

Date: 2026-10-31

## Status

Accepted. Refines how [ADR-0020](0020-pinned-probes-and-topology-resync.md) enforces pins.

## Context

ADR-0020 said pins would be enforced with `IP_UNICAST_IF` and a bound source address.
Since Linux 5.7, `SO_BINDTOIFINDEX` no longer needs privileges, and it is stricter:
- it restricts route lookups to the bound interface, as `IP_UNICAST_IF` does
- it also delivers to the socket only the packets that arrive on that interface

`socket2` exposes it through a safe API.

The shell also needs a monotonic clock that it can turn into domain `Instant`s, and a
way to re-pin a stream without leaving any probe without an outcome.

## Decision

- **Socket:** each echo probe stream gets its own ICMP or ICMPv6 datagram socket,
  bound to the pinned interface with `SO_BINDTOIFINDEX`.
  - Requests are 8 bytes of header plus the 8-byte token. The kernel fills in the
    identifier and the checksum.
  - Replies are parsed by a pure function, which Kani proves never panics and reads
    the right fields, for every input.
- **The pin:** targets are pinned to the egress binding, which is the tunnel when
  there is one and otherwise the NIC.
  - When the pin changes, a new stream starts on a new socket, and the old stream is
    retired: it stops sending, but keeps receiving until every probe it sent has an
    outcome.
  - A sample carries its pin's epoch.
- **Time:** `std::time::Instant` (which reads `CLOCK_MONOTONIC` on Linux), measured
  from daemon start, provides domain `Instant`s.
  - Receive timestamps are taken in userspace when the reply is read. This
    overstates RTT by about 0.3 ms compared to `ping`.
  - Kernel timestamps (`SO_TIMESTAMPING`) are a later improvement.

## Consequences

- Echo probing works unprivileged. Measured against `ping -I eth0` on the development
  host, RTTs agree within the userspace timestamp overhead.
- Still missing: ICMP errors via `IP_RECVERR` (the `Unreachable` outcome, and the
  basis of hop probes), loopback probes, and IPv6 default targets.
