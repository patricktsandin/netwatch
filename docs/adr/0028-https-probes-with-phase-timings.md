# 28. HTTPS probes with phase timings

Date: 2026-10-31

## Status

Accepted

## Context

ADR-0002 asks HTTPS probes to record phase timings: DNS, connect, TLS, time to first
byte, and total. ADR-0009 says a probe must check the expected response, so that a
captive portal answering 200 counts as a failure. High-level clients hide the phases,
and an HTTPS request is a long-running exchange, not a single packet waiting for a
reply.

## Decision

- **Phases by hand:** each phase is driven explicitly with `tokio`, `tokio-rustls`
  (the ring provider, Mozilla's roots from `webpki-roots`), and hyper's HTTP/1.1
  client connection, timing every step.
- **Pinning:** the TCP socket is pinned with `SO_BINDTODEVICE` (unprivileged since
  Linux 5.7), using the egress pin's interface name.
- **Name resolution** uses the system resolver and isn't pinned. Its time is the DNS
  phase.
- **The same `ProbeStream`:** each `Send` starts a request as its own task, carrying
  the probe's index as its token. A finished request comes back as a reply; a request
  still running when its probe times out is aborted. Scheduling, timeouts, and
  exactly-one-outcome all stay in the verified, model-checked domain code.
- **Outcomes:**
  - the expected response: answered, with the total time as the RTT
  - any other status: the new `UnexpectedResponse { code }` outcome, which migration
    0003 persists with the status
  - a failure before any response: `SendFailed`
- **Default targets:** `https://www.gstatic.com/generate_204` (expects 204),
  `https://cp.cloudflare.com/generate_204` (204), and
  `https://one.one.one.one/cdn-cgi/trace` (200, with `colo=` in the body). Every 10 s,
  with a 5 s timeout.

## Consequences

- HTTPS totals of 27–95 ms on the development host, split into phases.
- The phases are only logged so far. Persisting and graphing them needs a
  multi-value sample, which History doesn't have yet.
- The `colo=` point of presence isn't recorded as an identity attribute yet.
- IPv4 only, like the other default targets.
