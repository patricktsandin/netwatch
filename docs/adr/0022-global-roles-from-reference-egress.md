# 22. Global roles follow the egress path to a reference destination

Date: 2026-10-31

## Status

Accepted

## Context

The NIC, tunnel, and local interface roles (ADR-0002) are global: there is one of each,
not one per target. Something has to define which interface "the NIC" is. The host can
have several default routes (here, `eth0` with metric 100 and Wi-Fi with metric 600),
policy routing, and VPNs. The kernel is the only authority on which route actually wins.

The netlink adapter also has to decide what to do with the multicast notifications it
receives. Parsing each one and applying it as a delta would mean re-implementing the
kernel's route selection, and trusting the order in which messages arrive.

## Decision

- The global roles come from `RTM_GETROUTE` for a **reference destination**, 1.1.1.1.
  The kernel's answer gives the egress interface and the preferred source address:
  - a physical egress interface binds `Nic`
  - a tunnel interface (WireGuard or tun/tap, by link kind) binds `Tunnel`
  - the source address binds `LocalInterface`
- **A notification is only a trigger.** On any link, address, or route notification, the
  adapter resolves the path again and sends every global role's complete state. The
  content of a notification is never interpreted.
- The adapter **subscribes before its first resolution**, so no change can fall between
  the two.
- The mapping from egress path to roles is a domain rule, so it lives in
  `netwatch_topology::egress` (pure, and tested there), not in the adapter.

## Consequences

- Failover, link down/up, and readdressing are followed correctly. An `#[ignore]`d
  netns test checks this end to end, by failing over from `a0` to `b0` and back.
- Every notification costs two netlink round trips (`RTM_GETROUTE` + `RTM_GETLINK`). That
  is negligible at the rate topology changes.
- **Known limitations, to be addressed by later ADRs:**
  - The reference destination is IPv4 only. A host whose IPv6 egress differs from its
    IPv4 egress will have IPv6 targets measured over a NIC that isn't the global one.
    Per-target egress paths for hops will make this visible.
  - Behind a tunnel, `Nic` is left unbound. Finding the underlay needs a lookup toward
    the tunnel endpoint (WireGuard's peer endpoint comes over genetlink).
  - Wi-Fi and cellular identities (BSSID, serving cell) aren't observed yet.
