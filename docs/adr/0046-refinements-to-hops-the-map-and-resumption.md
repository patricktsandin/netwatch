# 46. Refinements to hop sharing, the map, and resumed epochs

Date: 2026-11-02

## Status

Accepted. Refines ADR-0042, ADR-0043, and ADR-0044, after a code review.

## Context

A review of the network map, DNS hops, and resumed epochs found three cases where
the behaviour those ADRs describe is wrong:
- **Hop 1's stream goes toward its owner's address**, and the owner was the lowest
  target. Since ADR-0043, that can be a DNS target on the LAN, such as a Pi-hole.
  A TTL-1 probe to it is answered by the resolver itself, so every target's hop 1
  would show the resolver instead of the gateway.
- **The map only recognized a target in its trace**, at TTL 4 and beyond. A
  resolver one hop away, often the gateway itself (`dns = "system"`), answers its
  own hop probes. It was drawn as three routers with the target four steps away.
- **A role's first epoch after a start resumed the identity from before the stop,
  however late that epoch started.** A daemon started during an outage, whose NIC
  comes back an hour later the same, would hide that return.

## Decision

- **Hop 1's owner is the lowest target with a public address** (not private, shared,
  loopback, link-local, or documentation space), or the lowest target if none has
  one. The scope test moves to the shared crate (`netwatch_shared::address`), since
  Probing and the report both use it.
- **A path reaches its target where the target's own address answers**, whether at a
  hop or in the trace. The target's node is placed there. Its hop roles from that TTL
  on merge into the target's node: they measure the target itself.
- **Only an epoch that starts within 60 s of the daemon's start can resume one.**
  Binding every role takes seconds. An identity that comes back later is a change
  and is marked.

## Consequences

- A target on the LAN still shares hop 1's stream. Its hop 1 shows the gateway, even
  though traffic to an on-link host doesn't pass the gateway. Telling on-link
  targets apart needs a route lookup per target (ADR-0022's limitation), which is
  left for later.
- A target whose hops all merge into it shows the worst of their statuses on the map,
  like any merged node.
