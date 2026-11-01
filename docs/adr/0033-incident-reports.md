# 33. Incident reports for an ISP

Date: 2026-11-01

## Status

Accepted.

## Context

When the connection misbehaves, the person running netwatch writes to their ISP's
network operations center (NOC). A NOC acts on specific evidence: when it happened,
for how long, where on the path the loss began, and the addresses of the routers
involved. Screenshots of graphs carry little of that, and a list of incidents per
role repeats one outage once for every target it touched.

## Decision

**netwatch writes a report for any time span**, at `GET /report?from_us=…&until_us=…`
(a printable HTML page) and `GET /report.txt?…` (plain text for an email body). The
UI's *Report* button opens it for the range on screen, or for the zoom.

**Overlapping incidents are one episode.** An *episode* is a connected group of
incidents whose bands overlap. Its severity is the worst of them, its probable cause
is the most upstream role among them, and it lists every role it affected. Each
episode is placed:
- *on this host or LAN* if its probable cause is the kernel, NIC, tunnel, local
  interface, or hop 1 (the gateway)
- *beyond the gateway* otherwise. These are the episodes a NOC can act on, and the
  report lists them first.

**Each episode carries path evidence**: for every role on the affected paths, its
identity (the router's address, for hops), probes sent, loss, and median RTT during
the episode, laid out like `mtr`. The report also gives a summary for the whole span:
each role's loss and RTT percentiles, and the host's current identities (interface,
local address, gateway).

**The daemon builds the report.** Grouping, placement, and statistics are pure
functions (`netwatch_adapter_web::report`) over stored history, tested like the rest
of the domain logic. They read raw samples where they still exist, and minute or hour
rollups beyond that, so a report covers all of retained history. The UI only opens
the page.

## Consequences

- One report states what the graphs show, in words a NOC can act on, and needs no
  account or attachment beyond the page itself.
- Path evidence older than 7 days comes from rollups, so for old episodes it is
  per-minute, not per-probe.
- Hop addresses are the routers that answered, which on many ISPs are private or
  CGNAT addresses. The report shows them as they are.
