//! The published language for the browser (ADR-0019). TypeScript versions of these
//! types are generated into `web/src/api/generated/` by `cargo test -p
//! netwatch-adapter-web`. Never edit those files by hand. Change these types, and the
//! frontend's boundary parser, in the same commit.

use netwatch_adapter_sqlite::{StoredEvent, StoredSample};
use netwatch_history::{Loss, Measurement, Sample};
use netwatch_shared::uom::si::time::microsecond;
use serde::Serialize;
use ts_rs::TS;

/// A role that has samples.
#[derive(Debug, Clone, Serialize, TS)]
#[ts(export, export_to = "../../../web/src/api/generated/")]
pub struct RoleSummary {
    /// The role's key, for example `target:1` (see `Role::key`).
    pub key: String,
    /// When its latest sample was taken, in µs since the Unix epoch.
    #[ts(type = "number")]
    pub latest_us: i64,
}

/// A configured target, for labeling its rows.
#[derive(Debug, Clone, Serialize, TS)]
#[ts(export, export_to = "../../../web/src/api/generated/")]
pub struct TargetView {
    /// The target's role key, for example `target:11`.
    pub key: String,
    /// A name for people, for example `1.1.1.1 (DNS)`.
    pub name: String,
}

/// One sample. Exactly one of `rtt_us` and `loss` is set (ADR-0004).
#[derive(Debug, Clone, Serialize, TS)]
#[ts(export, export_to = "../../../web/src/api/generated/")]
pub struct SamplePoint {
    /// When the sample was taken, in µs since the Unix epoch.
    #[ts(type = "number")]
    pub at_us: i64,
    /// The epoch of the pin the probe traveled under.
    #[ts(type = "number")]
    pub epoch: u64,
    /// The round-trip time in µs, if answered.
    #[ts(type = "number | null")]
    pub rtt_us: Option<i64>,
    /// Why it wasn't answered: `timed-out`, `unreachable`, `refused`, or `send-failed`.
    pub loss: Option<String>,
}

/// A topology event, for an event marker.
#[derive(Debug, Clone, Serialize, TS)]
#[ts(export, export_to = "../../../web/src/api/generated/")]
pub struct EventPoint {
    /// When it happened, in µs since the Unix epoch.
    #[ts(type = "number")]
    pub at_us: i64,
    /// `epoch-started`, `epoch-ended`, `desynchronized`, or `resynchronized`.
    pub kind: String,
    /// The role it concerns, as a key.
    pub role: Option<String>,
    /// The epoch it concerns.
    #[ts(type = "number | null")]
    pub epoch: Option<i64>,
    /// A description for people.
    pub detail: Option<String>,
}

/// A message on the live WebSocket.
#[derive(Debug, Clone, Serialize, TS)]
#[serde(tag = "type", rename_all = "kebab-case")]
#[ts(export, export_to = "../../../web/src/api/generated/")]
pub enum LiveMessage {
    /// A new sample for `role`.
    Sample {
        /// The role's key.
        role: String,
        /// The sample.
        point: SamplePoint,
    },
    /// A new topology event.
    Event {
        /// The event.
        event: EventPoint,
    },
    /// The kernel layer's latest report.
    Kernel {
        /// The report.
        report: KernelReportView,
    },
}

/// The kernel layer's state (ADR-0008).
#[derive(Debug, Clone, Serialize, TS)]
#[ts(export, export_to = "../../../web/src/api/generated/")]
pub struct KernelReportView {
    /// `healthy`, `degraded`, `failing`, or `unknown`.
    pub status: String,
    /// Watched counters that grew since the previous reading.
    pub increases: Vec<CounterIncrease>,
    /// Counters that went backwards (reset), whose difference means nothing.
    pub resets: Vec<String>,
    /// PSI `some avg10` in percent, if available.
    pub pressure: Option<PressureView>,
}

/// One watched counter's growth.
#[derive(Debug, Clone, Serialize, TS)]
#[ts(export, export_to = "../../../web/src/api/generated/")]
pub struct CounterIncrease {
    /// `Group.Name`, as in `/proc/net/snmp`.
    pub name: String,
    /// How much it grew.
    #[ts(type = "number")]
    pub by: u64,
}

/// Pressure stall information, in percent.
#[derive(Debug, Clone, Serialize, TS)]
#[ts(export, export_to = "../../../web/src/api/generated/")]
pub struct PressureView {
    /// Stalled on CPU.
    pub cpu: f64,
    /// Stalled on memory.
    pub memory: f64,
    /// Stalled on I/O.
    pub io: f64,
}

impl From<&netwatch_host_health::KernelReport> for KernelReportView {
    fn from(report: &netwatch_host_health::KernelReport) -> Self {
        use netwatch_host_health::KernelStatus;
        KernelReportView {
            status: match report.status {
                KernelStatus::Healthy => "healthy",
                KernelStatus::Degraded => "degraded",
                KernelStatus::Failing => "failing",
                KernelStatus::Unknown => "unknown",
            }
            .to_owned(),
            increases: report
                .increases
                .iter()
                .map(|(name, &by)| CounterIncrease {
                    name: name.clone(),
                    by,
                })
                .collect(),
            resets: report.resets.clone(),
            pressure: report.pressure.map(|p| PressureView {
                cpu: p.cpu,
                memory: p.memory,
                io: p.io,
            }),
        }
    }
}

fn loss_text(loss: Loss) -> &'static str {
    match loss {
        Loss::TimedOut => "timed-out",
        Loss::Unreachable(_) => "unreachable",
        Loss::Refused(_) => "refused",
        Loss::SendFailed => "send-failed",
    }
}

fn point(at_us: i64, epoch: u64, measurement: Measurement) -> SamplePoint {
    let (rtt_us, loss) = match measurement {
        Measurement::RoundTrip(rtt) => (Some(rtt.get::<microsecond>().round() as i64), None),
        Measurement::Lost(loss) => (None, Some(loss_text(loss).to_owned())),
    };
    SamplePoint {
        at_us,
        epoch,
        rtt_us,
        loss,
    }
}

impl From<&StoredSample> for SamplePoint {
    fn from(s: &StoredSample) -> Self {
        point(s.at.as_unix_micros(), s.epoch.get().get(), s.measurement)
    }
}

impl From<&Sample> for LiveMessage {
    fn from(s: &Sample) -> Self {
        LiveMessage::Sample {
            role: s.role.key(),
            point: point(s.at.as_unix_micros(), s.epoch.get().get(), s.measurement),
        }
    }
}

impl From<StoredEvent> for EventPoint {
    fn from(e: StoredEvent) -> Self {
        EventPoint {
            at_us: e.at.as_unix_micros(),
            kind: e.kind,
            role: e.role,
            epoch: e.epoch,
            detail: e.detail,
        }
    }
}
