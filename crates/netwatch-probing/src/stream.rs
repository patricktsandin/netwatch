//! A probe stream: fixed-cadence sends, timeouts, and reply matching.

use std::collections::BTreeMap;

use netwatch_shared::Instant;
use netwatch_shared::uom::si::f64::Time;
use netwatch_verified::sequence_of;

use crate::StreamConfig;

/// A probe's position in its stream, starting at 0. It doubles as the 64-bit token
/// carried in the echo payload (ADR-0023).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct ProbeIndex(u64);

impl ProbeIndex {
    /// Wraps an index, for example one decoded from a reply's payload token.
    pub const fn new(raw: u64) -> Self {
        Self(raw)
    }

    /// The index, as it is written into the token.
    pub const fn get(self) -> u64 {
        self.0
    }
}

/// The on-the-wire sequence number: the index modulo the sequence space.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct ProbeSequence(u16);

impl ProbeSequence {
    /// Wraps a sequence number read from a reply.
    pub const fn new(raw: u16) -> Self {
        Self(raw)
    }

    /// The number to put in the ICMP sequence field.
    pub const fn get(self) -> u16 {
        self.0
    }
}

/// The ICMP code of a destination-unreachable error, kept as a code so that the
/// domain doesn't have to interpret every value.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct UnreachableCode(pub u8);

/// Why the adapter couldn't send a probe.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum SendFailure {
    /// The pinned interface no longer exists (ADR-0020).
    InterfaceGone,
    /// The kernel has no route to the destination through the pinned interface.
    NoRoute,
    /// Any other error, as an errno.
    Other(i32),
}

/// What came back from the network, already parsed by the adapter.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Reply {
    /// The sequence number in the reply (or in the original header an error quotes).
    pub sequence: ProbeSequence,
    /// The token from the echoed payload. `None` for an ICMP error that quotes too
    /// little of the original packet to include it.
    pub token: Option<ProbeIndex>,
    /// What kind of reply this is.
    pub kind: ReplyKind,
}

/// The kind of a reply.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ReplyKind {
    /// An echo reply.
    Echo,
    /// A destination-unreachable error.
    Unreachable(UnreachableCode),
}

/// The one outcome every probe gets. These are never collapsed into a missing value.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum ProbeOutcome {
    /// Answered after `rtt`, measured from the send.
    Answered {
        /// The round-trip time.
        rtt: Time,
    },
    /// No answer within the timeout.
    TimedOut,
    /// An ICMP destination-unreachable error answered the probe.
    Unreachable {
        /// The error's ICMP code.
        code: UnreachableCode,
    },
    /// The probe could not be sent.
    SendFailed {
        /// Why the send failed.
        cause: SendFailure,
    },
}

/// One output of a [`ProbeStream`], to be handled in order.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Output {
    /// Send probe `index` now, with sequence number `sequence` and `index` as the
    /// token.
    Send {
        /// The probe's index, to use as its token.
        index: ProbeIndex,
        /// The sequence number to put on the wire.
        sequence: ProbeSequence,
    },
    /// Probe `index` has its outcome.
    Resolved {
        /// The probe that was resolved.
        index: ProbeIndex,
        /// Its outcome.
        outcome: ProbeOutcome,
    },
}

#[derive(Debug, Clone, Copy)]
struct InFlight {
    index: ProbeIndex,
    sent_at: Instant,
}

/// One stream of probes at a fixed cadence: the state machine specified by the
/// "Stream" section of `specs/probing.qnt`.
///
/// It hides sequence allocation, wraparound, timeouts, and matching. Callers call
/// [`tick`](Self::tick) at [`next_wake`](Self::next_wake) or later, carry out each
/// [`Output::Send`], report sends that failed, and pass replies in. What comes back
/// is exactly one [`Output::Resolved`] per probe.
#[derive(Debug)]
pub struct ProbeStream {
    config: StreamConfig,
    next_index: u64,
    next_due: Instant,
    in_flight: BTreeMap<ProbeSequence, InFlight>,
    retired: bool,
}

impl ProbeStream {
    /// A stream whose first probe is due at `first_due`.
    pub fn new(config: StreamConfig, first_due: Instant) -> Self {
        Self {
            config,
            next_index: 0,
            next_due: first_due,
            in_flight: BTreeMap::new(),
            retired: false,
        }
    }

