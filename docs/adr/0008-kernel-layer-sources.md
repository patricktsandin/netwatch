# 8. Kernel layer sources

Date: 2026-10-31

## Status

Accepted

## Context

The kernel layer answers one question: "is the host itself the problem?" Linux
exposes a lot of relevant state, all of it present on the development host (kernel
7.1, BTF available, PSI enabled, conntrack loaded). Too many rows would bury the
signal, so the UI needs a summary. Per-packet event streams from eBPF would cost more
than the information is worth.

## Decision

Collect the following. The UI shows the kernel as **one row**, whose status turns bad
when any source moves in a bad direction. Clicking the row shows each group.

Unprivileged:
1. **Stack baseline:**
   - Loopback ICMP round-trip time, and loopback UDP echo round-trip time (the UDP echo
     goes through the normal socket path, closer to what apps see).
   - Prober timer lateness.
   - Suspend/resume, detected from the `CLOCK_BOOTTIME` − `CLOCK_MONOTONIC` delta. A
     suspend becomes an event and a gap in the graphs, never fake loss.
2. **Protocol counters** (`/proc/net/snmp`, `snmp6`, `netstat`):
   - IP: InDiscards, InHdrErrors, OutNoRoutes, ReasmFails
   - ICMP: OutRateLimitGlobal, OutRateLimitHost
   - UDP: RcvbufErrors, SndbufErrors, InCsumErrors
   - TCP: RetransSegs, TCPTimeouts, resets, ListenOverflows, BacklogDrop
3. **Per-CPU packet processing:** `/proc/net/softnet_stat` (dropped, time_squeeze,
   flow_limit).
4. **Kernel tables:**
   - Conntrack: the entry count vs. `nf_conntrack_max`, plus its drop, insert_failed,
     and early_drop stats.
   - Neighbor caches (`arp_cache`, `ndisc_cache`): table-full events and packets
     discarded while an address was unresolved.
   - Socket memory vs. its limits (`sockstat{,6}`).
5. **Egress queues:** qdisc statistics (`TCA_STATS2`): backlog, drops, overlimits,
   requeues, and ECN marks.
6. **Host contention:** PSI for cpu, memory, io, and irq; NET_RX and NET_TX softirq
   counts; how NIC interrupts are distributed across CPUs; CPU steal time.
7. **Clock health:** sync state and estimated error from `adjtimex`, changes of clock
   source, and wall-clock jumps.
8. **Config drift:** poll the network-relevant sysctls, because inotify doesn't work on
   `/proc/sys`, and record each change as an event.

Privileged (optional, per ADR-0007):

9. **Kernel log** (`CAP_SYSLOG`): network-relevant lines from `/dev/kmsg` (NETDEV
   WATCHDOG, link up/down, driver resets, firmware errors), matched against a
   maintained pattern list and attributed to an interface where possible, recorded as
   events.
10. **Drop reasons** (`CAP_BPF` + `CAP_PERFMON`):
    - An `aya` program on the `skb:kfree_skb` tracepoint counts drops by
      `skb_drop_reason`, and by interface when the skb has a device.
    - Counting happens in **per-CPU maps inside the kernel**, and userspace reads them
      every 1s. Never stream per-packet events.
    - The program is built with CO-RE against BTF, so one binary works across kernel
      versions. Reason names come from the running kernel's BTF.

Out of scope: nftables ruleset changes, because they would need `CAP_NET_ADMIN`.

## Consequences

- Most causes that originate on the host can be identified from netwatch alone.
- eBPF adds a nightly toolchain, `bpf-linker`, and two crates (the programs and the types
  they share with their loader). A plain `cargo build` still builds everything, through
  `aya-build`.
- The kernel-log pattern list needs maintenance as drivers change their messages.
