//! Drop reasons (ADR-0008): loads the `kfree_skb` counter into the kernel, and reads
//! its per-CPU counts as counters named by this kernel's `skb_drop_reason` enum.
//!
//! Loading needs `CAP_BPF` and `CAP_PERFMON`. Without them, [`DropReasons::start`]
//! fails and the daemon reports the source as disabled (ADR-0007).

mod btf;

use std::collections::BTreeMap;

use aya::maps::{MapData, PerCpuArray};
use aya::programs::BtfTracePoint;
use netwatch_ebpf_common::{DROPS, PROGRAM, REASONS};

pub use btf::enum_variants;

/// Why the drop-reason source couldn't start.
#[derive(Debug, thiserror::Error)]
pub enum StartError {
    /// The program wasn't built into this binary (`AYA_BUILD_SKIP`).
    #[error("this build has no eBPF program")]
    NotBuilt,
    /// The kernel has no BTF, or no `skb_drop_reason`.
    #[error("the kernel doesn't name its drop reasons (no BTF, or a kernel before 5.17)")]
    NoReasons,
    /// Loading or attaching failed, usually for want of `CAP_BPF` and `CAP_PERFMON`.
    #[error(transparent)]
    Load(#[from] aya::EbpfError),
    /// The program was rejected.
    #[error(transparent)]
    Program(#[from] aya::programs::ProgramError),
    /// The map wasn't usable.
    #[error(transparent)]
    Map(#[from] aya::maps::MapError),
}

impl StartError {
    /// Whether the cause is a missing privilege, so the message can name it.
    pub fn is_permission(&self) -> bool {
        let mut cause: Option<&(dyn std::error::Error + 'static)> = Some(self);
        while let Some(error) = cause {
            if let Some(io) = error.downcast_ref::<std::io::Error>()
                && (io.kind() == std::io::ErrorKind::PermissionDenied
                    || io.raw_os_error() == Some(1))
            {
                return true;
            }
            cause = error.source();
        }
        false
    }
}

const PROGRAM_BYTES: &[u8] =
    aya::include_bytes_aligned!(concat!(env!("OUT_DIR"), "/netwatch-ebpf"));

/// The loaded program and its counts. Dropping it detaches the program.
pub struct DropReasons {
    _ebpf: aya::Ebpf,
    drops: PerCpuArray<MapData, u64>,
    /// Counter names by slot: `Drop.<REASON>`.
    names: BTreeMap<u32, String>,
}

impl DropReasons {
    /// Loads and attaches the program.
    pub fn start() -> Result<Self, StartError> {
        if PROGRAM_BYTES.is_empty() {
            return Err(StartError::NotBuilt);
        }
        let vmlinux =
            std::fs::read("/sys/kernel/btf/vmlinux").map_err(|_| StartError::NoReasons)?;
        let reasons = enum_variants(&vmlinux, "skb_drop_reason").ok_or(StartError::NoReasons)?;
        let names = reasons
            .into_iter()
            .filter(|&(_, value)| value < REASONS - 1)
            .map(|(name, value)| {
                let short = name.strip_prefix("SKB_DROP_REASON_").unwrap_or(&name);
                (value, format!("Drop.{short}"))
            })
            .chain([(REASONS - 1, "Drop.OTHER".to_owned())])
            .collect();
        let mut ebpf = aya::Ebpf::load(PROGRAM_BYTES)?;
        let btf = aya::Btf::from_sys_fs().map_err(|_| StartError::NoReasons)?;
        let program: &mut BtfTracePoint = ebpf
            .program_mut(PROGRAM)
            .ok_or(StartError::NotBuilt)?
            .try_into()?;
        program.load(PROGRAM, &btf)?;
        program.attach()?;
        let map = ebpf.take_map(DROPS).ok_or(StartError::NotBuilt)?;
        Ok(Self {
            drops: PerCpuArray::try_from(map)?,
            _ebpf: ebpf,
            names,
        })
    }

    /// Every named reason's total count since the program was attached, summed over
    /// CPUs, as counters for the kernel layer's `CounterWatch`.
    pub fn read(&self) -> BTreeMap<String, u64> {
        self.names
            .iter()
            .filter_map(|(&slot, name)| {
                let per_cpu = self.drops.get(&slot, 0).ok()?;
                Some((
                    name.clone(),
                    per_cpu.iter().copied().fold(0u64, u64::wrapping_add),
                ))
            })
            .collect()
    }
}
