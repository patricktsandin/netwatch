//! The open host service: the REST and WebSocket API, plus the embedded SPA
//! (ADR-0005, ADR-0019).
//!
//! - `GET /api/roles`: every role that has samples
//! - `GET /api/samples?role=<key>&since_us=<µs>`: one role's samples, oldest first
//! - `GET /api/rollups?role=<key>&tier=minute|hour&since_us=<µs>`: one role's rollups
//! - `GET /api/readings?role=<key>&since_us=<µs>`: one role's gauge readings
//! - `GET /api/reading-rollups?role=<key>&tier=minute|hour&since_us=<µs>`: their rollups
//! - `GET /api/gauges`: every gauge's label and unit
//! - `GET /api/events?since_us=<µs>`: events, oldest first, each with the roles it
//!   concerns
//! - `GET /api/incidents?since_us=<µs>[&until_us=<µs>]`: every incident overlapping
//!   that span (until now, by default), oldest first
//! - `GET /api/targets`: every configured target's name
//! - `GET /api/layers`: each layer's latest health report, by role key
//! - `GET /api/status`: every role's current status, graded by the daemon (ADR-0030)
//! - `GET /api/host`: facts about the host and its connection (ADR-0039)
//! - `GET /api/bindings`: every role's current identity and epoch, from Topology
//! - `GET /api/map`: the network map (ADR-0042)
//! - `GET /api/rows`: the rows several hop roles share, with their statuses (ADR-0045)
//! - `GET /api/config`: the targets as the settings page edits them
//! - `PUT /api/config`: replaces the targets, then the daemon restarts (ADR-0035)
//! - `DELETE /api/history`: flushes all history (ADR-0038)
//! - `GET /api/live`: a WebSocket of [`LiveMessage`]s
//! - `GET /report?from_us=<µs>&until_us=<µs>`: an incident report as a printable page,
//!   `GET /report.txt?…` as plain text (ADR-0033), and `GET /report.csv?…`, the raw
//!   samples around each episode (ADR-0039)
//! - everything else: the SPA from `web/dist`, embedded in release builds
//!
//! Reads open their own SQLite connection, which WAL mode lets run alongside the
//! single writer. Every read first asks the writer to flush, so a backfill never
//! misses a sample that the live stream already sent (ADR-0029,
//! `specs/live_backfill.qnt`).

pub mod api;
pub mod report;

use std::collections::BTreeMap;
use std::net::SocketAddr;
use std::path::PathBuf;

use axum::extract::ws::{Message, WebSocket, WebSocketUpgrade};
use axum::extract::{Query, State};
use axum::http::{HeaderMap, StatusCode, Uri, header};
use axum::response::{IntoResponse, Response};
use axum::routing::get;
use axum::{Json, Router};
use netwatch_adapter_sqlite::Store;
use netwatch_shared::{Role, WallTime};
use serde::Deserialize;
use tokio::sync::{broadcast, watch};

pub use api::{
    BindingView, EventPoint, GaugeView, IncidentView, LayerReportView, LiveMessage, MapView,
    ReadingPoint, ReadingRollupPoint, RoleSummary, RollupPoint, RowView, SamplePoint, TargetView,
    status_text,
};
pub use api::{ConfigView, HostView, TargetSpec};

#[derive(rust_embed::RustEmbed)]
#[folder = "../../web/dist"]
#[allow_missing = true]
struct Spa;

/// Asks the writer to write everything it has received, and resolves once it has.
pub type Flush = std::sync::Arc<dyn Fn() -> tokio::sync::oneshot::Receiver<()> + Send + Sync>;

/// Asks the writer to delete all history (ADR-0038), and resolves with the outcome.
pub type FlushHistory =
    std::sync::Arc<dyn Fn() -> tokio::sync::oneshot::Receiver<Result<(), String>> + Send + Sync>;

/// Reads and writes the configuration for the settings page (ADR-0035). The
/// composition root owns the file format, so it implements this.
pub trait Settings: Send + Sync {
    /// The targets as configured, and whether they can be saved.
    fn view(&self) -> ConfigView;
    /// Validates and saves `targets` as the complete list, then arranges for the
    /// daemon to restart with them. An error is a message for people.
    fn save(&self, targets: Vec<TargetSpec>) -> Result<(), String>;
}

/// The daemon's current view, kept up to date by the composition root.
#[derive(Debug, Clone, Default)]
pub struct Current {
    /// Every role's status, by role key.
    pub statuses: BTreeMap<String, String>,
    /// Every role's binding, by role key.
    pub bindings: BTreeMap<String, BindingView>,
    /// Facts about the host and its connection (ADR-0039).
    pub host: HostView,
    /// Every router name looked up so far, by address (ADR-0040).
    pub names: BTreeMap<String, String>,
    /// The network map (ADR-0042).
    pub map: MapView,
    /// The shared hop rows (ADR-0045).
    pub rows: Vec<RowView>,
}

impl Current {
    /// Every role the daemon grades, which is every role that has a row.
    pub fn known_roles(&self) -> Vec<Role> {
        self.statuses
            .keys()
            .filter_map(|key| Role::from_key(key))
            .collect()
    }
}

