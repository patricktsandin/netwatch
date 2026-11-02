# 47. A nearby target follows its router; what a stop left open

Date: 2026-11-02

## Status

Accepted. Supersedes ADR-0046's rule for targets reached at a hop, and states
ADR-0044's "before the start" precisely.

## Context

A second code review found two flaws in the previous round's fixes:
- **ADR-0046 merged the hop where a target's own address answered into the target's
  node.** Other targets' paths through the same router, at the same TTL, still made a
  router node of it. The gateway was drawn twice when the system resolver is the
  gateway. That breaks ADR-0042's rule that one address at one TTL is one node.
- **Which epochs can resume** was decided by whether an `epoch-ended` event exists.
  Flushing history keeps the highest epoch but deletes every event (ADR-0038), so an
  epoch that had really ended looked as if the stop had ended it.

## Decision

- **Where a target's own address answers one of its hops, that hop stays the router
  node every path through it shares. The target's node follows it**, one step deeper.
  The target's later hops merge into the target's node: they measure the target
  itself. A trace hop answered by the target's address still places the target at
  that TTL.
- **An epoch can resume only if the stop left it open**: its `ended_us` is unset when
  the daemon starts. The writer reads each role's open epoch, and the last public
  address, as it starts, before it closes those epochs. Events play no part, so a
  flush can't change the answer.

## Consequences

- A resolver at the gateway is drawn as the gateway's router node, followed by the
  resolver's target node. The address appears twice, but each node means one thing:
  the router on every path, and the target.
