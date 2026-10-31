//! Trace conformance: the Rust aggregate must behave exactly like the "Aggregate"
//! section of `specs/topology.qnt` (ADR-0016).
//!
//! The test asks Quint to generate random traces of the whole spec (world, aggregate,
//! and probes), and then replays only the aggregate's actions against [`Topology`]:
//! - `deliver` becomes the observation at the head of the previous state's `pending`
//! - `reportLoss` becomes `NotificationsLost`
//! - `deliverSnapshot` becomes the snapshot at the head of the previous state's
//!   `snapshots`
//! - `probeIdentify` becomes `ProbeIdentified` for the spec's `PROBE_ROLE`, with the
//!   identity the spec picked
//!
//! Every other action must leave the aggregate untouched. After every step, the test
//! checks the outputs, every role's binding, and whether the view is synchronized.
//!
//! The spec's roles are the integers `0..NUM_ROLES`, and they map, in ascending order,
//! onto `SPEC_ROLES`. That preserves the order the resync visits roles in, which
//! epoch allocation depends on.
//!
//! The test needs `quint` on `PATH`, or set `QUINT` to its path. A missing `quint`
//! fails the test rather than skipping it, because skipping would quietly weaken
//! the check (CLAUDE.md, "Workflow").

use std::collections::BTreeMap;
use std::num::{NonZeroU32, NonZeroU64};
use std::path::{Path, PathBuf};
use std::process::Command as Process;

use netwatch_shared::{Identity, IdentityEpoch, InterfaceIndex, InterfaceName, Role};
use netwatch_topology::{
    Binding, Command, Observation, Output, ResyncGeneration, Snapshot, Topology, TopologyEvent,
};
use serde::Deserialize;

const SPEC: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/../../specs/topology.qnt");
const TRACES: usize = 200;
const MAX_STEPS: usize = 60;
const SEED: &str = "0x6e657477617463";

/// Spec role `i` is `SPEC_ROLES[i]`. Must be ascending in `Role`'s order: the
/// spec's `NUM_ROLES` global roles, then its `PROBE_ROLE`.
const SPEC_ROLES: [Role; 3] = [
    Role::Nic,
    Role::Tunnel,
    Role::Hop {
        target: netwatch_shared::TargetId::new(1),
        ttl: netwatch_shared::HopTtl::new(std::num::NonZeroU8::MIN),
    },
];
const PROBE_ROLE: i64 = 2;

// --- The spec's state, as ITF encodes it ----------------------------------------

#[derive(Debug, Deserialize)]
struct SpecState {
    agg: SpecAggregate,
    outputs: Vec<SpecOutput>,
    pending: Vec<SpecObservation>,
    snapshots: Vec<SpecSnapshot>,
    #[serde(rename = "mbt::actionTaken")]
    action_taken: String,
    #[serde(rename = "mbt::nondetPicks")]
    picks: Picks,
}

#[derive(Debug, Deserialize)]
struct Picks {
    identity: SpecOption<i64>,
}