#[derive(Clone)]
struct AppState {
    flush: Flush,
    current: watch::Receiver<Current>,
    database: PathBuf,
    /// Reader connections kept open between reads, so that a backfill's many reads
    /// don't each open the database, set its pragmas, and check its migrations.
    readers: std::sync::Arc<std::sync::Mutex<Vec<Store>>>,
    live: broadcast::Sender<LiveMessage>,
    layers: watch::Receiver<BTreeMap<String, LayerReportView>>,
    targets: std::sync::Arc<Vec<TargetView>>,
    settings: std::sync::Arc<dyn Settings>,
    flush_history: FlushHistory,
}

/// How many idle reader connections are kept.
const IDLE_READERS: usize = 4;

/// What the daemon gives the UI server: where history is, and its live views.
pub struct Daemon {
    /// The SQLite database, which reads open their own connections to.
    pub database: PathBuf,
    /// Live messages, broadcast to every WebSocket.
    pub live: broadcast::Sender<LiveMessage>,
    /// Each layer's latest health report.
    pub layers: watch::Receiver<BTreeMap<String, LayerReportView>>,
    /// Every configured target's name.
    pub targets: Vec<TargetView>,
    /// The flush barrier (ADR-0029).
    pub flush: Flush,
    /// Deletes all history (ADR-0038).
    pub flush_history: FlushHistory,
    /// Statuses and bindings.
    pub current: watch::Receiver<Current>,
    /// The configuration, for the settings page (ADR-0035).
    pub settings: std::sync::Arc<dyn Settings>,
}

/// Serves the API and the SPA on `address` until the process ends.
pub async fn serve(address: SocketAddr, daemon: Daemon) -> std::io::Result<()> {
    let app = Router::new()
        .route("/api/roles", get(roles))
        .route("/api/samples", get(samples))
        .route("/api/rollups", get(rollups))
        .route("/api/readings", get(readings))
        .route("/api/reading-rollups", get(reading_rollups))
        .route("/api/gauges", get(|| async { Json(api::gauges()) }))
        .route("/api/events", get(events))
        .route("/api/incidents", get(incidents))
        .route("/api/targets", get(target_list))
        .route("/api/layers", get(layer_reports))
        .route("/api/status", get(statuses))
        .route("/api/bindings", get(bindings))
        .route("/api/map", get(network_map))
        .route("/api/rows", get(shared_rows))
        .route("/api/host", get(host))
        .route("/api/config", get(config).put(save_config))
        .route("/api/history", axum::routing::delete(delete_history))
        .route("/api/live", get(live_socket))
        .route("/report", get(report_page))
        .route("/report.txt", get(report_text))
        .route("/report.csv", get(report_csv))
        .fallback(spa)
        .layer(axum::middleware::from_fn(guard_site))
        .with_state(AppState {
            flush: daemon.flush,
            current: daemon.current,
            database: daemon.database,
            readers: std::sync::Arc::default(),
            live: daemon.live,
            layers: daemon.layers,
            targets: std::sync::Arc::new(daemon.targets),
            settings: daemon.settings,
            flush_history: daemon.flush_history,
        });
    let listener = tokio::net::TcpListener::bind(address).await?;
    tracing::info!(%address, "serving the UI");
    axum::serve(listener, app).await
}

#[derive(Deserialize)]
struct SamplesQuery {
    role: String,
    since_us: i64,
}

#[derive(Deserialize)]
struct RollupsQuery {
    role: String,
    tier: String,
    since_us: i64,
}

#[derive(Deserialize)]
struct EventsQuery {
    since_us: i64,
}

#[derive(Deserialize)]
struct IncidentsQuery {
    since_us: i64,
    until_us: Option<i64>,
}

/// Runs a read on the blocking pool, against an idle reader connection or a new one.
async fn read<T: Send + 'static>(
    state: &AppState,
    query: impl FnOnce(&Store) -> Result<T, netwatch_adapter_sqlite::StoreError> + Send + 'static,
) -> Result<T, (StatusCode, String)> {
    // The flush barrier: if the writer has gone away, there's nothing left to wait
    // for, so the read goes ahead.
    let _ = (state.flush)().await;
    let database = state.database.clone();
    let readers = state.readers.clone();
    tokio::task::spawn_blocking(move || {
        let idle = readers
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .pop();
        let store = match idle {
            Some(store) => store,
            None => Store::open(&database)?,
        };
        let result = query(&store);
        let mut idle = readers
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if idle.len() < IDLE_READERS {
            idle.push(store);
        }
        result
    })
    .await
    .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?
    .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))
}

async fn roles(
    State(state): State<AppState>,
) -> Result<Json<Vec<RoleSummary>>, (StatusCode, String)> {
    let roles = read(&state, |store| store.roles()).await?;
    Ok(Json(
        roles
            .into_iter()
            .map(|(role, latest)| RoleSummary {
                key: role.key(),
                latest_us: latest.as_unix_micros(),
            })
            .collect(),
    ))
}

