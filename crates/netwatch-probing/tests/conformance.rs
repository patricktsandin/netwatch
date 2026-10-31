//! Trace conformance: `ProbeStream` must behave exactly like the "Stream" section of
//! `specs/probing.qnt` (ADR-0016).
//!
//! Quint generates traces, which this test replays against a stream configured with
//! the spec's bounds: 1 tick = 1 s, cadence 1 s, timeout 2 s, sequence space 4.
//! - `advance` becomes `tick(now')`
//! - `failLastSend` becomes `send_failed` for the probe sent in the step before
//! - `deliverReply` becomes `reply` with the reply the spec picked
//!
//! The outputs of every step must be identical, except for probes at or beyond the
//! spec's `MAX_PROBES`, which bounds the model and not the stream. A real stream keeps
//! sending, so the harness drops outputs about those probes. That is sound because
//! such a probe is sent only after every probe sharing its sequence number has
//! expired, so it can only absorb replies the spec ignores anyway.
//!
//! Needs `quint` on `PATH`, or set `QUINT`.

#![allow(clippy::unwrap_used)] // Tests may unwrap (ADR-0015), including in helpers.

use std::path::{Path, PathBuf};
use std::process::Command;

use netwatch_probing::{
    Output, ProbeIndex, ProbeOutcome, ProbeSequence, ProbeStream, Reply, ReplyKind, SendFailure,
    SequenceSpace, StreamConfig, UnreachableCode,
};
use netwatch_shared::Instant;
use netwatch_shared::uom::si::f64::Time;
use netwatch_shared::uom::si::time::second;
use serde::Deserialize;
use serde::de::IgnoredAny;

const SPEC: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/../../specs/probing.qnt");
const TRACES: usize = 200;
const UNREACHABLE: UnreachableCode = UnreachableCode(1);
const SEND_FAILURE: SendFailure = SendFailure::Other(0);
const MAX_PROBES: u64 = 6;

fn within_model(output: &Output) -> bool {
    let (Output::Send { index, .. } | Output::Resolved { index, .. }) = output;
    index.get() < MAX_PROBES
}

#[derive(Debug, Deserialize)]
struct SpecState {
    now: i64,
    outputs: Vec<SpecOutput>,
    #[serde(rename = "mbt::actionTaken")]
    action: String,
    #[serde(rename = "mbt::nondetPicks")]
    picks: Picks,
}

#[derive(Debug, Deserialize)]
struct Picks {
    r: SpecOption<SpecReply>,
}

#[derive(Debug, Deserialize)]
#[serde(tag = "tag", content = "value")]
enum SpecOption<T> {
    Some(T),
    None(IgnoredAny),
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct SpecReply {
    seq: i64,
    token: i64,
    has_token: bool,
    kind: SpecKind,
}

#[derive(Debug, Deserialize)]
#[serde(tag = "tag", content = "value")]
enum SpecKind {
    Echo(IgnoredAny),
    Unreachable(IgnoredAny),
}

#[derive(Debug, Deserialize)]
#[serde(tag = "tag", content = "value")]
enum SpecOutput {
    Send { index: i64, seq: i64 },
    Resolved { index: i64, outcome: SpecOutcome },
}

#[derive(Debug, Deserialize)]
#[serde(tag = "tag", content = "value")]
enum SpecOutcome {
    Answered(i64),
    TimedOut(IgnoredAny),
    #[serde(rename = "Unreachable_")]
    Unreachable(IgnoredAny),
    SendFailed(IgnoredAny),
}

fn instant(ticks: i64) -> Instant {
    Instant::from_nanos(u64::try_from(ticks).unwrap() * 1_000_000_000)
}

fn index(spec: i64) -> ProbeIndex {
    ProbeIndex::new(u64::try_from(spec).unwrap())
}

fn sequence(spec: i64) -> ProbeSequence {
    ProbeSequence::new(u16::try_from(spec).unwrap())
}

fn output(spec: &SpecOutput) -> Output {
    match *spec {
        SpecOutput::Send { index: i, seq } => Output::Send {
            index: index(i),
            sequence: sequence(seq),
        },
        SpecOutput::Resolved {
            index: i,
            ref outcome,
        } => Output::Resolved {
            index: index(i),
            outcome: match *outcome {
                SpecOutcome::Answered(rtt) => ProbeOutcome::Answered {
                    rtt: Time::new::<second>(rtt as f64),
                },
                SpecOutcome::TimedOut(_) => ProbeOutcome::TimedOut,
                SpecOutcome::Unreachable(_) => ProbeOutcome::Unreachable { code: UNREACHABLE },
                SpecOutcome::SendFailed(_) => ProbeOutcome::SendFailed {
                    cause: SEND_FAILURE,
                },
            },
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
        .args([
            "run",
            SPEC,
            "--mbt",
            "--max-steps=40",
            "--seed=0x70726f6265",
        ])
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

fn replay(path: &Path) {
    let trace: itf::Trace<SpecState> =
        itf::trace_from_str(&std::fs::read_to_string(path).unwrap()).unwrap();
    let states: Vec<&SpecState> = trace.states.iter().map(|s| &s.value).collect();
    let config = StreamConfig::new(
        Time::new::<second>(1.0),
        Time::new::<second>(2.0),
        SequenceSpace::new(4).unwrap(),
    )
    .unwrap();
    let mut stream = ProbeStream::new(config, instant(0));

    for (step, pair) in states.windows(2).enumerate() {
        let (before, after) = (pair[0], pair[1]);
        let outputs = match after.action.as_str() {
            "advance" => stream.tick(instant(after.now)),
            "failLastSend" => {
                let last = before
                    .outputs
                    .iter()
                    .rev()
                    .find_map(|o| match *o {
                        SpecOutput::Send { index: i, .. } => Some(index(i)),
                        SpecOutput::Resolved { .. } => None,
                    })
                    .unwrap();
                stream.send_failed(last, SEND_FAILURE)
            }
            "deliverReply" => {
                let SpecOption::Some(ref r) = after.picks.r else {
                    panic!("deliverReply without a picked reply");
                };
                let reply = Reply {
                    sequence: sequence(r.seq),
                    token: r.has_token.then(|| index(r.token)),
                    kind: match r.kind {
                        SpecKind::Echo(_) => ReplyKind::Echo,
                        SpecKind::Unreachable(_) => ReplyKind::Unreachable(UNREACHABLE),
                    },
                };
                stream.reply(reply, instant(after.now))
            }
            other => panic!("unknown action {other}"),
        };
        let outputs: Vec<Output> = outputs.into_iter().filter(within_model).collect();
        let expected: Vec<Output> = after.outputs.iter().map(output).collect();
        assert_eq!(
            outputs,
            expected,
            "{}, step {}: {}",
            path.display(),
            step + 1,
            after.action
        );
    }
}

#[test]
fn probe_stream_conforms_to_the_quint_spec() {
    let dir = PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join("probing-traces");
    let traces = generate(&dir);
    assert!(
        traces.len() >= TRACES / 2,
        "quint produced {} traces",
        traces.len()
    );
    for trace in &traces {
        replay(trace);
    }
}
