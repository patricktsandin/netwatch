//! Enforces the dependency rule of ADR-0012: domain crates stay pure.
//!
//! A domain crate may depend only on other domain crates and on the small allowlist
//! of pure libraries below. Nothing in its normal (non-dev) dependency graph may do I/O
//! or bring in an async runtime. Dev-dependencies such as `proptest` are exempt,
//! because they never ship.
//!
//! Every workspace crate must be classified. A new crate that is neither domain,
//! adapter, nor the composition root fails this test until someone decides where it
//! belongs.

use std::collections::{BTreeSet, HashMap};

use cargo_metadata::{DependencyKind, MetadataCommand, Package, PackageId};

const DOMAIN_CRATES: &[&str] = &[
    "netwatch-shared",
    "netwatch-verified",
    "netwatch-topology",
    "netwatch-probing",
    "netwatch-host-health",
    "netwatch-history",
];

/// External crates a domain crate may name directly. Each one must be pure: no I/O,
/// no clocks, no randomness, no async.
const PURE_EXTERNAL_ALLOWLIST: &[&str] = &["vstd", "uom", "thiserror"];

/// Crates that must never appear anywhere in a domain crate's normal dependency
/// graph, even transitively.
const FORBIDDEN_ANYWHERE: &[&str] = &[
    "tokio",
    "mio",
    "async-std",
    "smol",
    "futures-io",
    "libc",
    "nix",
    "rustix",
    "socket2",
    "rusqlite",
    "netlink-sys",
    "zbus",
    "aya",
    "getrandom",
];

const ADAPTER_PREFIX: &str = "netwatch-adapter-";
/// The rest of the eBPF adapter (ADR-0008): the program that runs in the kernel, and
/// what it shares with its loader, `netwatch-adapter-ebpf`.
const EBPF_CRATES: &[&str] = &["netwatch-ebpf", "netwatch-ebpf-common"];
const COMPOSITION_ROOT: &str = "netwatch";

struct Workspace {
    packages: HashMap<PackageId, Package>,
    /// Normal (non-dev, non-build) dependency edges from the resolved graph.
    normal_deps: HashMap<PackageId, Vec<PackageId>>,
    members: Vec<PackageId>,
}

impl Workspace {
    fn load() -> Self {
        let metadata = MetadataCommand::new()
            .exec()
            .expect("cargo metadata must succeed for the test to mean anything");
        let resolve = metadata
            .resolve
            .expect("cargo metadata resolves dependencies by default");
        let normal_deps = resolve
            .nodes
            .into_iter()
            .map(|node| {
                let deps = node
                    .deps
                    .into_iter()
                    .filter(|dep| {
                        dep.dep_kinds
                            .iter()
                            .any(|k| k.kind == DependencyKind::Normal)
                    })
                    .map(|dep| dep.pkg)
                    .collect();
                (node.id, deps)
            })
            .collect();
        Self {
            members: metadata.workspace_members.clone(),
            packages: metadata
                .packages
                .into_iter()
                .map(|p| (p.id.clone(), p))
                .collect(),
            normal_deps,
        }
    }

    fn name(&self, id: &PackageId) -> &str {
        &self.packages[id].name
    }

    fn domain_members(&self) -> impl Iterator<Item = &PackageId> {
        self.members
            .iter()
            .filter(|id| DOMAIN_CRATES.contains(&self.name(id)))
    }

    /// Names of every crate reachable from `root` through normal dependencies.
    fn transitive_normal_deps(&self, root: &PackageId) -> BTreeSet<String> {
        let mut seen = BTreeSet::new();
        let mut stack = vec![root.clone()];
        while let Some(id) = stack.pop() {
            for dep in self.normal_deps.get(&id).into_iter().flatten() {
                if seen.insert(self.name(dep).to_owned()) {
                    stack.push(dep.clone());
                }
            }
        }
        seen
    }
}

#[test]
fn every_workspace_crate_is_classified() {
    let ws = Workspace::load();
    let unclassified: Vec<&str> = ws
        .members
        .iter()
        .map(|id| ws.name(id))
        .filter(|name| {
            !DOMAIN_CRATES.contains(name)
                && !name.starts_with(ADAPTER_PREFIX)
                && !EBPF_CRATES.contains(name)
                && *name != COMPOSITION_ROOT
        })
        .collect();
    assert!(
        unclassified.is_empty(),
        "classify these crates in dependency_rule.rs: {unclassified:?}"
    );
}

#[test]
fn domain_crates_depend_only_on_domain_crates_and_pure_libraries() {
    let ws = Workspace::load();
    for id in ws.domain_members() {
        let illegal: Vec<&str> = ws.normal_deps[id]
            .iter()
            .map(|dep| ws.name(dep))
            .filter(|dep| !DOMAIN_CRATES.contains(dep) && !PURE_EXTERNAL_ALLOWLIST.contains(dep))
            .collect();
        assert!(
            illegal.is_empty(),
            "{} depends directly on {illegal:?}; domain crates are sans-IO (ADR-0012)",
            ws.name(id)
        );
    }
}

#[test]
fn domain_crates_never_pull_in_io_or_async_transitively() {
    let ws = Workspace::load();
    for id in ws.domain_members() {
        let reachable = ws.transitive_normal_deps(id);
        let forbidden: Vec<&&str> = FORBIDDEN_ANYWHERE
            .iter()
            .filter(|f| reachable.contains(**f))
            .collect();
        assert!(
            forbidden.is_empty(),
            "{} transitively depends on {forbidden:?}; domain crates are sans-IO (ADR-0012)",
            ws.name(id)
        );
    }
}