async fn samples(
    State(state): State<AppState>,
    Query(query): Query<SamplesQuery>,
) -> Result<Json<Vec<SamplePoint>>, (StatusCode, String)> {
    let role = parse_role(&query.role)?;
    let since = WallTime::from_unix_micros(query.since_us);
    let samples = read(&state, move |store| store.samples(&role, since)).await?;
    Ok(Json(samples.iter().map(SamplePoint::from).collect()))
}

async fn rollups(
    State(state): State<AppState>,
    Query(query): Query<RollupsQuery>,
) -> Result<Json<Vec<RollupPoint>>, (StatusCode, String)> {
    let role = parse_role(&query.role)?;
    let tier = parse_tier(&query.tier)?;
    let since = WallTime::from_unix_micros(query.since_us);
    let rollups = read(&state, move |store| store.rollups(&role, tier, since)).await?;
    Ok(Json(rollups.iter().map(RollupPoint::from).collect()))
}

async fn readings(
    State(state): State<AppState>,
    Query(query): Query<SamplesQuery>,
) -> Result<Json<Vec<ReadingPoint>>, (StatusCode, String)> {
    let role = parse_role(&query.role)?;
    let since = WallTime::from_unix_micros(query.since_us);
    let readings = read(&state, move |store| store.readings(&role, since)).await?;
    Ok(Json(readings.iter().map(ReadingPoint::from).collect()))
}

async fn reading_rollups(
    State(state): State<AppState>,
    Query(query): Query<RollupsQuery>,
) -> Result<Json<Vec<ReadingRollupPoint>>, (StatusCode, String)> {
    let role = parse_role(&query.role)?;
    let tier = parse_tier(&query.tier)?;
    let since = WallTime::from_unix_micros(query.since_us);
    let rollups = read(&state, move |store| {
        store.reading_rollups(&role, tier, since)
    })
    .await?;
    Ok(Json(
        rollups.iter().filter_map(ReadingRollupPoint::new).collect(),
    ))
}

fn parse_role(key: &str) -> Result<Role, (StatusCode, String)> {
    Role::from_key(key).ok_or_else(|| (StatusCode::BAD_REQUEST, format!("unknown role {key:?}")))
}

fn parse_tier(tier: &str) -> Result<netwatch_history::Tier, (StatusCode, String)> {
    match tier {
        "minute" => Ok(netwatch_history::Tier::Minute),
        "hour" => Ok(netwatch_history::Tier::Hour),
        other => Err((StatusCode::BAD_REQUEST, format!("unknown tier {other:?}"))),
    }
}

async fn events(
    State(state): State<AppState>,
    Query(query): Query<EventsQuery>,
) -> Result<Json<Vec<EventPoint>>, (StatusCode, String)> {
    let since = WallTime::from_unix_micros(query.since_us);
    let events = read(&state, move |store| store.events(since)).await?;
    let known = state.current.borrow().known_roles();
    Ok(Json(
        events
            .into_iter()
            .map(|event| EventPoint::new(event, &known))
            .collect(),
    ))
}

async fn incidents(
    State(state): State<AppState>,
    Query(query): Query<IncidentsQuery>,
) -> Result<Json<Vec<IncidentView>>, (StatusCode, String)> {
    let from = WallTime::from_unix_micros(query.since_us);
    let until = WallTime::from_unix_micros(query.until_us.unwrap_or(i64::MAX));
    let incidents = read(&state, move |store| store.incidents(from, until)).await?;
    Ok(Json(incidents.iter().map(IncidentView::from).collect()))
}

#[derive(Deserialize)]
struct ReportQuery {
    from_us: i64,
    until_us: i64,
    /// The viewer's offset from UTC in minutes, east positive, for local times.
    offset_min: Option<i64>,
    /// Ticket details (ADR-0040), shown as given and never stored.
    account: Option<String>,
    service_address: Option<String>,
    contact: Option<String>,
    ticket: Option<String>,
    description: Option<String>,
}

/// A ticket detail as typed, trimmed and cut to `limit` characters. `None` if empty.
fn ticket_field(text: Option<&str>, limit: usize) -> Option<String> {
    let text = text?.trim();
    (!text.is_empty()).then(|| text.chars().take(limit).collect())
}

/// Longest stretch read from raw samples for one episode's evidence. Longer ones use
/// minute rollups, so one long outage can't make the report slow.
const RAW_EVIDENCE_LIMIT_US: i64 = 6 * 3600 * 1_000_000;

