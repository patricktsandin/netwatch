//! What the drop-reason eBPF program (`netwatch-ebpf`) and its loader
//! (`netwatch-adapter-ebpf`) agree on (ADR-0008). `no_std`, because the program is
//! built for the BPF target.
#![no_std]

/// The program, attached as `tp_btf/kfree_skb`.
pub const PROGRAM: &str = "kfree_skb";

/// The per-CPU array of drop counts, indexed by `skb_drop_reason`.
pub const DROPS: &str = "DROPS";

/// Slots in [`DROPS`]. A reason at or beyond the last slot (subsystem-specific
/// reasons carry high bits) is counted in the last one.
pub const REASONS: u32 = 256;