    /// Stops sending, for example because the stream's pin has changed (ADR-0020).
    /// The probes already in flight still get their outcomes: keep ticking and
    /// passing replies in until [`is_idle`](Self::is_idle).
    pub fn retire(&mut self) {
        self.retired = true;
    }

    /// Whether a retired stream has resolved every probe it sent, so it can be
    /// dropped without leaving any probe without an outcome.
    pub fn is_idle(&self) -> bool {
        self.retired && self.in_flight.is_empty()
    }

    /// When the stream next needs a [`tick`](Self::tick): its next send, or its
    /// earliest timeout, whichever comes first.
    pub fn next_wake(&self) -> Instant {
        self.in_flight
            .values()
            .map(|p| p.sent_at.after(self.config.timeout()))
            .fold(self.next_due, Ord::min)
    }

    /// Advances to `now`. Expires every probe whose timeout has passed, in sequence
    /// order, and then sends the next probe if one is due. Expiring first frees a
    /// sequence number before it can be reused (ADR-0023).
    ///
    /// If the stream has fallen more than one cadence behind, for example after a
    /// stalled timer or a suspend, the missed slots are skipped and the stream keeps
    /// its phase, instead of sending a burst.
    pub fn tick(&mut self, now: Instant) -> Vec<Output> {
        let timeout = self.config.timeout();
        let expired: Vec<ProbeSequence> = self
            .in_flight
            .iter()
            .filter(|(_, p)| p.sent_at.after(timeout) <= now)
            .map(|(&sequence, _)| sequence)
            .collect();
        let mut outputs: Vec<Output> = expired
            .into_iter()
            .filter_map(|sequence| self.in_flight.remove(&sequence))
            .map(|p| Output::Resolved {
                index: p.index,
                outcome: ProbeOutcome::TimedOut,
            })
            .collect();

        if !self.retired && self.next_due <= now {
            let cadence = self.config.cadence();
            while self.next_due.after(cadence) <= now {
                self.next_due = self.next_due.after(cadence);
            }
            let index = ProbeIndex(self.next_index);
            self.next_index = self
                .next_index
                .checked_add(1)
                .expect("u64 probe indices cannot be exhausted at any real cadence");
            let sequence = self.sequence(index);
            self.in_flight.insert(
                sequence,
                InFlight {
                    index,
                    sent_at: now,
                },
            );
            self.next_due = self.next_due.after(cadence);
            outputs.push(Output::Send { index, sequence });
        }
        outputs
    }

    /// The adapter couldn't send probe `index`. Ignored unless that probe is still
    /// in flight.
    pub fn send_failed(&mut self, index: ProbeIndex, cause: SendFailure) -> Vec<Output> {
        let sequence = self.sequence(index);
        match self.in_flight.get(&sequence) {
            Some(p) if p.index == index => {
                self.in_flight.remove(&sequence);
                vec![Output::Resolved {
                    index,
                    outcome: ProbeOutcome::SendFailed { cause },
                }]
            }
            _ => Vec::new(),
        }
    }

    /// A reply arrived at `now`. It resolves the in-flight probe with its sequence
    /// number, provided that its token (if it has one) is that probe's index. A late,
    /// duplicate, or foreign reply is ignored (ADR-0023).
    pub fn reply(&mut self, reply: Reply, now: Instant) -> Vec<Output> {
        let matches = self
            .in_flight
            .get(&reply.sequence)
            .is_some_and(|p| reply.token.is_none_or(|token| token == p.index));
        if !matches {
            return Vec::new();
        }
        let Some(p) = self.in_flight.remove(&reply.sequence) else {
            return Vec::new();
        };
        let outcome = match reply.kind {
            ReplyKind::Echo => ProbeOutcome::Answered {
                rtt: now.since(p.sent_at),
            },
            ReplyKind::Unreachable(code) => ProbeOutcome::Unreachable { code },
        };
        vec![Output::Resolved {
            index: p.index,
            outcome,
        }]
    }

    fn sequence(&self, index: ProbeIndex) -> ProbeSequence {
        let raw = sequence_of(index.0, u64::from(self.config.space().size()));
        ProbeSequence(u16::try_from(raw).expect("verified: sequence_of < space ≤ 2^16"))
    }
}