/// Rollups covering `from..until`, from the finest history that still exists there:
/// raw samples if the stretch is short and recent enough, otherwise stored minute
/// (or hour) rollups, completed with raw samples for the minutes not yet rolled up.
fn history_between(
    store: &Store,
    from: WallTime,
    until: WallTime,
    now: WallTime,
    only: Option<&[Role]>,
) -> Result<Vec<netwatch_history::Rollup>, netwatch_adapter_sqlite::StoreError> {
    use netwatch_adapter_sqlite::{MINUTE_RETENTION_DAYS, RAW_RETENTION_DAYS};
    use netwatch_history::{Tier, roll_up};
    let day = 24 * 3600 * 1_000_000_i64;
    let raw_since = now.as_unix_micros() - RAW_RETENTION_DAYS * day;
    if from.as_unix_micros() >= raw_since
        && until.as_unix_micros() - from.as_unix_micros() <= RAW_EVIDENCE_LIMIT_US
    {
        return Ok(roll_up(
            &store.samples_between(from, until, only)?,
            Tier::Minute,
        ));
    }
    let tier = if from.as_unix_micros() >= now.as_unix_micros() - MINUTE_RETENTION_DAYS * day {
        Tier::Minute
    } else {
        Tier::Hour
    };
    let rolled = store.rollup_progress(tier)?.unwrap_or(from).min(until);
    let mut rollups = store.rollups_between(tier, from, rolled, only)?;
    rollups.extend(roll_up(
        &store.samples_between(rolled.max(from), until, only)?,
        tier,
    ));
    Ok(rollups)
}

/// How measurements are taken, for the report (ADR-0009, ADR-0030, ADR-0031, ADR-0034).
const METHOD: &[&str] = &[
    "Probes: ICMP echo to each ICMP target every 5 s (2 s timeout); hops 1 to 3 toward each, by TTL-limited \
     echo every 1 s (0.9 s timeout), hop 1 shared by every target; a DNS query (. IN NS) to each DNS target \
     every 5 s; an HTTPS GET to each HTTPS target every 10 s (5 s timeout), checked against its expected \
     status. All probes are pinned to the egress interface.",
    "A role is degraded when any of its last 12 probes was lost, and failing (an outage) when half or more \
     were. Loss at a hop that doesn't carry through to the later hops or the target is the router \
     deprioritizing ICMP, not loss, and is never an incident.",
    "An incident's probable cause is the most upstream role whose own incident began first (or within 10 s). \
     An episode is incidents linked by probable cause. Beyond the gateway means it began at hop 2 or further.",
    "Hop identities are the addresses of the routers that sent ICMP time-exceeded. Router names are their \
     PTR records and the origin AS of their prefix (from Team Cymru). Times are UTC, from this host's clock.",
    "Path traces send 3 rounds of TTL-limited ICMP echo, 1 s apart (2 s timeout), at TTLs 4 to 30 toward each \
     ICMP and DNS target: a routine trace every 6 hours, and an episode trace when an incident opens at hop 2 \
     or beyond, at most once per 10 minutes per target. Hops that don't answer are routers that drop or \
     deprioritize ICMP, which is not loss.",
    "Jitter is the mean absolute difference between consecutive answered round trips of one role. A loss run \
     is a sequence of consecutive lost probes. Link load is the NIC's own traffic, sampled every second.",
];

/// The longest episode timeline, before steps grow from a minute to 10 minutes.
const MINUTE_STEPS_UP_TO_US: i64 = 2 * 3600 * 1_000_000;
const HOUR_US: i64 = 3600 * 1_000_000;
const DAY_US: i64 = 24 * HOUR_US;

fn wall_now() -> WallTime {
    WallTime::from_unix_micros(
        i64::try_from(
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap_or_default()
                .as_micros(),
        )
        .unwrap_or(i64::MAX),
    )
}

fn event_kind_text(kind: &str) -> &str {
    match kind {
        "epoch-started" => "identity changed",
        "started" => "netwatch started",
        "epoch-ended" => "identity lost",
        "desynchronized" => "topology resync started",
        "resynchronized" => "topology resynchronized",
        "kernel-log" => "kernel log",
        "source-disabled" => "source disabled",
        "clock-jump" => "clock jump",
        "config-drift" => "sysctl changed",
        "radio" => "radio changed",
        "public-address" => "public address",
        "trace" => "path trace",
        other => other,
    }
}

