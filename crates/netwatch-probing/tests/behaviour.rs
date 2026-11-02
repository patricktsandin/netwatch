//! `ProbeStream` and `StreamConfig` behaviour, as examples in glossary terms. The
//! exhaustive check is conformance against `specs/probing.qnt`.

#![allow(clippy::unwrap_used)] // Tests may unwrap (ADR-0015), including in helpers.

use netwatch_probing::{
    ExchangeStage, InvalidStreamConfig, Output, ProbeIndex, ProbeOutcome, ProbeSequence,
    ProbeStream, Reply, ReplyKind, SendFailure, SequenceSpace, StreamConfig, UnreachableCode,
};
use netwatch_shared::Instant;
use netwatch_shared::uom::si::f64::Time;
use netwatch_shared::uom::si::time::{millisecond, second};

fn secs(s: f64) -> Time {
    Time::new::<second>(s)
}

fn at(ms: u64) -> Instant {
    Instant::from_nanos(ms * 1_000_000)
}

fn icmp_stream() -> ProbeStream {
    let config = StreamConfig::new(secs(1.0), secs(0.8), SequenceSpace::ICMP).unwrap();
    ProbeStream::new(config, at(0))
}

fn echo(index: u64) -> Reply {
    Reply {
        sequence: ProbeSequence::new(u16::try_from(index % 65_536).unwrap()),
        token: Some(ProbeIndex::new(index)),
        kind: ReplyKind::Echo,
    }
}

fn sent(index: u64) -> Output {
    Output::Send {
        index: ProbeIndex::new(index),
        sequence: ProbeSequence::new(u16::try_from(index % 65_536).unwrap()),
    }
}

#[test]
fn config_rejects_a_timeout_that_outlives_the_sequence_space() {
    let space = SequenceSpace::new(4).unwrap();
    assert_eq!(
        StreamConfig::new(secs(1.0), secs(4.0), space),
        Err(InvalidStreamConfig::TimeoutOutlivesSequenceSpace)
    );
    assert!(StreamConfig::new(secs(1.0), secs(3.9), space).is_ok());
}

#[test]
fn config_rejects_non_positive_durations() {
    let space = SequenceSpace::ICMP;
    assert_eq!(
        StreamConfig::new(secs(0.0), secs(1.0), space),
        Err(InvalidStreamConfig::NonPositiveCadence)
    );
    assert_eq!(
        StreamConfig::new(secs(1.0), secs(-1.0), space),
        Err(InvalidStreamConfig::NonPositiveTimeout)
    );
}

#[test]
fn first_probe_is_sent_when_due_and_not_before() {
    let config = StreamConfig::new(secs(1.0), secs(0.8), SequenceSpace::ICMP).unwrap();
    let mut stream = ProbeStream::new(config, at(500));

    assert_eq!(stream.tick(at(499)), vec![]);
    assert_eq!(stream.tick(at(500)), vec![sent(0)]);
}

#[test]
fn echo_reply_resolves_its_probe_with_the_round_trip_time() {
    let mut stream = icmp_stream();
    stream.tick(at(0));

    let outputs = stream.reply(echo(0), at(12));

    assert_eq!(
        outputs,
        vec![Output::Resolved {
            index: ProbeIndex::new(0),
            outcome: ProbeOutcome::Answered {
                rtt: Time::new::<millisecond>(12.0)
            },
        }]
    );
}

#[test]
fn duplicate_reply_is_ignored() {
    let mut stream = icmp_stream();
    stream.tick(at(0));
    stream.reply(echo(0), at(12));

    assert_eq!(stream.reply(echo(0), at(13)), vec![]);
}

#[test]
fn reply_with_another_probes_token_is_ignored() {
    let mut stream = icmp_stream();
    stream.tick(at(0));
    let foreign = Reply {
        token: Some(ProbeIndex::new(65_536)),
        ..echo(0)
    };

    assert_eq!(stream.reply(foreign, at(12)), vec![]);
}

#[test]
fn unanswered_probe_times_out_exactly_at_its_deadline() {
    let mut stream = icmp_stream();
    stream.tick(at(0));

    assert_eq!(stream.tick(at(799)), vec![]);
    assert_eq!(
        stream.tick(at(800)),
        vec![Output::Resolved {
            index: ProbeIndex::new(0),
            outcome: ProbeOutcome::TimedOut
        }]
    );
}

#[test]
fn reply_after_the_timeout_is_ignored() {
    let mut stream = icmp_stream();
    stream.tick(at(0));
    stream.tick(at(800));

    assert_eq!(stream.reply(echo(0), at(900)), vec![]);
}

