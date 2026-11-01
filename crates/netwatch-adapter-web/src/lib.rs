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
//! - `GET /api/bindings`: every role's current identity and epoch, from Topology
//! - `GET /api/config`: the targets as the settings page edits them
//! - `PUT /api/config`: replaces the targets, then the daemon restarts (ADR-0035)
//! - `GET /api/live`: a WebSocket of [`LiveMessage`]s
//! - `GET /report?from_us=<µs>&until_us=<µs>`: an incident report as a printable page,
//!   and `GET /report.txt?…` as plain text (ADR-0033)
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
    BindingView, EventPoint, GaugeView, IncidentView, LayerReportView, LiveMessage, ReadingPoint,
    ReadingRollupPoint, RoleSummary, RollupPoint, SamplePoint, TargetView, status_text,
};
pub use api::{ConfigView, TargetSpec};

#[derive(rust_embed::RustEmbed)]
#[folder = "../../web/dist"]
#[allow_missing = true]
struct Spa;

/// Asks the writer to write everything it has received, and resolves once it has.
pub type Flush = std::sync::Arc<dyn Fn() -> tokio::sync::oneshot::Receiver<()> + Send + Sync>;

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
    live: broadcast::Sender<LiveMessage>,
    layers: watch::Receiver<BTreeMap<String, LayerReportView>>,
    targets: std::sync::Arc<Vec<TargetView>>,
    settings: std::sync::Arc<dyn Settings>,
}