/// Everything the report says about the connection, from current state (ADR-0039).
fn connection(
    state: &AppState,
    traces: &[netwatch_adapter_sqlite::StoredTrace],
) -> Vec<(String, String)> {
    let current = state.current.borrow().clone();
    let binding = |key: &str| current.bindings.get(key).map(|b| b.identity.clone());
    let named = |address: &String| match current.names.get(address) {
        Some(name) => format!("{address} ({name})"),
        None => address.clone(),
    };
    let mut facts: Vec<(String, Option<String>)> = vec![
        ("NIC".to_owned(), binding("nic")),
        ("Link".to_owned(), current.host.link.clone()),
        ("Tunnel".to_owned(), binding("tunnel")),
        ("Local address".to_owned(), binding("local-interface")),
    ];
    // Hops by distance and family, each router once.
    for ttl in 1..=3 {
        for (family, v6) in [("IPv4", false), ("IPv6", true)] {
            let mut routers: Vec<String> = current
                .bindings
                .iter()
                .filter(|(key, _)| key.starts_with("hop:") && key.ends_with(&format!(":{ttl}")))
                .map(|(_, b)| b.identity.clone())
                .filter(|a| a.contains(':') == v6)
                .collect();
            routers.sort();
            routers.dedup();
            let what = if ttl == 1 {
                format!("{family} gateway")
            } else {
                format!("{family} hop {ttl}")
            };
            let routers: Vec<String> = routers.iter().map(named).collect();
            facts.push((what, (!routers.is_empty()).then(|| routers.join(", "))));
        }
    }
    // Where the ISP's network most likely begins, and whether it puts this host
    // behind carrier-grade NAT (ADR-0040).
    let hops: Vec<(u8, std::net::IpAddr)> = current
        .bindings
        .iter()
        .filter_map(|(key, b)| match Role::from_key(key)? {
            Role::Hop { ttl, .. } => Some((ttl.get().get(), b.identity.parse().ok()?)),
            _ => None,
        })
        .collect();
    let first_public = report::first_public_hop(&hops, traces)
        .map(|(ttl, address)| format!("hop {ttl}: {}", named(&address.to_string())));
    facts.push(("First public hop".to_owned(), first_public));
    let mut shared: Vec<(u8, std::net::IpAddr)> = hops
        .iter()
        .copied()
        .filter(|&(_, a)| netwatch_shared::address::is_shared(a))
        .collect();
    shared.sort_unstable();
    shared.dedup();
    facts.push((
        "Carrier-grade NAT".to_owned(),
        (!shared.is_empty()).then(|| {
            let at: Vec<String> = shared
                .iter()
                .map(|(ttl, a)| format!("hop {ttl} ({a})"))
                .collect();
            format!(
                "likely: {} in shared address space (RFC 6598)",
                at.join(", ")
            )
        }),
    ));
    facts.push(("Public address".to_owned(), current.host.public.clone()));
    let resolvers: Vec<String> = state
        .targets
        .iter()
        .filter(|t| t.name.starts_with("system resolver"))
        .map(|t| t.name.clone())
        .collect();
    facts.push((
        "System resolver".to_owned(),
        (!resolvers.is_empty()).then(|| resolvers.join(", ")),
    ));
    let layers = state.layers.borrow().clone();
    for (what, key) in [("Wi-Fi", "wifi"), ("Cellular", "cellular")] {
        facts.push((
            what.to_owned(),
            layers.get(key).and_then(|l| l.notes.first().cloned()),
        ));
    }
    facts
        .into_iter()
        .filter_map(|(k, v)| Some((k, v?)))
        .collect()
}

fn about(state: &AppState) -> Vec<(String, String)> {
    let host = state.current.borrow().host.clone();
    let clock = match (host.clock_synchronized, host.clock_error_us) {
        (Some(true), Some(error)) => Some(format!(
            "synchronized (estimated error {:.1} ms)",
            error as f64 / 1000.0
        )),
        (Some(true), None) => Some("synchronized".to_owned()),
        (Some(false), _) => Some("NOT synchronized: timestamps may be off".to_owned()),
        (None, _) => None,
    };
    [
        ("Host", host.hostname),
        ("Kernel", host.kernel),
        ("netwatch", Some(host.version)),
        ("Clock", clock),
    ]
    .into_iter()
    .filter_map(|(k, v)| Some((k.to_owned(), v?)))
    .collect()
}

