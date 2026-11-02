# 45. One row per shared hop stream

Date: 2026-11-02

## Status

Accepted. Refines ADR-0005's "one row per layer" for hops.

## Context

Hops are roles per target (ADR-0002), so that paths, incidents, and probable causes
are reasoned about per target. Since ADR-0026 and ADR-0043, one probe stream feeds
several of those roles: hop 1's stream is copied into every target's hop-1 role,
and hops 2–3 into every target with the same address. With four targets, the graph
page shows hop 1's identical samples four times, and each shared hop 2 or 3 twice.

The data of those rows is identical, but their statuses can differ. Rate limiting is
judged along each target's path (ADR-0026). The same loss at hop 2 is
`rate-limited` toward a target that is healthy, and `degraded` toward one that
isn't.

## Decision

- **The graph page draws one row per shared hop stream** (glossary: *row*). Its
  members are the hop roles the stream serves, as `netwatch_probing::sharing` plans
  them. A row's samples are its owner role's: the members' samples are copies.
- **The daemon decides** (ADR-0030):
  - `GET /api/rows` lists every row with more than one member, its members, and its
    status.
  - Each grading's `statuses` message also carries every such row's status: the
    worst of its members' statuses.
  - The UI never works out a row's members or its status.
- **A shared row shows:**
  - a title that names every target it leads to
  - its worst status, with each target's own status on hover when they differ
  - the incident bands of all its members, with each band drawn once and its
    tooltip naming the target's path it belongs to
  - the event markers of all its members
- **Nothing else changes.** Storage, rollups, incidents, reports, and the network
  map stay per role.

## Consequences

- With four ICMP and DNS targets toward two addresses, 12 hop rows become 5.
- The same router reached by different streams keeps one row per stream. For
  example, hop 2 toward 1.1.1.1 and toward 8.8.8.8 are separate rows. Their samples
  differ, and a router that answers one stream but not the other is evidence of
  rate limiting. The network map already shows that router once.
- Shared samples are still stored once per member role. Storing them once would
  change history, rollups, and reports, and is left for a later decision.
