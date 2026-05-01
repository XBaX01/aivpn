//! Management HTTP API over Unix socket.
//!
//! Enabled via `--features management-api`. Binds to a Unix domain socket
//! (default `/run/aivpn/api.sock`) and exposes a REST API for managing clients
//! and triggering live reloads without restarting the server.
//!
//! Unix-only: Unix domain sockets are not available on Windows.
#![cfg(unix)]

use std::sync::Arc;
use std::time::Instant;

use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::response::IntoResponse;
use axum::routing::{get, post};
use axum::{Json, Router};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use hyper_util::rt::TokioIo;
use tokio::net::UnixListener;
use tower::util::ServiceExt;

use crate::client_db::{ClientDatabase, ClientStats};

// ── State ────────────────────────────────────────────────────────────────────

#[derive(Clone)]
struct ApiState {
    db: Arc<ClientDatabase>,
    started_at: Instant,
}

// ── Wire types (PSK is never included) ───────────────────────────────────────

#[derive(Serialize)]
struct ClientResponse {
    id: String,
    name: String,
    vpn_ip: String,
    enabled: bool,
    created_at: DateTime<Utc>,
    stats: ClientStats,
}

impl From<crate::client_db::ClientConfig> for ClientResponse {
    fn from(c: crate::client_db::ClientConfig) -> Self {
        Self {
            id: c.id,
            name: c.name,
            vpn_ip: c.vpn_ip.to_string(),
            enabled: c.enabled,
            created_at: c.created_at,
            stats: c.stats,
        }
    }
}

#[derive(Deserialize)]
struct AddClientRequest {
    name: String,
}

#[derive(Serialize)]
struct StatusResponse {
    version: &'static str,
    uptime_secs: u64,
}

#[derive(Serialize)]
struct ErrorResponse {
    error: String,
}

fn err(msg: impl ToString) -> Json<ErrorResponse> {
    Json(ErrorResponse { error: msg.to_string() })
}

// ── Handlers ─────────────────────────────────────────────────────────────────

async fn get_status(State(state): State<ApiState>) -> impl IntoResponse {
    Json(StatusResponse {
        version: env!("CARGO_PKG_VERSION"),
        uptime_secs: state.started_at.elapsed().as_secs(),
    })
}

async fn list_clients(State(state): State<ApiState>) -> impl IntoResponse {
    let clients: Vec<ClientResponse> = state.db.list_clients().into_iter().map(Into::into).collect();
    Json(clients)
}

async fn add_client(
    State(state): State<ApiState>,
    Json(body): Json<AddClientRequest>,
) -> impl IntoResponse {
    let db = state.db.clone();
    let name = body.name.clone();
    match tokio::task::spawn_blocking(move || db.add_client(&name)).await {
        Ok(Ok(client)) => (StatusCode::CREATED, Json(ClientResponse::from(client))).into_response(),
        Ok(Err(e)) => (StatusCode::CONFLICT, err(e)).into_response(),
        Err(_) => (StatusCode::INTERNAL_SERVER_ERROR, err("internal error")).into_response(),
    }
}

async fn get_client(
    State(state): State<ApiState>,
    Path(id): Path<String>,
) -> impl IntoResponse {
    match state.db.find_by_id(&id) {
        Some(client) => Json(ClientResponse::from(client)).into_response(),
        None => (StatusCode::NOT_FOUND, err(format!("Client '{}' not found", id))).into_response(),
    }
}

async fn remove_client(
    State(state): State<ApiState>,
    Path(id): Path<String>,
) -> impl IntoResponse {
    let db = state.db.clone();
    match tokio::task::spawn_blocking(move || db.remove_client(&id)).await {
        Ok(Ok(())) => StatusCode::NO_CONTENT.into_response(),
        Ok(Err(e)) => (StatusCode::NOT_FOUND, err(e)).into_response(),
        Err(_) => (StatusCode::INTERNAL_SERVER_ERROR, err("internal error")).into_response(),
    }
}

async fn reload(State(state): State<ApiState>) -> impl IntoResponse {
    let db = state.db.clone();
    match tokio::task::spawn_blocking(move || db.reload_if_changed()).await {
        Ok(changed) => Json(serde_json::json!({ "reloaded": changed })).into_response(),
        Err(_) => (StatusCode::INTERNAL_SERVER_ERROR, err("internal error")).into_response(),
    }
}

// ── Router ───────────────────────────────────────────────────────────────────

fn router(state: ApiState) -> Router {
    Router::new()
        .route("/api/v1/status", get(get_status))
        .route("/api/v1/clients", get(list_clients).post(add_client))
        .route("/api/v1/clients/:id", get(get_client).delete(remove_client))
        .route("/api/v1/reload", post(reload))
        .with_state(state)
}

// ── Entry point ───────────────────────────────────────────────────────────────

/// Start the management API server on the given Unix socket path.
/// If `socket_path` is `None`, the server is not started.
/// Errors are logged but do not affect the main gateway.
pub async fn serve(db: Option<Arc<ClientDatabase>>, socket_path: Option<String>) {
    let Some(db) = db else {
        tracing::info!("Management API: no client database configured, skipping");
        return;
    };
    let Some(path) = socket_path else {
        tracing::info!("Management API: no socket path configured, skipping");
        return;
    };

    // Remove stale socket from a previous run
    if let Err(e) = std::fs::remove_file(&path) {
        if e.kind() != std::io::ErrorKind::NotFound {
            tracing::warn!("Management API: could not remove existing socket '{}': {}", path, e);
        }
    }

    // Ensure parent directory exists
    if let Some(parent) = std::path::Path::new(&path).parent() {
        if let Err(e) = std::fs::create_dir_all(parent) {
            tracing::warn!("Management API: cannot create socket directory '{}': {}", parent.display(), e);
            return;
        }
    }

    let listener = match UnixListener::bind(&path) {
        Ok(l) => l,
        Err(e) => {
            tracing::warn!("Management API: failed to bind '{}': {}", path, e);
            return;
        }
    };

    // Restrict socket to owner only (prevents other local users from calling the API)
    if let Err(e) = std::fs::set_permissions(&path, std::os::unix::fs::PermissionsExt::from_mode(0o600)) {
        tracing::warn!("Management API: failed to set socket permissions: {}", e);
    }

    tracing::info!("Management API listening on unix:{}", path);

    let state = ApiState {
        db,
        started_at: Instant::now(),
    };
    let app = router(state);

    loop {
        let (stream, _) = match listener.accept().await {
            Ok(s) => s,
            Err(e) => {
                tracing::warn!("Management API: accept error: {}", e);
                continue;
            }
        };
        let svc = app.clone();
        tokio::spawn(async move {
            let io = TokioIo::new(stream);
            let hyper_svc = hyper::service::service_fn(move |req| svc.clone().oneshot(req));
            if let Err(e) = hyper::server::conn::http1::Builder::new()
                .serve_connection(io, hyper_svc)
                .await
            {
                tracing::debug!("Management API: connection error: {}", e);
            }
        });
    }
}
