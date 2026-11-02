# 48. A shared hop is one thing, for traces and the map too

Date: 2026-11-02

## Status

Accepted. Extends ADR-0043 and ADR-0045.

## Context

A shared hop stream serves several targets' hop roles (ADR-0043). Each role keeps
its own samples and incidents, so a problem at a shared hop 2 opens one incident per
target. Two parts of the daemon still treated those copies as separate things:
- **Episode traces.** Each copy's incident asked the trace schedule for an episode
  trace (ADR-0040). The traces swept the same path toward the same address, so the
  probe load on the routers that rate-limit doubled. Stream sharing exists to avoid
  exactly that.
- **The map.** While a shared hop has no responding address, during the outage a user
  opens the map for, each copy was a node of its own.

## Decision

- **Only the stream owner's hop incidents ask for episode traces.** The copies'
  incidents are still recorded and shown. The target's own incidents still ask for a
  trace of its own.
- **The map draws the copies of a shared hop with no responding address as one node,**
  the owner's, with every copy's role merged into it, like a router node.

## Consequences

- A DNS target that shares hops 2–3 with an ICMP target gets no episode trace of its
  own for an incident at those hops. The ICMP target's trace covers the same path.
