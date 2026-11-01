# 40. Path traces, router names, and ticket details

Date: 2026-11-01

## Status

Accepted. Extends ADR-0039.

## Context

ADR-0039's report answers a NOC's first round of questions. The second round is
about *where* in their network, and whether the customer's own traffic is to blame:
- Which routers are those addresses? Router names usually give the city, the device,
  and the interface. An origin AS shows whose network a hop is in, which separates
  "inside our network" from "at a peering point".
- What does the path look like beyond hop 3, and did it change when the problem
  started? Hops 1–3 are measured continuously (ADR-0025), but most ISP cores and
  peerings are further out.
- Was the customer saturating their own link?
- What did the loss look like? Isolated drops, long blackouts, and bursts each point
  to a different fault. Jitter matters for voice.
- Who is the customer, and which ticket is this? A NOC can't act without an account
  number.

## Decision

**Path traces** (glossary: *path trace*). A one-off TTL sweep toward an ICMP or DNS
target's address, over the target's pin:
- **Probes:** TTLs 4 to 30, each probed by one ICMP echo per round, in 3 rounds 1 s
  apart, with a 2 s timeout. Hops 1–3 are left out: their continuous series already
  cover them, and extra probes would share the gateway's ICMP rate limit with them
  (ADR-0026).
- **The result:** for each TTL, who answered, how many probes were answered, and
  their best and median RTT. It ends at the first TTL where the destination itself
  answered. Silent TTLs after the last responder are cut down to one row that says
  no hop answered from there on.
- **When** (the *trace schedule*, `specs/traces.qnt`): a *routine trace* of every
  traceable target when the daemon starts and every 6 h, so there is a recent normal
  path to compare against; and an *episode trace* when an incident opens on a target
  or one of its hops beyond hop 1, at most once per 10 minutes per target. A target
  never has two traces running at once. Durations are measured with the monotonic
  clock.
- **Storage:** a `traces` table, kept forever like incidents. A trace is small, and
  it is evidence. Each trace is also an event on its target's row.
- HTTPS targets aren't traced. Their address comes from DNS on every probe, so it has
  no fixed address to trace toward.

**Router names** (glossary: *router name*). For every hop identity and every trace
responder, the daemon looks up:
- the address's PTR name, from the system resolver
- its origin AS and the AS's name, from Team Cymru's DNS service
  (`origin.asn.cymru.com`, `origin6.asn.cymru.com`, `asn.cymru.com`)

Results are cached for an hour, and failures for 10 minutes. The DNS messages are
encoded and parsed in the sockets adapter. Their parsers take untrusted bytes, so
they are Kani-harnessed, like every other parser. The Cymru text answers are parsed
in Host health. Names are shown next to hop addresses in the UI and the report, and
trace responders' names are stored with the trace, because a name can change later.
`asn_lookups = false` in the configuration turns off the Cymru queries. Those queries
send router addresses (never this host's own) to a third party through the system
resolver.

**The report gains:**
- **Ticket details:** account number, service address, contact, ISP ticket
  reference, and a free-text description, typed into the report dialog. They are
  passed to the report as query parameters, never stored by the daemon, and
  remembered only in the browser.
- **The first public hop:** the first hop whose address isn't private, shared, or
  link-local. The report names it as where the ISP's network most likely starts.
- **Per episode:**
  - link load: the NIC's mean and peak receive and transmit throughput during the
    episode, compared with the link speed
  - for each path role, loss bursts (how many, the longest, and a breakdown by
    length) and jitter (the RFC 3550 mean absolute difference between consecutive
    RTTs)
  - its path traces, with the latest routine trace before it, marking hops whose
    responder changed
- **Path traces:** each routine trace in the span, in an appendix.

## Consequences

- A NOC can place the fault by AS and router name without running its own
  traceroute, and can see whether the path moved.
- Path traces add about 81 probes per target per trace. Episode traces are bounded
  by the 10-minute cooldown.
- Jitter and loss bursts need raw samples, so episodes older than 7 days show none.
- The daemon never stores the customer's ticket details. Every report URL carries
  them, so it is as private as the browser history it lands in.
- Not covered: TCP retransmissions during an episode. Kernel counters are graded but
  not stored as readings. Also not covered: modem line statistics, which need the
  modem.
