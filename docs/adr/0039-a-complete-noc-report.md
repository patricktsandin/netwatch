# 39. A complete NOC report, and the facts it needs

Date: 2026-11-01

## Status

Accepted. Extends ADR-0033.

## Context

A NOC triages a ticket faster when it has nothing left to ask for. ADR-0033's report
gave episodes with path evidence. A NOC also asks:
- who the customer is on the network (public address, where traffic enters)
- what the local link is (speed, duplex, flaps), and whether the clock can be trusted
- how the outage looked over time
- why probes failed
- what "normal" was
- whether it recurs at certain hours
- what else changed

It also wants the raw data.

## Decision

**Two new facts, which the main UI shows too:**
- **Public identity.** Every 5 minutes, through the egress interface, the daemon reads
  `https://one.one.one.one/cdn-cgi/trace` and parses `ip=`, `colo=`, and `loc=`. The
  parser is pure and Kani-harnessed. It is shown in the UI header. A change is an
  event on the local interface's row, so the report can list every public address
  the span used. IPv4 only, like the HTTPS probes.
- **Link facts.** Each second, from sysfs: the NIC's speed, duplex, driver, MTU, and
  carrier-change count. The facts are notes on the NIC row. `Link.CarrierChanges`
  joins the NIC's counters, and grows on every link flap, which fails the NIC
  (`NIC_RULES`).

**The report gains:**
- **About:** hostname, kernel, netwatch version, report span in UTC (with the host's
  offset), clock synchronization and estimated error, and how measurements are
  taken (probes, cadences, timeouts, and what "outage" and "degradation" mean).
- **Connection:** NIC and link facts, local address, the IPv4 and IPv6 gateways, the
  ISP-side routers at hops 2–3, the public address and entry point, the system
  resolver, and the radios' state.
- **Summary:** episode counts by place, total and longest outage beyond the gateway,
  and each target's availability.
- **By hour** (by day for spans over 7 days): probes, loss, and median and p95 RTT
  across all targets, so time-of-day patterns show.
- **Per episode:**
  - why probes failed, by role: timeouts, ICMP unreachable (with the code), refused,
    and so on
  - a minute-by-minute timeline (10-minute steps for long episodes) of the cause's
    and the affected targets' loss
  - each path role's median RTT in the hour before, next to the median during
- **Events:** every event in the span, in UTC.
- **Raw samples:** `/report.csv` has every sample from 2 minutes before to 2 minutes
  after each episode, with role, identity, outcome, and ICMP code.

All of it is pure functions over stored history (`netwatch_adapter_web::report`), with
tests.

## Consequences

- One report answers the NOC's first round of questions.
- Loss reasons and the CSV need raw samples, so for episodes older than 7 days the
  report says they have expired.
- The public address is sent to one.one.one.one every 5 minutes. That is the same
  service the default HTTPS target already probes.
