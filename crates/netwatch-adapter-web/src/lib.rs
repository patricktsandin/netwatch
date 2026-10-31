//! The open host service: the REST and WebSocket API, plus the embedded SPA
//! (ADR-0005, ADR-0019).
//!
//! - `GET /api/roles`: every role that has samples
//! - `GET /api/samples?role=<key>&since_us=<µs>`: one role's samples, oldest first
//! - `GET /api/events?since_us=<µs>`: topology events, oldest first
//! - `GET /api/live`: a WebSocket of [`LiveMessage`]s
//! - everything else: the SPA from `web/dist`, embedded in release builds
//!
//! Reads open their own SQLite connection, which WAL mode lets run alongside the
//! single writer.

pub mod api;

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
use tokio::sync::broadcast;

pub use api::{EventPoint, LiveMessage, RoleSummary, SamplePoint};

#[derive(rust_embed::RustEmbed)]
#[folder = "$CARGO_MANIFEST_DIR/../../web/dist"]
#[allow_missing = true]
struct Spa;

#[derive(Clone)]
struct AppState {
    database: PathBuf,
    live: broadcast::Sender<LiveMessage>,
}

/// Serves the API and the SPA on `address` until the process ends.
pub async fn serve(
    address: SocketAddr,
    database: PathBuf,
    live: broadcast::Sender<LiveMessage>,
) -> std::io::Result<()> {
    let app = Router::new()
        .route("/api/roles", get(roles))
        .route("/api/samples", get(samples))
        .route("/api/events", get(events))
        .route("/api/live", get(live_socket))
        .fallback(spa)
        .with_state(AppState { database, live });
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
struct EventsQuery {
    since_us: i64,
}

/// Runs a read against a fresh connection on the blocking pool.
async fn read<T: Send + 'static>(
    state: &AppState,
    query: impl FnOnce(&Store) -> Result<T, netwatch_adapter_sqlite::StoreError> + Send + 'static,
) -> Result<T, (StatusCode, String)> {
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

async fn events(
    State(state): State<AppState>,
    Query(query): Query<EventsQuery>,
) -> Result<Json<Vec<EventPoint>>, (StatusCode, String)> {
    let since = WallTime::from_unix_micros(query.since_us);
    let events = read(&state, move |store| store.events(since)).await?;
    Ok(Json(events.into_iter().map(EventPoint::from).collect()))
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
