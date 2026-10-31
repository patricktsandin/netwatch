# 23. Probe matching: tokens where possible, a sequence window otherwise

Date: 2026-10-31

## Status

Accepted

## Context

ICMP echo sequence numbers are 16 bits, so a probe stream reuses them, every 18 hours
at a 1 s cadence. A reply that arrives late can carry a sequence number that now
belongs to a newer probe. Two kinds of reply differ in what they carry:
- **Echo replies** echo the whole payload back.
- **ICMP errors** (unreachable, time exceeded) quote the original packet. Many
  routers quote only the first 8 bytes of ICMP: type, code, checksum, identifier,
  and sequence number, so no payload.

The ICMP identifier doesn't help either: with the kernel's ICMP datagram sockets, the
kernel assigns it and routes replies to the right socket on its own.

## Decision

- **Every probe gets a 64-bit index** within its stream. It is sent in the echo
  payload as a token, and its low bits are the sequence number.
- **Matching:** a reply matches the in-flight probe with its sequence number. If the
  reply carries a token, the token must equal that probe's index. A reply that
  matches nothing (late, duplicated, or foreign) is ignored.
- **The window is part of the type:** `StreamConfig` can only be constructed when
  `timeout < sequence space × cadence`. With that, expiring timed-out probes before
  each send means a sequence number is never reused while it is still in flight.
- **One stream per socket**, so the kernel's identifier routing separates the streams.
- **A tokenless reply** (an ICMP error) is attributed correctly only if it arrives
  within the reuse window. `specs/probing.qnt` checks this condition rather than
  assuming it: allow replies older than the window and the simulator finds a
  misattribution immediately. Matching that ignores tokens fails the same way.
- **Catching up after a stall:** a stream that falls more than one cadence behind (a
  stalled timer, or a suspend) skips the missed slots and keeps its phase, instead of
  sending a burst. The skipped slots show up as timer lateness.

## Consequences

- Echo probes are matched exactly. Error replies are matched exactly whenever they
  arrive within the reuse window (18 h at a 1 s cadence), which in practice is always.
- The window rule shows up as a validation error at configuration time, never as a
  bug at runtime.
- The verified crate proves that indices less than one sequence space apart never
  share a sequence number.
