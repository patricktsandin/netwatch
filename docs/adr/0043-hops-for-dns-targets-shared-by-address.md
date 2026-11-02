# 43. Hops for DNS targets, with hop streams shared by destination address

Date: 2026-11-02

## Status

Accepted. Supersedes ADR-0025's "hop probes … to each target" for which targets get
hops, and extends ADR-0026's sharing to hops 2 and 3.

## Context

Hops 1–3 are probed only for ICMP targets. No ADR chose that: hop probes (ADR-0025)
came before DNS targets (ADR-0027), and nothing extended them. As a result:
- A DNS target has no hop rows, and no hop incidents. Probable causes can't point
  at a router on its path, and its loss can't be told apart from rate limiting.
- The network map (ADR-0042) joins a DNS target's path from the host straight to
  its first trace hop, by an unmeasured edge.

Giving every target its own hop streams would multiply the probes the ISP's first
routers answer. ADR-0026 showed what that does: a router's ICMP rate limit drops
answers, and the UI shows loss that isn't there. In the default configuration the
DNS targets probe the same addresses as ICMP targets (1.1.1.1, 8.8.8.8, 9.9.9.9),
so separate streams would probe the same routers on the same path twice over.

## Decision

- **DNS targets get hops 1–3.** A DNS target's server address is fixed (`system` is
  resolved when the configuration loads), so its hops are probed like an ICMP
  target's. HTTPS targets still get none: their address comes from DNS on every
  probe (ADR-0028), so there is no fixed address to probe toward.
- **Hop probes stay ICMP echoes**, whatever the target's kind. A DNS target's hops
  are the ICMP path toward its server.
- **One stream per hop and shared key** (glossary: *stream sharing*):
  - Hop 1 is shared by every target with hops in the same address family, as
    ADR-0026 decided.
  - Hops 2 and 3 are shared by every target with hops toward the same destination
    address. The same address from the same pin is the same route lookup, so the
    targets' paths coincide as far as the routing table can tell.

  A shared stream's resolutions are copied to every target's hop role. Each role
  keeps its own epoch and its own samples, so history, incidents, and the map are
  per target as before.
- **The plan is a pure function** (`netwatch_probing::sharing`): given the targets
  with hops and their addresses, it gives each stream's TTL, destination, and the
  targets it serves. The composition root only runs it. It is a function, not a
  state machine, so it has behaviour tests and properties, not a Quint spec
  (ADR-0016):
  - every hop role is served by exactly one stream
  - no two streams have the same TTL and the same key

## Consequences

- With the default targets, DNS targets add no probes: each one shares the streams
  of the ICMP target with its address. A DNS server without an ICMP twin, such as
  `system`, adds one stream each for hops 2 and 3.
- An ICMP probe and a DNS query (UDP port 53) to one address can take different
  routes where routers balance load per flow. A DNS target's hops are then the
  ICMP route, which is nearly always, but not certainly, the route of its queries.
  The user manual says so. Probing hops with TTL-limited DNS queries would measure
  the queries' own route. It is left for later, if the two are ever seen to differ.
- Sharing by address is static. Sharing by responding addresses, so that targets
  whose paths merge after hop 1 share hops 2 and 3 too (ADR-0002), remains to do.
- Each DNS target gains three hop rows in the UI and in reports.
