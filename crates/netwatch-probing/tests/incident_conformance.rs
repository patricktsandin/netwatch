//! Trace conformance: `IncidentTracker` must behave exactly like
//! `specs/incidents.qnt` (ADR-0016, ADR-0031).
//!
//! Quint generates traces, which this test replays against the tracker. Spec role 0
//! is hop 1 toward target 1, and role 1 is target 1, so role 0 is upstream of role 1
//! in both. One tick is one second.
//! - `advance` does nothing: time only labels the outcomes and gradings after it
//! - `outcome` becomes `outcome(role, now, answered)`
//! - `grade` becomes `grade(now, grades)`, and must return the spec's `updates`
//!
//! Needs `quint` on `PATH`, or set `QUINT`.

#![allow(clippy::unwrap_used)] // Tests may unwrap (ADR-0015), including in helpers.

use std::collections::BTreeMap;
use std::num::NonZeroU8;
use std::path::{Path, PathBuf};
use std::process::Command;

use netwatch_probing::Health;
use netwatch_probing::incident::{Incident, IncidentTracker, Severity};
use netwatch_shared::uom::si::f64::Time;
use netwatch_shared::uom::si::time::second;
use netwatch_shared::{HopTtl, Role, TargetId, WallTime};
use serde::Deserialize;
use serde::de::IgnoredAny;

const SPEC: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/../../specs/incidents.qnt");
const TRACES: usize = 200;
const NONE: i64 = -1;

#[derive(Debug, Deserialize)]
struct SpecState {
    now: i64,
    updates: Vec<SpecIncident>,
    #[serde(rename = "mbt::actionTaken")]
    action: String,
    #[serde(rename = "mbt::nondetPicks")]
    picks: Picks,
}

#[derive(Debug, Deserialize)]
struct Picks {
    r: SpecOption<i64>,
    answered: SpecOption<bool>,
    h0: SpecOption<String>,
    h1: SpecOption<String>,
}

#[derive(Debug, Deserialize)]
#[serde(tag = "tag", content = "value")]
enum SpecOption<T> {
    Some(T),
    None(IgnoredAny),
}

impl<T: Clone> SpecOption<T> {
    fn get(&self) -> T {
        match self {
            SpecOption::Some(value) => value.clone(),
            SpecOption::None(_) => panic!("the spec didn't pick this value"),
        }
    }
}

#[derive(Debug, Deserialize)]
struct SpecIncident {
    role: i64,
    started: i64,
    ended: i64,
    severity: String,
    cause: i64,
}

fn role(spec: i64) -> Role {
    let target = TargetId::new(1);
    match spec {
        0 => Role::Hop {
            target,
            ttl: HopTtl::new(NonZeroU8::MIN),
        },
        1 => Role::Target(target),
        other => panic!("the spec has no role {other}"),
    }
}

fn at(ticks: i64) -> WallTime {
    WallTime::from_unix_micros(ticks * 1_000_000)
}

fn health(spec: &str) -> Health {
    match spec {
        "healthy" => Health::Healthy,
        "degraded" => Health::Degraded,
        "failing" => Health::Failing,
        other => panic!("the spec has no health {other:?}"),
    }
}

fn incident(spec: &SpecIncident) -> Incident {
    Incident {
        role: role(spec.role),
        started: at(spec.started),
        ended: (spec.ended != NONE).then(|| at(spec.ended)),
        severity: match spec.severity.as_str() {
            "outage" => Severity::Outage,
            "degradation" => Severity::Degradation,
            other => panic!("the spec has no severity {other:?}"),
        },
        cause: (spec.cause != NONE).then(|| role(spec.cause)),
    }
}

fn generate(dir: &Path) -> Vec<PathBuf> {
    std::fs::create_dir_all(dir).unwrap();
    for stale in std::fs::read_dir(dir).unwrap() {
        std::fs::remove_file(stale.unwrap().path()).unwrap();
    }
    let quint = std::env::var("QUINT").unwrap_or_else(|_| "quint".to_owned());
    let status = Command::new(&quint)
        .args(["run", SPEC, "--mbt", "--max-steps=40", "--seed=0x696e6364"])
        .arg(format!("--n-traces={TRACES}"))
        .arg(format!("--max-samples={TRACES}"))
        .arg(format!(
            "--out-itf={}/trace_{{seq}}.itf.json",
            dir.display()
        ))
        .status()
        .unwrap_or_else(|e| panic!("could not run `{quint}` (install Quint or set QUINT): {e}"));
    assert!(status.success());
    let mut traces: Vec<PathBuf> = std::fs::read_dir(dir)
        .unwrap()
        .map(|e| e.unwrap().path())
        .collect();
    traces.sort();
    traces
}

fn replay(path: &Path) -> usize {
    let trace: itf::Trace<SpecState> =
        itf::trace_from_str(&std::fs::read_to_string(path).unwrap()).unwrap();
    // The spec's grace is one tick, which is one second here.
    let mut tracker = IncidentTracker::new(Time::new::<second>(1.0));
    let mut updates_seen = 0;
    for (step, state) in trace.states.iter().map(|s| &s.value).enumerate().skip(1) {
        match state.action.as_str() {
            "advance" => {}
            "outcome" => {
                tracker.outcome(
                    &role(state.picks.r.get()),
                    at(state.now),
                    state.picks.answered.get(),
                );
            }
            "grade" => {
                let grades = BTreeMap::from([
                    (role(0), health(&state.picks.h0.get())),
                    (role(1), health(&state.picks.h1.get())),
                ]);
                let expected: Vec<Incident> = state.updates.iter().map(incident).collect();
                updates_seen += expected.len();
                assert_eq!(
                    tracker.grade(at(state.now), &grades),
                    expected,
                    "{}, step {step}",
                    path.display()
                );
            }
            other => panic!("unknown action {other}"),
        }
    }
    updates_seen
}

#[test]
fn incident_tracker_conforms_to_the_quint_spec() {
    let dir = PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join("incident-traces");
    let traces = generate(&dir);
    assert!(
        traces.len() >= TRACES / 2,
        "quint produced {} traces",
        traces.len()
    );
    let updates: usize = traces.iter().map(|t| replay(t)).sum();
    // The traces must actually exercise the tracker.
    assert!(
        updates > TRACES,
        "only {updates} updates in {} traces",
        traces.len()
    );
}