async fn build_report(
    state: &AppState,
    query: ReportQuery,
) -> Result<(report::render::Report, BTreeMap<String, String>), (StatusCode, String)> {
    use report::analysis;
    if query.until_us <= query.from_us {
        return Err((
            StatusCode::BAD_REQUEST,
            "until_us must be after from_us".to_owned(),
        ));
    }
    let (from, until) = (
        WallTime::from_unix_micros(query.from_us),
        WallTime::from_unix_micros(query.until_us),
    );
    let now = wall_now();
    let raw_since = now.as_unix_micros() - netwatch_adapter_sqlite::RAW_RETENTION_DAYS * DAY_US;
    let span = until.as_unix_micros() - from.as_unix_micros();
    let (period, period_us) = if span <= 7 * DAY_US {
        ("Hour", HOUR_US)
    } else {
        ("Day", DAY_US)
    };
    let (episodes, summary, without_a_way_out_us, periods, events, normal_paths) =
        read(state, move |store| {
            use netwatch_probing::trace::TraceKind;
            let known: Vec<Role> = store.roles()?.into_iter().map(|(role, _)| role).collect();
            let identities = store.epoch_identities(
                WallTime::from_unix_micros(from.as_unix_micros() - HOUR_US),
                until,
            )?;
            let mut episodes = Vec::new();
            let incidents = store.incidents(from, until)?;
            let without_a_way_out_us = analysis::time_without_a_way_out(&incidents, from, until);
            for episode in report::episodes(&incidents) {
                let start = episode.started.max(from);
                let end = episode.ended.unwrap_or(until).min(until);
                let on_path = report::roles_on_paths(&episode.affected, &known);
                let evidence = history_between(store, start, end, now, Some(&on_path))?;
                // The hour before the episode itself began, which may be before the span.
                let before_from =
                    WallTime::from_unix_micros(episode.started.as_unix_micros() - HOUR_US);
                let before =
                    history_between(store, before_from, episode.started, now, Some(&on_path))?;
                // Of several epochs with one identity (a restart starts a new one),
                // the one with the most probes is the baseline: medians don't merge.
                let mut most: BTreeMap<(Role, Option<String>), (u64, f64)> = BTreeMap::new();
                for s in report::path_stats(&before, &identities) {
                    if let Some(median) = s.median_ms {
                        let best = most.entry((s.role, s.identity)).or_insert((s.sent, median));
                        if s.sent > best.0 {
                            *best = (s.sent, median);
                        }
                    }
                }
                let before = most
                    .into_iter()
                    .map(|(key, (_, median))| (key, median))
                    .collect();
                let raw = start.as_unix_micros() >= raw_since;
                let step_us =
                    if end.as_unix_micros() - start.as_unix_micros() <= MINUTE_STEPS_UP_TO_US {
                        60_000_000
                    } else {
                        600_000_000
                    };
                let raw_end = end.min(WallTime::from_unix_micros(
                    start.as_unix_micros() + RAW_EVIDENCE_LIMIT_US,
                ));
                let (reasons, timeline, patterns, load) = if raw {
                    let load = analysis::link_load(&store.readings_between(start, raw_end)?);
                    let samples = store.samples_between(start, raw_end, Some(&on_path))?;
                    let mut targets: Vec<Role> = episode
                        .affected
                        .iter()
                        .filter(|r| matches!(r, Role::Target(_)))
                        .cloned()
                        .collect();
                    if targets.is_empty() {
                        targets.clone_from(&episode.affected);
                    }
                    (
                        analysis::loss_reasons(&samples),
                        analysis::timeline(&samples, &episode.cause, &targets, start, step_us),
                        analysis::loss_patterns(&samples),
                        load,
                    )
                } else {
                    (BTreeMap::new(), Vec::new(), BTreeMap::new(), None)
                };
                // The traces of every target whose path the episode touched (ADR-0040):
                // the episode traces that ran during it, and the routine trace before it.
                let traced: Vec<Role> = episode
                    .affected
                    .iter()
                    .filter_map(|r| match r {
                        Role::Target(t) | Role::Hop { target: t, .. } => Some(Role::Target(*t)),
                        _ => None,
                    })
                    .collect::<std::collections::BTreeSet<_>>()
                    .into_iter()
                    .collect();
                let traces: Vec<_> = store
                    .traces(episode.started, end.max(episode.started))?
                    .into_iter()
                    .filter(|t| t.kind == TraceKind::Episode && traced.contains(&t.role))
                    .collect();
                // Each target's baseline is a routine trace of the path its episode
                // trace ran on: another path's would read as a change (ADR-0057).
                let mut baselines = BTreeMap::new();
                for role in &traced {
                    let path = traces
                        .iter()
                        .find(|t| &t.role == role)
                        .and_then(|t| t.path.as_deref());
                    if let Some(trace) =
                        store.latest_trace(role, TraceKind::Routine, episode.started, path)?
                    {
                        baselines.insert(role.clone(), trace);
                    }
                }
                episodes.push(report::render::EpisodeReport {
                    path: report::path_stats(&evidence, &identities),
                    episode,
                    before,
                    reasons,
                    timeline,
                    step_us,
                    raw,
                    patterns,
                    load,
                    traces,
                    baselines,
                });
            }
            let mut normal_paths = Vec::new();
            // The normal path is the routine trace of the path each target was last
            // traced on in the span, not of whatever network it was on before.
            for role in known.iter().filter(|r| matches!(r, Role::Target(_))) {
                let path = store.latest_path(role, until)?;
                normal_paths.extend(store.latest_trace(
                    role,
                    TraceKind::Routine,
                    until,
                    path.as_deref(),
                )?);
            }
            let history = history_between(store, from, until, now, None)?;
            let summary = report::path_stats(&history, &identities);
            let targets: Vec<&netwatch_history::Rollup> = history
                .iter()
                .filter(|r| matches!(r.role, Role::Target(_)))
                .collect();
            let periods = analysis::by_period(&targets, period_us);
            let events: Vec<report::render::EventLine> = store
                .events(from)?
                .into_iter()
                // A resumed epoch isn't a change (ADR-0044); the start says it all.
                .filter(|e| e.at < until && e.kind != "epoch-resumed")
                .map(|e| report::render::EventLine {
                    at: e.at,
                    kind: event_kind_text(&e.kind).to_owned(),
                    role: e.role.as_deref().and_then(Role::from_key),
                    detail: e.detail.unwrap_or_default(),
                })
                .collect();
            Ok((
                episodes,
                summary,
                without_a_way_out_us,
                periods,
                events,
                normal_paths,
            ))
        })
        .await?;
    let connection = connection(state, &normal_paths);
    let current = state.current.borrow().clone();
    let ticket = [
        ("Account number", query.account.as_deref(), 200),
        ("Service address", query.service_address.as_deref(), 400),
        ("Contact", query.contact.as_deref(), 400),
        ("ISP ticket", query.ticket.as_deref(), 200),
    ]
    .into_iter()
    .filter_map(|(k, v, limit)| Some((k.to_owned(), ticket_field(v, limit)?)))
    .collect();
    let names = state
        .targets
        .iter()
        .map(|t| (t.key.clone(), t.name.clone()))
        .collect();
    Ok((
        report::render::Report {
            from,
            until,
            generated: now,
            utc_offset_s: query.offset_min.unwrap_or(0).clamp(-24 * 60, 24 * 60) * 60,
            about: about(state),
            ticket,
            description: ticket_field(query.description.as_deref(), 4000),
            names: current.names.clone(),
            link_speed_mbps: current.host.link_speed_mbps,
            normal_paths,
            connection,
            method: METHOD.iter().map(|m| (*m).to_owned()).collect(),
            episodes,
            summary,
            without_a_way_out_us,
            periods,
            period,
            events,
            csv_href: format!(
                "/report.csv?from_us={}&until_us={}",
                query.from_us, query.until_us
            ),
        },
        names,
    ))
}

