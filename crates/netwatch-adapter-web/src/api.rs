//! The published language for the browser (ADR-0019). TypeScript versions of these
//! types are generated into `web/src/api/generated/` by `cargo test -p
//! netwatch-adapter-web`. Never edit those files by hand. Change these types, and the
//! frontend's boundary parser, in the same commit.

use netwatch_adapter_sqlite::{StoredEvent, StoredSample};
use netwatch_history::{Loss, Measurement, Reading, ReadingRollup, Sample};
use netwatch_probing::incident::{Incident, Severity};
use netwatch_probing::map::{EdgeKind, NetworkMap, Place};
use netwatch_shared::uom::si::time::microsecond;
use netwatch_shared::{Gauge, Role, TargetId, WallTime};
use std::collections::{BTreeMap, BTreeSet};
use std::net::IpAddr;

use serde::{Deserialize, Serialize};
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

/// A role's current identity and epoch, straight from the topology aggregate.
#[derive(Debug, Clone, Serialize, TS)]
#[ts(export, export_to = "../../../web/src/api/generated/")]
pub struct BindingView {
    /// The identity, for people: `eth0 (ifindex 5)`, `192.168.1.1`.
    pub identity: String,
    /// For an address, its router name in words, once it has been looked up
    /// (ADR-0040).
    pub name: Option<String>,
    /// The epoch during which the role holds it.
    #[ts(type = "number")]
    pub epoch: u64,
}

/// A status, as the API spells it: `healthy`, `degraded`, `failing`, `unknown`, or
/// `rate-limited`.
pub fn status_text(health: netwatch_probing::Health) -> String {
    use netwatch_probing::Health;
    match health {
        Health::Unknown => "unknown",
        Health::Healthy => "healthy",
        Health::RateLimited => "rate-limited",
        Health::Degraded => "degraded",
        Health::Failing => "failing",
    }
    .to_owned()
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
    /// Why it wasn't answered: `timed-out`, `unreachable`, `refused`,
    /// `unexpected-response`, or `send-failed`.
    pub loss: Option<String>,
}

/// One rollup: a tier interval of one role in one epoch (ADR-0006). RTT fields are
/// null when nothing in the interval was answered.
#[derive(Debug, Clone, Serialize, TS)]
#[ts(export, export_to = "../../../web/src/api/generated/")]
pub struct RollupPoint {
    /// The interval's start, in µs since the Unix epoch.
    #[ts(type = "number")]
    pub start_us: i64,
    /// The epoch every summarized sample belongs to.
    #[ts(type = "number")]
    pub epoch: u64,
    /// How many samples it summarizes.
    #[ts(type = "number")]
    pub count: u64,
    /// How many of them were lost.
    #[ts(type = "number")]
    pub lost: u64,
    /// The mean of the answered RTTs.
    #[ts(type = "number | null")]
    pub mean_us: Option<u64>,
    /// The median, within 10%.
    #[ts(type = "number | null")]
    pub p50_us: Option<u64>,
    /// The 95th percentile, within 10%.
    #[ts(type = "number | null")]
    pub p95_us: Option<u64>,
}

impl From<&netwatch_history::Rollup> for RollupPoint {
    fn from(r: &netwatch_history::Rollup) -> Self {
        let answered = r.summary.count.saturating_sub(r.summary.lost);
        RollupPoint {
            start_us: r.start.as_unix_micros(),
            epoch: r.epoch.get().get(),
            count: r.summary.count,
            lost: r.summary.lost,
            mean_us: (answered > 0).then(|| r.summary.rtt_sum_us / answered),
            p50_us: r.histogram.quantile_us(0.5),
            p95_us: r.histogram.quantile_us(0.95),
        }
    }
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
    /// The keys of the roles it concerns, whose rows it is drawn on (ADR-0031).
    pub roles: Vec<String>,
}

/// One gauge reading (ADR-0032).
#[derive(Debug, Clone, Serialize, TS)]
#[ts(export, export_to = "../../../web/src/api/generated/")]
pub struct ReadingPoint {
    /// The gauge's key.
    pub gauge: String,
    /// When it was read, in µs since the Unix epoch.
    #[ts(type = "number")]
    pub at_us: i64,
    /// The role's epoch when it was read.
    #[ts(type = "number")]
    pub epoch: u64,
    /// The value, in the gauge's unit.
    pub value: f64,
}

