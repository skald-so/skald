//! Skald server.

use std::io;
use std::time::Duration;

use axum::extract::State;
use axum::extract::ws::{WebSocket, WebSocketUpgrade};
use axum::http::{StatusCode, header};
use axum::response::{IntoResponse, Response};
use axum::routing::get;
use axum::{Extension, Json, Router, middleware};
use serde::Serialize;
use sqlx::migrate::Migrator;
use sqlx::postgres::{PgConnectOptions, PgPoolOptions};
use sqlx::{Connection, PgConnection, PgPool};

mod protocol;

pub use protocol::Versions;
use protocol::{Stated, Statement};

/// The OpenAPI spec the server implements: `api/openapi.json`, served at `GET /openapi.json`.
pub const OPENAPI_SPEC: &str = include_str!("../../../api/openapi.json");

/// The schema migrations in `migrations/`, applied at start.
pub static MIGRATOR: Migrator = sqlx::migrate!();

/// How long to wait for a PostgreSQL connection, and for PostgreSQL to answer the health check.
const DATABASE_TIMEOUT: Duration = Duration::from_secs(5);

/// Connects to PostgreSQL at `url`; fails if no connection opens within [`DATABASE_TIMEOUT`].
pub async fn connect(url: &str) -> Result<PgPool, sqlx::Error> {
    let options: PgConnectOptions = url.parse()?;
    // The pool reports every connection failure as a timeout; a direct connection reports the cause.
    let conn = tokio::time::timeout(DATABASE_TIMEOUT, PgConnection::connect_with(&options))
        .await
        .map_err(|_| {
            sqlx::Error::Io(io::Error::new(
                io::ErrorKind::TimedOut,
                format!("no connection within {DATABASE_TIMEOUT:?}"),
            ))
        })??;
    conn.close().await?;
    Ok(PgPoolOptions::new()
        .acquire_timeout(DATABASE_TIMEOUT)
        .connect_lazy_with(options))
}

/// The state the handlers share.
#[derive(Clone)]
struct AppState {
    db: PgPool,
    versions: Versions,
}

/// The HTTP API described by [`OPENAPI_SPEC`], backed by the PostgreSQL pool `db`, accepting the
/// protocol versions `versions`.
pub fn app(db: PgPool, versions: Versions) -> Router {
    let checked =
        |statement| middleware::from_fn_with_state((versions, statement), protocol::check);
    Router::new()
        .route("/health", get(health))
        .route("/openapi.json", get(openapi_spec))
        .route(
            "/protocol",
            get(protocol_versions).layer(checked(Statement::Header)),
        )
        .route("/ws", get(websocket).layer(checked(Statement::Subprotocol)))
        .with_state(AppState { db, versions })
}

/// `Health` and `HealthUnavailable` in the spec.
#[derive(Serialize)]
struct Health {
    status: &'static str,
}

async fn health(State(state): State<AppState>) -> (StatusCode, Json<Health>) {
    let ping = sqlx::query("SELECT 1").execute(&state.db);
    match tokio::time::timeout(DATABASE_TIMEOUT, ping).await {
        Ok(Ok(_)) => (StatusCode::OK, Json(Health { status: "ok" })),
        Ok(Err(_)) | Err(_) => (
            StatusCode::SERVICE_UNAVAILABLE,
            Json(Health {
                status: "unavailable",
            }),
        ),
    }
}

async fn openapi_spec() -> impl IntoResponse {
    ([(header::CONTENT_TYPE, "application/json")], OPENAPI_SPEC)
}

/// `Protocol` in the spec.
#[derive(Serialize)]
struct Protocol {
    supported: Vec<u64>,
}

async fn protocol_versions(State(state): State<AppState>) -> Json<Protocol> {
    Json(Protocol {
        supported: state.versions.supported(),
    })
}

/// Opens a WebSocket with the subprotocol that states the accepted version.
async fn websocket(
    Extension(Stated(version)): Extension<Stated>,
    upgrade: WebSocketUpgrade,
) -> Response {
    upgrade
        .protocols([skald_proto::subprotocol(version)])
        .on_upgrade(hold)
}

/// Keeps a WebSocket open, answering pings, until the client closes it; it carries no frames yet.
async fn hold(mut socket: WebSocket) {
    while let Some(Ok(_)) = socket.recv().await {}
}