async fn report_page(
    State(state): State<AppState>,
    Query(query): Query<ReportQuery>,
) -> Result<Response, (StatusCode, String)> {
    let (report, names) = build_report(&state, query).await?;
    let label = |role: &Role| report::label(role, &names);
    Ok((
        [(header::CONTENT_TYPE, "text/html; charset=utf-8")],
        report::render::html(&report, &label),
    )
        .into_response())
}

async fn report_text(
    State(state): State<AppState>,
    Query(query): Query<ReportQuery>,
) -> Result<Response, (StatusCode, String)> {
    let (report, names) = build_report(&state, query).await?;
    let label = |role: &Role| report::label(role, &names);
    Ok((
        [(header::CONTENT_TYPE, "text/plain; charset=utf-8")],
        report::render::text(&report, &label),
    )
        .into_response())
}

/// How much before and after each episode the CSV includes.
const CSV_MARGIN_US: i64 = 2 * 60 * 1_000_000;

/// The raw samples around every episode in the span, as CSV (ADR-0039).
async fn report_csv(
    State(state): State<AppState>,
    Query(query): Query<ReportQuery>,
) -> Result<Response, (StatusCode, String)> {
    let (from, until) = (
        WallTime::from_unix_micros(query.from_us),
        WallTime::from_unix_micros(query.until_us),
    );
    let (samples, identities) = read(&state, move |store| {
        // Each episode's window, and only the roles on its affected paths, as in the
        // report's path tables.
        let known: Vec<Role> = store.roles()?.into_iter().map(|(role, _)| role).collect();
        let mut samples = Vec::new();
        for e in report::episodes(&store.incidents(from, until)?) {
            let end = e.ended.unwrap_or(until).min(until).as_unix_micros();
            let start = e.started.max(from).as_unix_micros() - CSV_MARGIN_US;
            let end = (end + CSV_MARGIN_US).min(start + RAW_EVIDENCE_LIMIT_US);
            let on_path = report::roles_on_paths(&e.affected, &known);
            samples.extend(store.samples_between(
                WallTime::from_unix_micros(start),
                WallTime::from_unix_micros(end),
                Some(&on_path),
            )?);
        }
        // Overlapping windows read some samples twice.
        samples.sort_by(|a, b| (a.at, &a.role).cmp(&(b.at, &b.role)));
        samples.dedup_by(|a, b| a.at == b.at && a.role == b.role);
        let identities = store.epoch_identities(from, until)?;
        Ok((samples, identities))
    })
    .await?;
    let names: BTreeMap<String, String> = state
        .targets
        .iter()
        .map(|t| (t.key.clone(), t.name.clone()))
        .collect();
    let label = |role: &Role| report::label(role, &names);
    Ok((
        [
            (header::CONTENT_TYPE, "text/csv; charset=utf-8"),
            (
                header::CONTENT_DISPOSITION,
                "attachment; filename=\"netwatch-samples.csv\"",
            ),
        ],
        report::analysis::csv(&samples, &identities, &label),
    )
        .into_response())
}

async fn config(State(state): State<AppState>) -> Json<ConfigView> {
    Json(state.settings.view())
}

/// Whether a write request came from the UI itself, not from another web page the
/// browser is showing (ADR-0035): a cross-site page can't set `X-Netwatch` without a
/// CORS preflight, which is never answered, and the request must be [`same_site`].
pub fn from_the_ui(headers: &HeaderMap) -> bool {
    let marked = headers
        .get("x-netwatch")
        .is_some_and(|v| v.as_bytes() == b"1");
    let json = headers
        .get(header::CONTENT_TYPE)
        .and_then(|v| v.to_str().ok())
        .is_some_and(|t| t.starts_with("application/json"));
    marked && json && same_site(headers)
}