impl From<&Reading> for ReadingPoint {
    fn from(r: &Reading) -> Self {
        ReadingPoint {
            gauge: r.gauge.key().to_owned(),
            at_us: r.at.as_unix_micros(),
            epoch: r.epoch.get().get(),
            value: r.value,
        }
    }
}

/// One gauge's readings over one tier interval (ADR-0032).
#[derive(Debug, Clone, Serialize, TS)]
#[ts(export, export_to = "../../../web/src/api/generated/")]
pub struct ReadingRollupPoint {
    /// The gauge's key.
    pub gauge: String,
    /// The interval's start, in µs since the Unix epoch.
    #[ts(type = "number")]
    pub start_us: i64,
    /// The role's epoch.
    #[ts(type = "number")]
    pub epoch: u64,
    /// The mean value in the interval.
    pub mean: f64,
    /// The smallest value in the interval.
    pub min: f64,
    /// The largest value in the interval.
    pub max: f64,
}

impl ReadingRollupPoint {
    /// `None` for an empty interval, which has nothing to draw.
    pub fn new(r: &ReadingRollup) -> Option<Self> {
        Some(ReadingRollupPoint {
            gauge: r.gauge.key().to_owned(),
            start_us: r.start.as_unix_micros(),
            epoch: r.epoch.get().get(),
            mean: r.summary.mean()?,
            min: r.summary.min,
            max: r.summary.max,
        })
    }
}

/// A gauge, for labeling and formatting its readings.
#[derive(Debug, Clone, Serialize, TS)]
#[ts(export, export_to = "../../../web/src/api/generated/")]
pub struct GaugeView {
    /// The gauge's key, as readings name it.
    pub key: String,
    /// A short name, for people.
    pub label: String,
    /// `bit/s`.
    pub unit: String,
}

/// Every gauge netwatch knows.
pub fn gauges() -> Vec<GaugeView> {
    Gauge::ALL
        .iter()
        .map(|g| GaugeView {
            key: g.key().to_owned(),
            label: g.label().to_owned(),
            unit: g.unit().key().to_owned(),
        })
        .collect()
}

/// One incident: a role's degraded or failing period (ADR-0031).
#[derive(Debug, Clone, Serialize, TS)]
#[ts(export, export_to = "../../../web/src/api/generated/")]
pub struct IncidentView {
    /// The role's key.
    pub role: String,
    /// Where its band starts, in µs since the Unix epoch.
    #[ts(type = "number")]
    pub started_us: i64,
    /// Where its band ends; `null` while it's still open.
    #[ts(type = "number | null")]
    pub ended_us: Option<i64>,
    /// `degradation` or `outage`.
    pub severity: String,
    /// The probable cause's role key, if any.
    pub cause: Option<String>,
}

impl From<&Incident> for IncidentView {
    fn from(incident: &Incident) -> Self {
        IncidentView {
            role: incident.role.key(),
            started_us: incident.started.as_unix_micros(),
            ended_us: incident.ended.map(WallTime::as_unix_micros),
            severity: match incident.severity {
                Severity::Degradation => "degradation",
                Severity::Outage => "outage",
            }
            .to_owned(),
            cause: incident.cause.as_ref().map(Role::key),
        }
    }
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
    /// All history was flushed (ADR-0038): drop everything and backfill.
    Cleared,
    /// A gauge reading.
    Reading {
        /// The role's key.
        role: String,
        /// The reading.
        reading: ReadingPoint,
    },
    /// An incident opened, changed, or closed.
    Incident {
        /// The incident as it now stands.
        incident: IncidentView,
    },
    /// Every role's current status, graded by the daemon (ADR-0030).
    Statuses {
        /// Status by role key.
        statuses: BTreeMap<String, String>,
    },
    /// Every role's current identity, from Topology.
    Bindings {
        /// Binding by role key.
        bindings: BTreeMap<String, BindingView>,
    },
    /// The network map, whenever it changes (ADR-0042).
    Map {
        /// The map as it now stands.
        map: MapView,
    },
    /// A layer's latest health report.
    Layer {
        /// The layer's role key: `kernel` or `nic`.
        role: String,
        /// The report.
        report: LayerReportView,
    },
}

