# 27. DNS probes: `. IN NS` over a pinned UDP socket

Date: 2026-10-31

## Status

Accepted. Narrows ADR-0005's choice of `hickory-proto` for building DNS queries.

## Context

ADR-0009 calls for DNS probes to the public resolvers and to the system resolver. The
probe has to measure the resolver, not the health of some domain behind it. A DNS
server's error answer (SERVFAIL, REFUSED) is neither a timeout nor unreachable.

## Decision

- **The question:** every probe asks `. IN NS`, the root zone's name servers, which
  every resolver answers from cache. The DNS message ID is the probe's sequence
  number. A response can't carry our token, so matching relies on the sequence
  window (ADR-0023), together with checking that the response is to exactly our
  question.
- **The codec:** the query (17 bytes) and the response parser are written by hand
  instead of with `hickory-proto`. Only the header and the question need parsing, and
  a parser that small can be proven panic-free with Kani, for every input.
- **The socket:** a UDP socket connected to the server on port 53, pinned with
  `SO_BINDTOIFINDEX` (ADR-0024).
- **Outcomes:** NOERROR is answered. Any other rcode is the new `Refused { code }`
  outcome, persisted as loss `refused` with its code. Migration 0002 rebuilds the
  `samples` table to allow it.
- Targets now have names for people, served at `/api/targets`, and the UI labels rows
  with them.

## Consequences

- DNS RTTs to 1.1.1.1, 8.8.8.8, and 9.9.9.9 are measured every 5 s alongside ICMP.
- Still missing: querying the system resolver (from `/etc/resolv.conf` or
  systemd-resolved's upstream servers), and HTTPS probes.