/// Whether a request may be answered at all (ADR-0050): its `Host` is an IP literal
/// or `localhost`, which a DNS-rebound name is not, and its `Origin`, if it has one,
/// is that host, which another web page's is not. WebSockets aren't covered by CORS,
/// so without this any page could read the live stream.
pub fn same_site(headers: &HeaderMap) -> bool {
    let text = |name: header::HeaderName| headers.get(name).and_then(|v| v.to_str().ok());
    let host = text(header::HOST).unwrap_or("");
    let hostname = host
        .rsplit_once(':')
        .filter(|(_, port)| port.bytes().all(|b| b.is_ascii_digit()))
        .map_or(host, |(name, _)| name);
    let literal = hostname == "localhost"
        || hostname.parse::<std::net::IpAddr>().is_ok()
        || hostname
            .strip_prefix('[')
            .and_then(|h| h.strip_suffix(']'))
            .is_some_and(|h| h.parse::<std::net::Ipv6Addr>().is_ok());
    let same_origin = text(header::ORIGIN).is_none_or(|origin| {
        origin
            .split_once("://")
            .is_some_and(|(_, authority)| authority == host)
    });
    literal && same_origin
}

async fn guard_site(request: axum::extract::Request, next: axum::middleware::Next) -> Response {
    if same_site(request.headers()) {
        next.run(request).await
    } else {
        (
            StatusCode::FORBIDDEN,
            "netwatch answers only its own UI, by an IP address or localhost",
        )
            .into_response()
    }
}

async fn save_config(
    State(state): State<AppState>,
    headers: HeaderMap,
    body: axum::body::Bytes,
) -> Result<StatusCode, (StatusCode, String)> {
    if !from_the_ui(&headers) {
        return Err((
            StatusCode::FORBIDDEN,
            "settings can only be saved from the netwatch UI".to_owned(),
        ));
    }
    let targets: Vec<TargetSpec> = serde_json::from_slice(&body).map_err(|e| {
        (
            StatusCode::BAD_REQUEST,
            format!("not a list of targets: {e}"),
        )
    })?;
    let settings = state.settings.clone();
    tokio::task::spawn_blocking(move || settings.save(targets))
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?
        .map_err(|message| (StatusCode::UNPROCESSABLE_ENTITY, message))?;
    Ok(StatusCode::NO_CONTENT)
}

async fn delete_history(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> Result<StatusCode, (StatusCode, String)> {
    if !from_the_ui(&headers) {
        return Err((
            StatusCode::FORBIDDEN,
            "history can only be flushed from the netwatch UI".to_owned(),
        ));
    }
    (state.flush_history)()
        .await
        .map_err(|_| {
            (
                StatusCode::SERVICE_UNAVAILABLE,
                "the writer has stopped".to_owned(),
            )
        })?
        .map_err(|message| (StatusCode::INTERNAL_SERVER_ERROR, message))?;
    // Every open UI drops what it holds and backfills from the empty store.
    let _ = state.live.send(LiveMessage::Cleared);
    Ok(StatusCode::NO_CONTENT)
}

async fn target_list(State(state): State<AppState>) -> Json<Vec<TargetView>> {
    Json(state.targets.as_ref().clone())
}

async fn statuses(State(state): State<AppState>) -> Json<BTreeMap<String, String>> {
    Json(state.current.borrow().statuses.clone())
}

async fn host(State(state): State<AppState>) -> Json<HostView> {
    Json(state.current.borrow().host.clone())
}

async fn bindings(State(state): State<AppState>) -> Json<BTreeMap<String, BindingView>> {
    Json(state.current.borrow().bindings.clone())
}

async fn network_map(State(state): State<AppState>) -> Json<MapView> {
    Json(state.current.borrow().map.clone())
}

async fn shared_rows(State(state): State<AppState>) -> Json<Vec<RowView>> {
    Json(state.current.borrow().rows.clone())
}

async fn layer_reports(State(state): State<AppState>) -> Json<BTreeMap<String, LayerReportView>> {
    Json(state.layers.borrow().clone())
}

async fn live_socket(State(state): State<AppState>, upgrade: WebSocketUpgrade) -> Response {
    let receiver = state.live.subscribe();
    upgrade.on_upgrade(move |socket| forward(socket, receiver))
}

/// Forwards live messages until the client goes away. A client that falls behind
/// is told so by closing the socket, and it reconnects and backfills.
async fn forward(mut socket: WebSocket, mut live: broadcast::Receiver<LiveMessage>) {
    loop {
        let message = match live.recv().await {
            Ok(message) => message,
            Err(broadcast::error::RecvError::Lagged(_) | broadcast::error::RecvError::Closed) => {
                let _ = socket.send(Message::Close(None)).await;
                return;
            }
        };
        let Ok(text) = serde_json::to_string(&message) else {
            continue;
        };
        if socket.send(Message::Text(text.into())).await.is_err() {
            return;
        }
    }
}

async fn spa(uri: Uri) -> Response {
    let path = uri.path().trim_start_matches('/');
    let file = Spa::get(path).or_else(|| Spa::get("index.html"));
    match file {
        Some(file) => (
            [(header::CONTENT_TYPE, file.metadata.mimetype().to_owned())],
            file.data,
        )
            .into_response(),
        None => (
            StatusCode::NOT_FOUND,
            "the UI hasn't been built: run `npm ci && npm run build` in web/",
        )
            .into_response(),
    }
}