/// A layer's health (ADR-0008).
#[derive(Debug, Clone, Serialize, TS)]
#[ts(export, export_to = "../../../web/src/api/generated/")]
pub struct LayerReportView {
    /// `healthy`, `degraded`, `failing`, or `unknown`.
    pub status: String,
    /// What the layer is doing, in words, such as a radio's state (ADR-0036).
    pub notes: Vec<String>,
    /// Watched counters that grew since the previous reading.
    pub increases: Vec<CounterIncrease>,
    /// Counters that went backwards (reset), whose difference means nothing.
    pub resets: Vec<String>,
    /// PSI `some avg10` in percent, if available.
    pub pressure: Option<PressureView>,
    /// The connection-tracking table's fill level in percent, if conntrack is loaded.
    pub conntrack_fill: Option<f64>,
    /// Whether the clock is NTP-synchronized, if known.
    pub clock_synchronized: Option<bool>,
    /// The clock's estimated error in µs, if known.
    #[ts(type = "number | null")]
    pub clock_error_us: Option<i64>,
    /// The layer's sources that are disabled right now, and why (ADR-0007).
    pub disabled: Vec<String>,
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

impl From<&netwatch_host_health::LayerReport> for LayerReportView {
    fn from(report: &netwatch_host_health::LayerReport) -> Self {
        use netwatch_host_health::LayerStatus;
        LayerReportView {
            status: match report.status {
                LayerStatus::Healthy => "healthy",
                LayerStatus::Degraded => "degraded",
                LayerStatus::Failing => "failing",
                LayerStatus::Unknown => "unknown",
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
            conntrack_fill: report.conntrack_fill,
            clock_synchronized: report.clock.map(|(synchronized, _)| synchronized),
            clock_error_us: report.clock.map(|(_, error)| error),
            disabled: Vec::new(),
            notes: Vec::new(),
        }
    }
}

fn loss_text(loss: Loss) -> &'static str {
    match loss {
        Loss::TimedOut => "timed-out",
        Loss::Unreachable(_) => "unreachable",
        Loss::Refused(_) => "refused",
        Loss::UnexpectedResponse(_) => "unexpected-response",
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

impl EventPoint {
    /// An event, with the roles it concerns among `known` (every role the daemon
    /// grades), as [`concerns`] decides.
    pub fn new(e: StoredEvent, known: &[Role]) -> Self {
        let roles = concerns(&e.kind, e.role.as_deref().and_then(Role::from_key), known);
        EventPoint {
            at_us: e.at.as_unix_micros(),
            kind: e.kind,
            role: e.role,
            epoch: e.epoch,
            detail: e.detail,
            roles,
        }
    }
}

/// The rows an event belongs on (ADR-0031):
/// - a role's epoch starting or ending concerns that role and every known role
///   downstream of it, whose paths changed with it. The kernel's identity is
///   loopback, which no path goes through, so its epochs concern only itself.
/// - losing or regaining sync with the kernel's topology concerns the roles that
///   topology binds: the NIC, tunnel, and local interface
/// - an epoch resumed after a restart concerns no role: nothing changed (ADR-0044)
/// - anything else concerns the role it names, or the kernel if it names none
pub fn concerns(kind: &str, role: Option<Role>, known: &[Role]) -> Vec<String> {
    match (kind, role) {
        ("epoch-started" | "epoch-ended", Some(role)) if role != Role::Kernel => {
            std::iter::once(role.key())
                .chain(
                    known
                        .iter()
                        .filter(|other| role.is_upstream_of(other))
                        .map(Role::key),
                )
                .collect()
        }
        // A restart resumed the identity: nothing changed (ADR-0044).
        ("epoch-resumed", _) => Vec::new(),
        ("desynchronized" | "resynchronized", _) => [Role::Nic, Role::Tunnel, Role::LocalInterface]
            .iter()
            .map(Role::key)
            .collect(),
        (_, role) => vec![role.unwrap_or(Role::Kernel).key()],
    }
}

/// One target as the settings page edits it (ADR-0035): the file's own values, so
/// `dns = "system"` stays `system`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export, export_to = "../../../web/src/api/generated/")]
pub struct TargetSpec {
    /// The stable id, or `null` for a new target, which the daemon numbers.
    pub id: Option<u32>,
    /// A name for people; `null` for the daemon's default name.
    pub name: Option<String>,
    /// `icmp`, `dns`, or `https`.
    pub kind: String,
    /// An address for `icmp`; an address or `system` for `dns`; an `https://` URL.
    pub address: String,
    /// For `https`: the status the response must have.
    pub expect_status: Option<u16>,
    /// For `https`: text the response body must contain.
    pub expect_body: Option<String>,
}

/// The settings page's view of the configuration.
#[derive(Debug, Clone, Serialize, TS)]
#[ts(export, export_to = "../../../web/src/api/generated/")]
pub struct ConfigView {
    /// The file the daemon loaded, or `null` if it runs on its built-in defaults.
    pub path: Option<String>,
    /// Whether saving is possible. If not, `reason` says why.
    pub editable: bool,
    /// Why saving isn't possible, if it isn't.
    pub reason: Option<String>,
    /// Every configured target, in file order.
    pub targets: Vec<TargetSpec>,
}

/// Facts about the host and its connection, for the UI header and the report
/// (ADR-0039). Each is `null` until known.
#[derive(Debug, Clone, Default, Serialize, TS)]
#[ts(export, export_to = "../../../web/src/api/generated/")]
pub struct HostView {
    /// The host's name.
    pub hostname: Option<String>,
    /// The running kernel's release.
    pub kernel: Option<String>,
    /// This netwatch's version.
    pub version: String,
    /// The public address and where it enters Cloudflare, in words.
    pub public: Option<String>,
    /// The public address alone.
    pub public_address: Option<String>,
    /// The NIC's link facts, in words.
    pub link: Option<String>,
    /// The NIC's negotiated speed, in Mb/s.
    pub link_speed_mbps: Option<u32>,
    /// Whether the clock is NTP-synchronized.
    pub clock_synchronized: Option<bool>,
    /// The clock's estimated error, in µs.
    #[ts(type = "number | null")]
    pub clock_error_us: Option<i64>,
}

/// The network map (ADR-0042), as the Map tab draws it.
#[derive(Debug, Clone, Default, PartialEq, Serialize, TS)]
#[ts(export, export_to = "../../../web/src/api/generated/")]
pub struct MapView {
    /// Every node, by depth.
    pub nodes: Vec<MapNodeView>,
    /// Every edge.
    pub edges: Vec<MapEdgeView>,
}

/// One node of the network map (glossary: *map node*).
#[derive(Debug, Clone, PartialEq, Serialize, TS)]
#[ts(export, export_to = "../../../web/src/api/generated/")]
pub struct MapNodeView {
    /// A stable key: the role's key for a node of its own (`nic`, `target:1`,
    /// `hop:1:2`), `router:<address>:<ttl>`, or `silent:<target>:<ttl>`, named after
    /// the first target through it.
    pub id: String,
    /// `role`, `router`, or `silent`.
    pub kind: String,
    /// Its position along the path. Every edge goes to a greater depth.
    #[ts(type = "number")]
    pub depth: u16,
    /// For a router or a silent hop, its TTL.
    #[ts(type = "number | null")]
    pub ttl: Option<u8>,
    /// What it is, for people: an address or an interface.
    pub identity: Option<String>,
    /// For an address, its router name in words.
    pub name: Option<String>,
    /// The worst status of `roles`, as `status_text` spells it; `null` for a node seen
    /// only by a trace.
    pub status: Option<String>,
    /// The roles merged into it, each with its own status.
    pub roles: Vec<MapRoleView>,
    /// What the latest traces found here.
    pub sightings: Vec<SightingView>,
    /// The keys of the targets whose paths go through it.
    pub targets: Vec<String>,
}

/// A role merged into a map node.
#[derive(Debug, Clone, PartialEq, Serialize, TS)]
#[ts(export, export_to = "../../../web/src/api/generated/")]
pub struct MapRoleView {
    /// The role's key.
    pub key: String,
    /// Its own status.
    pub status: String,
}

/// What one target's latest trace found at a map node.
#[derive(Debug, Clone, PartialEq, Serialize, TS)]
#[ts(export, export_to = "../../../web/src/api/generated/")]
pub struct SightingView {
    /// The target's key.
    pub target: String,
    /// When the trace started, in µs since the Unix epoch.
    #[ts(type = "number")]
    pub at_us: i64,
    /// Probes sent at this TTL.
    #[ts(type = "number")]
    pub sent: u32,
    /// Of those, answered.
    #[ts(type = "number")]
    pub answered: u32,
    /// The median answer, in µs.
    #[ts(type = "number | null")]
    pub median_us: Option<i64>,
}

/// One edge of the network map (glossary: *map edge*).
#[derive(Debug, Clone, PartialEq, Serialize, TS)]
#[ts(export, export_to = "../../../web/src/api/generated/")]
pub struct MapEdgeView {
    /// The upstream node's id.
    pub from: String,
    /// The downstream node's id.
    pub to: String,
    /// `probed`, `traced`, `unmeasured`, or `off-path`.
    pub kind: String,
    /// The keys of the targets whose paths take it.
    pub targets: Vec<String>,
}

impl MapView {
    /// The map in words the UI can draw. `bindings` gives the layers' identities,
    /// `names` the router names known for addresses, and `traced` when each target's
    /// latest trace ran.
    pub fn new(
        map: &NetworkMap,
        bindings: &BTreeMap<String, BindingView>,
        names: &BTreeMap<IpAddr, String>,
        traced: &BTreeMap<TargetId, WallTime>,
    ) -> Self {
        let through = |place: &Place| -> BTreeSet<TargetId> {
            map.edges
                .iter()
                .filter(|e| &e.from == place || &e.to == place)
                .flat_map(|e| e.targets.iter().copied())
                .collect()
        };
        // A target's path has one place per TTL, so two silent places at one TTL never
        // share a target, and the first target through one names it uniquely.
        let ids: BTreeMap<&Place, String> = map
            .nodes
            .iter()
            .map(|node| {
                let id = match &node.place {
                    Place::Role(role) => role.key(),
                    Place::Router { address, ttl } => format!("router:{address}:{}", ttl.get()),
                    Place::Silent { ttl, .. } => {
                        let first = through(&node.place).first().map_or(0, |t| t.get());
                        format!("silent:{first}:{}", ttl.get())
                    }
                };
                (&node.place, id)
            })
            .collect();
        let id = |place: &Place| ids.get(place).cloned().unwrap_or_default();
        let target_keys =
            |targets: &BTreeSet<TargetId>| targets.iter().map(|&t| Role::Target(t).key()).collect();
        let nodes = map
            .nodes
            .iter()
            .map(|node| {
                let (kind, ttl, identity, name) = match &node.place {
                    Place::Role(role) => {
                        let binding = bindings.get(&role.key());
                        (
                            "role",
                            None,
                            binding.map(|b| b.identity.clone()),
                            binding.and_then(|b| b.name.clone()),
                        )
                    }
                    Place::Router { address, ttl } => (
                        "router",
                        Some(ttl.get().get()),
                        Some(address.to_string()),
                        names.get(address).cloned(),
                    ),
                    Place::Silent { ttl, .. } => ("silent", Some(ttl.get().get()), None, None),
                };
                MapNodeView {
                    id: id(&node.place),
                    kind: kind.to_owned(),
                    depth: node.depth,
                    ttl,
                    identity,
                    name,
                    status: node.health.map(status_text),
                    roles: node
                        .roles
                        .iter()
                        .map(|(role, &health)| MapRoleView {
                            key: role.key(),
                            status: status_text(health),
                        })
                        .collect(),
                    sightings: node
                        .sightings
                        .iter()
                        .filter_map(|s| {
                            Some(SightingView {
                                target: Role::Target(s.target).key(),
                                at_us: traced.get(&s.target)?.as_unix_micros(),
                                sent: s.sent,
                                answered: s.answered,
                                median_us: s.median.map(|m| m.get::<microsecond>().round() as i64),
                            })
                        })
                        .collect(),
                    targets: target_keys(&through(&node.place)),
                }
            })
            .collect();
        let edges = map
            .edges
            .iter()
            .map(|edge| MapEdgeView {
                from: id(&edge.from),
                to: id(&edge.to),
                kind: match edge.kind {
                    EdgeKind::Probed => "probed",
                    EdgeKind::Traced => "traced",
                    EdgeKind::Unmeasured => "unmeasured",
                    EdgeKind::OffPath => "off-path",
                }
                .to_owned(),
                targets: target_keys(&edge.targets),
            })
            .collect();
        MapView { nodes, edges }
    }
}