/// What the daemon gives the UI server: where history is, and its live views.
pub struct Daemon {
    /// The SQLite database, which each read opens on its own.
    pub database: PathBuf,
    /// Live messages, broadcast to every WebSocket.
    pub live: broadcast::Sender<LiveMessage>,
    /// Each layer's latest health report.
    pub layers: watch::Receiver<BTreeMap<String, LayerReportView>>,
    /// Every configured target's name.
    pub targets: Vec<TargetView>,
    /// The flush barrier (ADR-0029).
    pub flush: Flush,
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
        .route("/api/config", get(config).put(save_config))
        .route("/api/live", get(live_socket))
        .route("/report", get(report_page))
        .route("/report.txt", get(report_text))
        .fallback(spa)
        .with_state(AppState {
            flush: daemon.flush,
            current: daemon.current,
            database: daemon.database,
            live: daemon.live,
            layers: daemon.layers,
            targets: std::sync::Arc::new(daemon.targets),
            settings: daemon.settings,
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

/// Runs a read against a fresh connection on the blocking pool.
async fn read<T: Send + 'static>(
    state: &AppState,
    query: impl FnOnce(&Store) -> Result<T, netwatch_adapter_sqlite::StoreError> + Send + 'static,
) -> Result<T, (StatusCode, String)> {
    // The flush barrier: if the writer has gone away, there's nothing left to wait
    // for, so the read goes ahead.
    let _ = (state.flush)().await;
    let database = state.database.clone();
    tokio::task::spawn_blocking(move || {
        let store = Store::open(&database)?;
        query(&store)
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
    let role = Role::from_key(&query.role).ok_or_else(|| {
        (
            StatusCode::BAD_REQUEST,
            format!("unknown role {:?}", query.role),
        )
    })?;
    let since = WallTime::from_unix_micros(query.since_us);
    let samples = read(&state, move |store| store.samples(&role, since)).await?;
    Ok(Json(samples.iter().map(SamplePoint::from).collect()))
}

async fn rollups(
    State(state): State<AppState>,
    Query(query): Query<RollupsQuery>,
) -> Result<Json<Vec<RollupPoint>>, (StatusCode, String)> {
    let role = Role::from_key(&query.role).ok_or_else(|| {
        (
            StatusCode::BAD_REQUEST,
            format!("unknown role {:?}", query.role),
        )
    })?;
    let tier = match query.tier.as_str() {
        "minute" => netwatch_history::Tier::Minute,
        "hour" => netwatch_history::Tier::Hour,
        other => return Err((StatusCode::BAD_REQUEST, format!("unknown tier {other:?}"))),
    };
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
) -> Result<Vec<netwatch_history::Rollup>, netwatch_adapter_sqlite::StoreError> {
    use netwatch_adapter_sqlite::{MINUTE_RETENTION_DAYS, RAW_RETENTION_DAYS};
    use netwatch_history::{Tier, roll_up};
    let day = 24 * 3600 * 1_000_000_i64;
    let raw_since = now.as_unix_micros() - RAW_RETENTION_DAYS * day;
    if from.as_unix_micros() >= raw_since
        && until.as_unix_micros() - from.as_unix_micros() <= RAW_EVIDENCE_LIMIT_US
    {
        return Ok(roll_up(&store.samples_between(from, until)?, Tier::Minute));
    }
    let tier = if from.as_unix_micros() >= now.as_unix_micros() - MINUTE_RETENTION_DAYS * day {
        Tier::Minute
    } else {
        Tier::Hour
    };
    let rolled = store.rollup_progress(tier)?.unwrap_or(from).min(until);
    let mut rollups = store.rollups_between(tier, from, rolled)?;
    rollups.extend(roll_up(
        &store.samples_between(rolled.max(from), until)?,
        tier,
    ));
    Ok(rollups)
}

async fn build_report(
    state: &AppState,
    query: ReportQuery,
) -> Result<(report::render::Report, BTreeMap<String, String>), (StatusCode, String)> {
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
    let now = WallTime::from_unix_micros(
        i64::try_from(
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap_or_default()
                .as_micros(),
        )
        .unwrap_or(i64::MAX),
    );
    let (episodes, summary) = read(state, move |store| {
        let known: Vec<Role> = store.roles()?.into_iter().map(|(role, _)| role).collect();
        let identities = store.epoch_identities(from, until)?;
        let episodes = report::episodes(&store.incidents(from, until)?)
            .into_iter()
            .map(|episode| {
                let end = episode.ended.unwrap_or(until).min(until);
                let on_path = report::roles_on_paths(&episode.affected, &known);
                let evidence: Vec<_> = history_between(store, episode.started.max(from), end, now)?
                    .into_iter()
                    .filter(|r| on_path.contains(&r.role))
                    .collect();
                Ok((episode, report::path_stats(&evidence, &identities)))
            })
            .collect::<Result<Vec<_>, netwatch_adapter_sqlite::StoreError>>()?;
        let summary = report::path_stats(&history_between(store, from, until, now)?, &identities);
        Ok((episodes, summary))
    })
    .await?;
    let current = state.current.borrow().clone();
    let identity = |key: &str| current.bindings.get(key).map(|b| b.identity.clone());
    let gateway = current
        .bindings
        .iter()
        .find(|(key, _)| key.starts_with("hop:") && key.ends_with(":1"))
        .map(|(_, b)| b.identity.clone());
    let host = [
        ("NIC", identity("nic")),
        ("Tunnel", identity("tunnel")),
        ("Address", identity("local-interface")),
        ("Gateway", gateway),
    ]
    .into_iter()
    .filter_map(|(what, id)| id.map(|id| (what.to_owned(), id)))
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
            host,
            episodes,
            summary,
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

async fn config(State(state): State<AppState>) -> Json<ConfigView> {
    Json(state.settings.view())
}

/// Whether a write request came from the UI itself, not from another web page the
/// browser is showing (ADR-0035): a cross-site page can't set `X-Netwatch` without a
/// CORS preflight, which is never answered, and a DNS-rebound name fails the `Host`
/// check.
pub fn from_the_ui(headers: &HeaderMap) -> bool {
    let text = |name: header::HeaderName| headers.get(name).and_then(|v| v.to_str().ok());
    let marked = headers
        .get("x-netwatch")
        .is_some_and(|v| v.as_bytes() == b"1");
    let json = text(header::CONTENT_TYPE).is_some_and(|t| t.starts_with("application/json"));
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
    marked && json && literal && same_origin
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

async fn target_list(State(state): State<AppState>) -> Json<Vec<TargetView>> {
    Json(state.targets.as_ref().clone())
}

async fn statuses(State(state): State<AppState>) -> Json<BTreeMap<String, String>> {
    Json(state.current.borrow().statuses.clone())
}

async fn bindings(State(state): State<AppState>) -> Json<BTreeMap<String, BindingView>> {
    Json(state.current.borrow().bindings.clone())
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
