//! Builds the eBPF program (`crates/netwatch-ebpf`) for the BPF target with the
//! nightly toolchain and `bpf-linker`, so that `cargo build` builds everything
//! (ADR-0008). `AYA_BUILD_SKIP=1` skips it; the daemon then reports the source as
//! disabled.

fn main() -> anyhow::Result<()> {
    let root = concat!(env!("CARGO_MANIFEST_DIR"), "/../netwatch-ebpf");
    println!("cargo::rerun-if-changed={root}/src");
    println!("cargo::rerun-if-changed=../netwatch-ebpf-common/src");
    aya_build::build_ebpf(
        [aya_build::Package {
            name: "netwatch-ebpf",
            root_dir: root,
            ..Default::default()
        }],
        aya_build::Toolchain::Nightly,
    )
}
