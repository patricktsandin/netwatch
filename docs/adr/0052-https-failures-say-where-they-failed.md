# 52. HTTPS failures say where they failed, over either family

Date: 2026-11-02

## Status

Accepted. Supersedes ADR-0028's "a failure before any response: `SendFailed`" and
its IPv4-only connection.

## Context

ADR-0028 recorded every HTTPS exchange that failed before a response as
`SendFailed`. Most such failures aren't failures to send. A name that doesn't
resolve, a refused connection, and a certificate a captive portal forged all
happen beyond this host. Stored as `send-failed`, they read in the report as "could
not be sent", which points a NOC at the host rather than the remote end, against
ADR-0004's distinct outcomes.

The exchange was also IPv4 only: a host with only AAAA records failed every probe,
although every probe type must support both families.

## Decision

- **A new outcome, `ExchangeFailed { stage }`** (stored as `exchange-failed`, with
  the stage as its code), for an exchange that failed beyond this host. The stage is
  where it failed: resolving the name (1), connecting (2), the TLS handshake (3), or
  the HTTP request (4).
- **`SendFailed` stays for local failures**: the socket couldn't be created or
  pinned to the interface.
- **Either family**: the probe connects to the first address the resolver gives,
  IPv4 or IPv6.
- Migration 0011 (ADR-0049), not yet released, admits the new reason too.

## Consequences

- The report names the stage: "HTTPS exchange failed in the TLS handshake".
- Which address family is used follows the system resolver's order (RFC 6724),
  as any client on this host would.
