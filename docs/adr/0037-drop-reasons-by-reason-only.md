# 37. Drop reasons are counted by reason only

Date: 2026-11-01

## Status

Accepted. Amends ADR-0008, item 10.

## Context

ADR-0008 planned to count drops "by `skb_drop_reason`, and by interface when the skb
has a device", built with CO-RE so one binary works on every kernel. Reading the
interface means following `skb->dev->ifindex`, whose offsets differ between kernels.
Portably, that needs CO-RE field relocations, which Rust eBPF (`aya-ebpf`) doesn't
emit reliably yet.

## Decision

- The program attaches to `tp_btf/kfree_skb`, whose typed arguments `(skb, location,
  reason, rx_sk)` are the same on every kernel with drop reasons. It counts by reason
  alone, in a 256-slot per-CPU array, and puts subsystem-specific reasons (high bits
  set) in the last slot, `Drop.OTHER`.
- Reason names come from the running kernel's BTF, through a small parser of the one
  enum, with a Kani harness.
- The counts join the kernel layer's counters as `Drop.<REASON>`. Only the reasons
  that mean the host ran short grade it.
- Counting by interface waits until CO-RE field access works from Rust.

## Consequences

- The kernel layer says *why* packets were dropped, but not on which interface. The
  NIC's own counters still cover drops in the device and driver.
- The program needs no per-kernel offsets.
