//! The open host service: the REST and WebSocket API, plus the embedded SPA
//! (ADR-0005, ADR-0019).
//!
//! - `GET /api/roles`: every role that has samples
//! - `GET /api/samples?role=<key>&since_us=<µs>`: one role's samples, oldest first
//! - `GET /api/rollups?role=<key>&tier=minute|hour&since_us=<µs>`: one role's rollups
//! - `GET /api/events?since_us=<µs>`: topology events, oldest first
//! - `GET /api/targets`: every configured target's name
//! - `GET /api/layers`: each layer's latest health report, by role key
//! - `GET /api/live`: a WebSocket of [`LiveMessage`]s
//! - everything else: the SPA from `web/dist`, embedded in release builds
//!
//! Reads open their own SQLite connection, which WAL mode lets run alongside the
//! single writer. Every read first asks the writer to flush, so a backfill never
//! misses a sample that the live stream already sent (ADR-0029,
//! `specs/live_backfill.qnt`).

pub mod api;

use std::collections::BTreeMap;
use std::net::SocketAddr;
use std::path::PathBuf;

use axum::extract::ws::{Message, WebSocket, WebSocketUpgrade};
use axum::extract::{Query, State};
use axum::http::{StatusCode, Uri, header};
use axum::response::{IntoResponse, Response};
use axum::routing::get;
use axum::{Json, Router};
use netwatch_adapter_sqlite::Store;
use netwatch_shared::{Role, WallTime};
use serde::Deserialize;
use tokio::sync::{broadcast, watch};

pub use api::{
    EventPoint, LayerReportView, LiveMessage, RoleSummary, RollupPoint, SamplePoint, TargetView,
};

#[derive(rust_embed::RustEmbed)]
#[folder = "../../web/dist"]
#[allow_missing = true]
struct Spa;

/// Asks the writer to write everything it has received, and resolves once it has.
pub type Flush = std::sync::Arc<dyn Fn() -> tokio::sync::oneshot::Receiver<()> + Send + Sync>;

#[derive(Clone)]
struct AppState {
    flush: Flush,
    database: PathBuf,
    live: broadcast::Sender<LiveMessage>,
    layers: watch::Receiver<BTreeMap<String, LayerReportView>>,
    targets: std::sync::Arc<Vec<TargetView>>,
}

/// Serves the API and the SPA on `address` until the process ends.
pub async fn serve(
    address: SocketAddr,
    database: PathBuf,
    live: broadcast::Sender<LiveMessage>,
    layers: watch::Receiver<BTreeMap<String, LayerReportView>>,
    targets: Vec<TargetView>,
    flush: Flush,
) -> std::io::Result<()> {
    let app = Router::new()
        .route("/api/roles", get(roles))
        .route("/api/samples", get(samples))
        .route("/api/rollups", get(rollups))
        .route("/api/events", get(events))
        .route("/api/targets", get(target_list))
        .route("/api/layers", get(layer_reports))
        .route("/api/live", get(live_socket))
        .fallback(spa)
        .with_state(AppState {
            flush,
            database,
            live,
            layers,
            targets: std::sync::Arc::new(targets),
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

async fn events(
    State(state): State<AppState>,
    Query(query): Query<EventsQuery>,
) -> Result<Json<Vec<EventPoint>>, (StatusCode, String)> {
    let since = WallTime::from_unix_micros(query.since_us);
    let events = read(&state, move |store| store.events(since)).await?;
    Ok(Json(events.into_iter().map(EventPoint::from).collect()))
}

async fn target_list(State(state): State<AppState>) -> Json<Vec<TargetView>> {
    Json(state.targets.as_ref().clone())
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