#[derive(Debug, Deserialize)]
#[serde(tag = "tag", content = "value")]
enum SpecOption<T> {
    Some(T),
    None(serde::de::IgnoredAny),
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct SpecAggregate {
    bindings: BTreeMap<i64, SpecBinding>,
    synchronized: bool,
}

#[derive(Debug, Deserialize)]
struct SpecBinding {
    identity: i64,
    epoch: i64,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(tag = "tag", content = "value")]
enum SpecObservation {
    IdentityObserved { role: i64, identity: i64 },
    RoleVanished(i64),
}

#[derive(Debug, Clone, Deserialize)]
struct SpecSnapshot {
    generation: i64,
    state: BTreeMap<i64, i64>,
}

#[derive(Debug, Deserialize)]
#[serde(tag = "tag", content = "value")]
enum SpecOutput {
    EpochStarted {
        epoch: i64,
        role: i64,
        identity: i64,
    },
    EpochEnded {
        epoch: i64,
        role: i64,
    },
    Desynchronized(i64),
    Resynchronized(i64),
    Resync(i64),
}

// --- Mapping spec values onto domain values -------------------------------------

fn role(spec: i64) -> Role {
    let index = usize::try_from(spec).expect("spec roles are non-negative");
    SPEC_ROLES[index].clone()
}

fn identity(spec: i64) -> Identity {
    let raw = u32::try_from(spec + 1).expect("spec identities are small and non-negative");
    Identity::Interface {
        index: InterfaceIndex::new(NonZeroU32::new(raw).expect("spec identity + 1 is positive")),
        name: InterfaceName::parse(format!("if{spec}").as_bytes()).expect("ifN is a valid name"),
    }
}

fn epoch(spec: i64) -> IdentityEpoch {
    let raw = u64::try_from(spec).expect("spec epochs are positive");
    IdentityEpoch::new(NonZeroU64::new(raw).expect("spec epochs start at 1"))
}

fn generation(spec: i64) -> ResyncGeneration {
    ResyncGeneration::new(u64::try_from(spec).expect("spec generations are non-negative"))
}

fn observation(spec: &SpecObservation) -> Observation {
    match *spec {
        SpecObservation::IdentityObserved {
            role: r,
            identity: i,
        } => Observation::IdentityObserved {
            role: role(r),
            identity: identity(i),
        },
        SpecObservation::RoleVanished(r) => Observation::RoleVanished { role: role(r) },
    }
}

fn snapshot(spec: &SpecSnapshot) -> Observation {
    Observation::SnapshotReceived(Snapshot {
        generation: generation(spec.generation),
        bindings: spec
            .state
            .iter()
            .map(|(&r, &i)| (role(r), identity(i)))
            .collect(),
    })
}

fn output(spec: &SpecOutput) -> Output {
    match *spec {
        SpecOutput::EpochStarted {
            epoch: e,
            role: r,
            identity: i,
        } => Output::Event(TopologyEvent::EpochStarted {
            epoch: epoch(e),
            role: role(r),
            identity: identity(i),
        }),
        SpecOutput::EpochEnded { epoch: e, role: r } => Output::Event(TopologyEvent::EpochEnded {
            epoch: epoch(e),
            role: role(r),
        }),
        SpecOutput::Desynchronized(g) => Output::Event(TopologyEvent::Desynchronized {
            generation: generation(g),
        }),
        SpecOutput::Resynchronized(g) => Output::Event(TopologyEvent::Resynchronized {
            generation: generation(g),
        }),
        SpecOutput::Resync(g) => Output::Command(Command::Resync {
            generation: generation(g),
        }),
    }
}

// --- Trace generation and replay ------------------------------------------------

fn generate_traces(dir: &Path) -> Vec<PathBuf> {
    std::fs::create_dir_all(dir).expect("can create the trace directory");
    for stale in std::fs::read_dir(dir).expect("can list the trace directory") {
        std::fs::remove_file(stale.expect("can read a directory entry").path())
            .expect("can remove a stale trace");
    }
    let quint = std::env::var("QUINT").unwrap_or_else(|_| "quint".to_owned());
    let status = Process::new(&quint)
        .args(["run", SPEC, "--mbt"])
        .arg(format!("--n-traces={TRACES}"))
        .arg(format!("--max-samples={TRACES}"))
        .arg(format!("--max-steps={MAX_STEPS}"))
        .arg(format!("--seed={SEED}"))
        .arg(format!(
            "--out-itf={}/trace_{{seq}}.itf.json",
            dir.display()
        ))
        .status()
        .unwrap_or_else(|e| panic!("could not run `{quint}` (install Quint or set QUINT): {e}"));
    assert!(status.success(), "quint run failed: {status}");
    let mut traces: Vec<PathBuf> = std::fs::read_dir(dir)
        .expect("can list the trace directory")
        .map(|entry| entry.expect("can read a directory entry").path())
        .collect();
    traces.sort();
    traces
}

fn replay(path: &Path) {
    let json = std::fs::read_to_string(path).expect("can read the trace");
    let trace: itf::Trace<SpecState> = itf::trace_from_str(&json).expect("trace matches SpecState");
    let states: Vec<&SpecState> = trace.states.iter().map(|s| &s.value).collect();
    let mut topology = Topology::new();

    for (step, pair) in states.windows(2).enumerate() {
        let (before, after) = (pair[0], pair[1]);
        let context = || {
            format!(
                "{}, step {}: {}",
                path.display(),
                step + 1,
                after.action_taken
            )
        };

        let observed = match after.action_taken.as_str() {
            "deliver" => Some(observation(&before.pending[0])),
            "reportLoss" => Some(Observation::NotificationsLost),
            "deliverSnapshot" => Some(snapshot(&before.snapshots[0])),
            "probeIdentify" => {
                let SpecOption::Some(picked) = after.picks.identity else {
                    panic!("probeIdentify without a picked identity");
                };
                Some(Observation::ProbeIdentified {
                    role: role(PROBE_ROLE),
                    identity: identity(picked),
                })
            }
            _ => None,
        };
        let outputs = observed.map_or_else(Vec::new, |o| topology.observe(o));

        let expected: Vec<Output> = after.outputs.iter().map(output).collect();
        assert_eq!(outputs, expected, "outputs differ at {}", context());
        assert_eq!(
            topology.is_synchronized(),
            after.agg.synchronized,
            "synchronization differs at {}",
            context()
        );
        for (spec_role, domain_role) in SPEC_ROLES.iter().enumerate() {
            let spec_role = i64::try_from(spec_role).expect("few roles");
            let expected = after.agg.bindings.get(&spec_role).map(|b| Binding {
                identity: identity(b.identity),
                epoch: epoch(b.epoch),
            });
            assert_eq!(
                topology.binding(domain_role),
                expected.as_ref(),
                "binding of {domain_role:?} differs at {}",
                context()
            );
        }
    }
}

#[test]
fn topology_conforms_to_the_quint_spec() {
    let dir = PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join("topology-traces");
    let traces = generate_traces(&dir);
    assert!(
        traces.len() >= TRACES / 2,
        "expected about {TRACES} traces, quint produced {}",
        traces.len()
    );
    for trace in &traces {
        replay(trace);
    }
}
