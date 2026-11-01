//! Counts freed packets by drop reason, in a per-CPU array inside the kernel
//! (ADR-0008). Userspace reads and sums the counts once a second; nothing is ever
//! streamed per packet.
//!
//! `tp_btf/kfree_skb` passes the tracepoint's arguments typed and in a fixed order,
//! `(skb, location, reason, rx_sk)`, on every kernel that has drop reasons, so the
//! program needs no structure offsets.
#![no_std]
#![no_main]

use aya_ebpf::macros::{btf_tracepoint, map};
use aya_ebpf::maps::PerCpuArray;
use aya_ebpf::programs::BtfTracePointContext;
use netwatch_ebpf_common::REASONS;

#[map(name = "DROPS")]
static DROPS: PerCpuArray<u64> = PerCpuArray::with_max_entries(REASONS, 0);

#[btf_tracepoint(function = "kfree_skb")]
pub fn kfree_skb(ctx: BtfTracePointContext) -> i32 {
    let reason: u64 = ctx.arg(2);
    let slot = if reason < u64::from(REASONS) {
        reason as u32
    } else {
        REASONS - 1
    };
    if let Some(count) = DROPS.get_ptr_mut(slot) {
        // SAFETY: the map is per-CPU, so no other program touches this slot while
        // this one runs, and the pointer comes from the map itself.
        #[allow(unsafe_code)]
        unsafe {
            *count += 1
        };
    }
    0
}

#[cfg(not(test))]
#[panic_handler]
fn panic(_info: &core::panic::PanicInfo) -> ! {
    loop {}
}