#[test]
fn unreachable_error_without_a_token_resolves_by_sequence() {
    let mut stream = icmp_stream();
    stream.tick(at(0));
    let error = Reply {
        token: None,
        kind: ReplyKind::Unreachable(UnreachableCode(3)),
        ..echo(0)
    };

    assert_eq!(
        stream.reply(error, at(5)),
        vec![Output::Resolved {
            index: ProbeIndex::new(0),
            outcome: ProbeOutcome::Unreachable {
                code: UnreachableCode(3)
            },
        }]
    );
}

#[test]
fn send_failure_resolves_the_probe_immediately() {
    let mut stream = icmp_stream();
    stream.tick(at(0));

    assert_eq!(
        stream.send_failed(ProbeIndex::new(0), SendFailure::InterfaceGone),
        vec![Output::Resolved {
            index: ProbeIndex::new(0),
            outcome: ProbeOutcome::SendFailed {
                cause: SendFailure::InterfaceGone
            },
        }]
    );
}

#[test]
fn cadence_is_fixed_and_independent_of_replies() {
    let mut stream = icmp_stream();
    stream.tick(at(0));
    stream.reply(echo(0), at(300));

    assert_eq!(stream.tick(at(999)), vec![]);
    assert_eq!(stream.tick(at(1_000)), vec![sent(1)]);
}

/// ADR-0023: a stream that falls far behind, for example after a suspend, skips
/// the missed slots and keeps its phase. It doesn't send a burst.
#[test]
fn stream_that_falls_behind_skips_missed_slots_and_keeps_its_phase() {
    let mut stream = icmp_stream();
    stream.tick(at(0));

    let after_stall = stream.tick(at(5_300));

    assert_eq!(
        after_stall,
        vec![
            Output::Resolved {
                index: ProbeIndex::new(0),
                outcome: ProbeOutcome::TimedOut
            },
            sent(1),
        ]
    );
    assert_eq!(stream.next_wake(), at(6_000));
}

#[test]
fn next_wake_is_the_earlier_of_the_next_send_and_the_next_timeout() {
    let mut stream = icmp_stream();
    stream.tick(at(0));

    assert_eq!(stream.next_wake(), at(800));
}

#[test]
fn retired_stream_stops_sending_but_still_resolves_what_is_in_flight() {
    let mut stream = icmp_stream();
    stream.tick(at(0));
    stream.retire();

    assert!(!stream.is_idle());
    assert_eq!(
        stream.tick(at(1_000)).len(),
        1,
        "only the timeout, no new send"
    );
    assert!(stream.is_idle());
}

#[test]
fn time_exceeded_answers_a_hop_probe() {
    let config = StreamConfig::new(secs(1.0), secs(0.8), SequenceSpace::ICMP)
        .unwrap()
        .ttl_limited();
    let mut stream = ProbeStream::new(config, at(0));
    stream.tick(at(0));
    let hop = Reply {
        kind: ReplyKind::TimeExceeded,
        ..echo(0)
    };

    assert_eq!(
        stream.reply(hop, at(2)),
        vec![Output::Resolved {
            index: ProbeIndex::new(0),
            outcome: ProbeOutcome::Answered {
                rtt: Time::new::<millisecond>(2.0)
            },
        }]
    );
}

#[test]
fn time_exceeded_is_a_loss_on_a_stream_without_a_ttl_limit() {
    // A routing loop toward the target: some router's time exceeded is not the
    // target answering (ADR-0049).
    let mut stream = icmp_stream();
    stream.tick(at(0));
    let looped = Reply {
        kind: ReplyKind::TimeExceeded,
        ..echo(0)
    };

    assert_eq!(
        stream.reply(looped, at(2)),
        vec![Output::Resolved {
            index: ProbeIndex::new(0),
            outcome: ProbeOutcome::TimeExceeded,
        }]
    );
}

#[test]
fn an_exchange_that_failed_beyond_the_host_is_not_a_send_failure() {
    let mut stream = icmp_stream();
    stream.tick(at(0));
    let forged = Reply {
        kind: ReplyKind::ExchangeFailed(ExchangeStage::Tls),
        ..echo(0)
    };

    assert_eq!(
        stream.reply(forged, at(2)),
        vec![Output::Resolved {
            index: ProbeIndex::new(0),
            outcome: ProbeOutcome::ExchangeFailed {
                stage: ExchangeStage::Tls
            },
        }]
    );
}
