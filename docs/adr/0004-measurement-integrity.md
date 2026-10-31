# 4. Measurement integrity rules

Date: 2026-10-31

## Status

Accepted

## Context

A monitor that reports wrong numbers is worse than no monitor, because people act on
it. Several common mistakes lead to wrong numbers:
- Wall-clock time jumps, from NTP steps and suspend.
- Probes scheduled relative to the last reply, which hides stalls (coordinated omission).
- Collapsing every failure into a NULL round-trip time.
- Matching replies to probes by the order they arrive.
- Treating ICMP rate limiting at an intermediate hop as packet loss.

## Decision

- Measure every duration with a monotonic clock. Use wall-clock time only to timestamp
  samples, stored as UTC Unix microseconds.
- Use kernel receive timestamps (`SO_TIMESTAMPING`) when available, and record which
  timestamp source each sample used.
- Probes run on a fixed cadence. Record how late the timer fired; this feeds the
  kernel layer (ADR-0008).
- Timeout, ICMP error (unreachable, admin-prohibited, …), and local send failure are
  distinct outcomes, stored as distinct result codes.
- Match replies to probes by identifier, sequence number, and the embedded original
  header.
- Loss at an intermediate hop that doesn't carry through to later hops is shown as
  likely rate limiting, never alarmed on as loss.

## Consequences

- The numbers can be trusted. Reply matching and sequence-number wraparound become
  targets for formal proof (ADR-0016).
- The UI needs a way to show loss that is likely rate limiting.
- Recording the timestamp source and timer lateness costs some extra storage.
