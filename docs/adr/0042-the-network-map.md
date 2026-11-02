# 42. The network map

Date: 2026-11-02

## Status

Accepted. Builds on ADR-0025 (hops), ADR-0030 (the daemon decides), and ADR-0040
(path traces).

## Context

The main screen lists roles one row per role. It shows each role's history well, but
not how the roles connect. The same router can appear on several targets' paths:
hop 1 is shared, and an ISP's core routers usually are too. When several rows turn
red at once, people have to work out by hand whether the paths share the failing
router.

The daemon already knows everything needed to draw the paths:
- each hop's responding address (its identity), from Topology
- every role's status, from the grading (ADR-0030)
- each target's latest path trace beyond hop 3 (ADR-0040)

## Decision

**A *Map* tab beside the graphs shows the *network map*.** The map is a directed
acyclic graph of every path from this host to every target. Each node shows its
status.

**The daemon builds the map** (`netwatch_probing::map`), as a pure function of the
statuses, the hop identities, and each target's latest trace. It rebuilds the map
after each grading and publishes it on the live stream only when it has changed. The
UI lays it out and draws it, and decides nothing (ADR-0030).

**Nodes** (glossary: *map node*):
- **The host's layers:** the kernel, NIC, tunnel, and local interface, one node each,
  in path order.
- **The radios:** one node each, joined to the kernel by an *off-path* edge, because
  they carry no probes (ADR-0036).
- **Routers:** a hop is identified by its responding address and its TTL. Hops of
  different targets with the same address and TTL are one node, so paths that share
  a router meet there. Without the TTL, two paths could cross the same two routers in
  opposite orders and form a cycle.
- **Unknown hops:** a hop with no responding address yet is a node of its own.
- **Trace hops:** each TTL past the last continuously measured hop, from the target's
  latest trace (routine or episode). Each responder is a router node, identified the
  same way. A TTL where several addresses answered (load balancing) gets one node
  per address. A TTL where nobody answered gets a *silent* node of its own, for that
  target only.
- **Targets:** one node each, at the TTL where its trace reached it. Without a
  trace that reached it, a target sits one step past the last hop known.

**Edges** join consecutive positions on each target's path. Every node at one
position joins every node at the next. Each edge lists the targets whose paths use
it. An edge is one of four kinds:
- **Probed:** both ends are measured continuously.
- **Traced:** at least one end comes only from a trace.
- **Unmeasured:** hops between its ends are unknown. Examples: a DNS target has no
  continuous hops, and an ICMP target before its first trace runs.
- **Off-path:** a radio's edge, described above.

**Status:**
- A node's status is the worst status among the roles merged into it. The node also
  lists each of those roles with its own status, so a router that is rate-limited on
  one path and degraded on another shows both.
- A trace-only node has no status, because nothing measures it continuously. It
  shows what its trace found: when the trace ran, how many probes were answered, and
  the median RTT.

**Depth.** Each node has a depth along the path: the kernel is 0, the NIC 1, the
tunnel 2, the local interface 3, and a hop at TTL *n* is 3 + *n*. Every edge goes to
a strictly greater depth, which keeps the map acyclic, and the UI draws one depth
per row. A property test checks this for arbitrary inputs.

**There is no Quint spec.** The map is a function, not a state machine (ADR-0016).
Its properties are tested with `proptest` instead:
- it is acyclic
- every target has a path from the kernel
- every role appears in exactly one node

## Consequences

- The map shows paths as they are now, not as they were. History stays on the
  graphs and in the report.
- A load-balanced TTL joins every responder to every responder at the next TTL. This
  shows more paths than one flow can actually take. A trace can't tell which
  responder leads to which.
- Beyond hop 3, the map is only as fresh as the latest trace, which can be up to 6 h
  old. Each trace node says when its trace ran.
- HTTPS targets have no trace (ADR-0040). Each one hangs off the local interface by
  an unmeasured edge.
- The map is sent only when it changes, so a stable network adds nothing to the live
  stream. Statuses change it at most once a second.
