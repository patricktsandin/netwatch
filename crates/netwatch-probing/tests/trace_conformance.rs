//! Trace conformance: `TraceSchedule` must behave exactly like `specs/traces.qnt`
//! (ADR-0016, ADR-0040).
//!
//! Quint generates traces, which this test replays against the schedule. Spec
//! target 0 is target 1 and spec target 1 is target 2. Spec role `near` is hop 1,
//! `hop` is hop 2, and `target` is the target. One tick is one second.
//! - `advance` does nothing: time only labels the steps after it
//! - `incident` becomes `incident(update, now)`
//! - `tick` becomes `tick(now)`
//! - `finish` becomes `finished(target, now)`
//!
//! Every step must return the spec's `started`. Needs `quint` on `PATH`, or set
//! `QUINT`.

#![allow(clippy::unwrap_used)] // Tests may unwrap (ADR-0015), including in helpers.

use std::num::NonZeroU8;
use std::path::{Path, PathBuf};
use std::process::Command;

use netwatch_probing::incident::{Incident, Severity};
use netwatch_probing::trace::{TraceKind, TraceRequest, TraceSchedule};
use netwatch_shared::uom::si::f64::Time;
use netwatch_shared::uom::si::time::second;
use netwatch_shared::{HopTtl, Instant, Role, TargetId, WallTime};
use serde::Deserialize;
use serde::de::IgnoredAny;

const SPEC: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/../../specs/traces.qnt");
const TRACES: usize = 200;

#[derive(Debug, Deserialize)]
struct SpecState {
    now: i64,
    started: Vec<SpecStart>,
    #[serde(rename = "mbt::actionTaken")]
    action: String,
    #[serde(rename = "mbt::nondetPicks")]
    picks: Picks,
}

#[derive(Debug, Deserialize)]
struct SpecStart {
    target: i64,
    kind: String,
}

#[derive(Debug, Deserialize)]
struct Picks {
    t: SpecOption<i64>,
    r: SpecOption<String>,
    #[serde(rename = "isOpen")]
    is_open: SpecOption<bool>,
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

fn target(spec: i64) -> TargetId {
    TargetId::new(u32::try_from(spec + 1).unwrap())
}

fn role(t: TargetId, spec: &str) -> Role {
    let hop = |n| Role::Hop {
        target: t,
        ttl: HopTtl::new(NonZeroU8::new(n).unwrap()),
    };
    match spec {
        "near" => hop(1),
        "hop" => hop(2),
        "target" => Role::Target(t),
        other => panic!("the spec has no role {other:?}"),
    }
}

fn at(ticks: i64) -> Instant {
    Instant::from_nanos(u64::try_from(ticks).unwrap() * 1_000_000_000)
}

fn request(spec: &SpecStart) -> TraceRequest {
    TraceRequest {
        target: target(spec.target),
        kind: match spec.kind.as_str() {
            "routine" => TraceKind::Routine,
            "episode" => TraceKind::Episode,
            other => panic!("the spec has no trace kind {other:?}"),
        },
    }
}

fn generate(dir: &Path) -> Vec<PathBuf> {
    std::fs::create_dir_all(dir).unwrap();
    for stale in std::fs::read_dir(dir).unwrap() {
        std::fs::remove_file(stale.unwrap().path()).unwrap();
    }
    let quint = std::env::var("QUINT").unwrap_or_else(|_| "quint".to_owned());
    let status = Command::new(&quint)
        .args(["run", SPEC, "--mbt", "--max-steps=40", "--seed=0x74726163"])
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
    // The spec's cooldown is two ticks and its routine interval three.
    let mut schedule = TraceSchedule::new(
        [target(0), target(1)],
        Time::new::<second>(2.0),
        Time::new::<second>(3.0),
        at(0),
    );
    let mut started = 0;
    for (step, state) in trace.states.iter().map(|s| &s.value).enumerate().skip(1) {
        let now = at(state.now);
        let actual: Vec<TraceRequest> = match state.action.as_str() {
            "advance" => Vec::new(),
            "incident" => {
                let t = target(state.picks.t.get());
                let update = Incident {
                    role: role(t, &state.picks.r.get()),
                    started: WallTime::from_unix_micros(0),
                    ended: (!state.picks.is_open.get()).then(|| WallTime::from_unix_micros(0)),
                    severity: Severity::Outage,
                    cause: None,
                };
                schedule.incident(&update, now).into_iter().collect()
            }
            "tick" => schedule.tick(now),
            "finish" => schedule
                .finished(target(state.picks.t.get()), now)
                .into_iter()
                .collect(),
            other => panic!("unknown action {other}"),
        };
        let expected: Vec<TraceRequest> = state.started.iter().map(request).collect();
        started += expected.len();
        assert_eq!(actual, expected, "{}, step {step}", path.display());
    }
    started
}

#[test]
fn trace_schedule_conforms_to_the_quint_spec() {
    let dir = PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join("trace-schedule-traces");
    let traces = generate(&dir);
    assert!(
        traces.len() >= TRACES / 2,
        "quint produced {} traces",
        traces.len()
    );
    let started: usize = traces.iter().map(|t| replay(t)).sum();
    // The traces must actually exercise the schedule.
    assert!(
        started > TRACES,
        "only {started} traces started in {} traces",
        traces.len()
    );
}
