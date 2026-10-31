# 2. Monitored layers are logical roles with changing identities

Date: 2026-10-31

## Status

Accepted

## Context

Like mtr, netwatch should show where along the path a problem originates, but it
should start lower: inside the host. It must also keep following the path as the
network changes. The NIC, the addresses, and the hops all change, through failover,
DHCP renewal, VPNs, and upstream reroutes.

The development host is a good example. `eth0` carries the default route (metric
100). `wlp0s20f3` is a Wi-Fi backup default route (metric 600), and a `wwan0` modem is
also present. Failover between them is a normal, expected event.

If series were keyed by address or interface name, every change would split the
history into unrelated fragments. If series ignored identity entirely, measurements of
different things would be mixed together.

Some layer definitions needed care:
- On Linux, pinging the host's own IP goes through `lo` and never touches the NIC, so
  it doesn't measure the local interface.
- Intermediate routers deprioritize ICMP, so pinging the gateway's IP mixes link quality
  with how busy the gateway's CPU is.
- Behind a VPN or policy routing, different targets take different paths, so one
  shared set of "first three hops" is wrong for some of them.

## Decision

Each layer is a **role**. Series are keyed by role. The concrete **identity** behind
the role (interface, IP, MAC, responding address, access point, cell, anycast point of
presence) is an attribute of each sample, and it can change over time.

| Layer | Measures |
|---|---|
| kernel | Host network stack health (ADR-0008) |
| nic | The physical device carrying the egress route right now: link state, carrier changes, speed/duplex, driver/hardware counters (ethtool netlink, `IFLA_STATS64`). On Wi-Fi: signal, noise, bitrate, retries, BSSID (nl80211). On cellular: signal quality, access technology, serving cell (ModemManager) |
| tunnel | When the egress route goes through a VPN interface: the same counters as the NIC, plus WireGuard handshake age. Linked to the physical NIC underneath, found by a route lookup to the tunnel endpoint |
| local interface | Addresses, DHCP/SLAAC lifetimes, the gateway's neighbor state, interface counters, and active link-layer probes of the gateway: ARP (v4) and neighbor solicitation (v6) |
| hops 1–3 | Per target, from that target's own route lookup, using TTL-limited probes. Targets whose hops resolve identically share one probe stream |
| targets | ICMP, DNS (per-server query time, rcode), and HTTP(S) (phase timings for DNS, connect, TLS, TTFB, and total) |

Changes of access point, serving cell, or anycast point of presence are topology events.

## Consequences

- History stays continuous across changes, while samples stay honest about what
  they measured. This needs the identity-epoch machinery from ADR-0003.
- The tunnel layer shows whether a problem is inside the tunnel or on the link beneath it.
- Link-layer probing of the gateway measures the link without the gateway's IP stack or
  its ICMP rate limiting, but it requires `CAP_NET_RAW` (ADR-0007).
- Hops per target cost more probes. Deduplicating identical paths keeps the common case
  (no VPN, one default route) as cheap as a single shared set of hops.
- Wi-Fi and cellular statistics add the nl80211 and ModemManager adapters.
