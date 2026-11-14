//! Skald server.

use std::io;
use std::sync::Arc;
use std::time::Duration;

use axum::extract::State;
use axum::http::{StatusCode, header};
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post, put};
use axum::{Json, Router, middleware};
use serde::Serialize;
use skald_proto::ServerName;
use sqlx::migrate::Migrator;
use sqlx::postgres::{PgConnectOptions, PgPoolOptions};
use sqlx::{Connection, PgConnection, PgPool};

mod accounts;
mod connections;
mod device_flow;
mod devices;
mod github;
mod google;
mod http_client;
mod links;
mod mailboxes;
mod oidc;
mod protocol;
mod refusal;
mod sign_in;
mod spaces;

pub use github::{ClientSecret, GitHubApp, GitHubUrl};
use google::Google;
pub use oidc::{OidcError, OidcIdentity, OidcProvider, OidcSettings, issuer_url};
use protocol::Statement;
pub use protocol::Versions;
pub use sign_in::{CALLBACK_PATH, PublicUrl, SignInSettings};

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

/// The server's settings.
#[derive(Debug, Clone, Default)]
pub struct Settings {
    /// The protocol versions it accepts.
    pub versions: Versions,
    /// Sign-in; off without it.
    pub sign_in: Option<SignInSettings>,
    /// The number of spaces an account may own (`docs/dev/architecture.md`, "Cloud limits"); no
    /// limit without it.
    pub owned_spaces_limit: Option<u32>,
    /// The server's name in agent addresses; without it, the host of sign-in's public address, if
    /// that is a valid server name.
    pub server_name: Option<ServerName>,
}

/// The state the handlers share.
#[derive(Clone)]
struct AppState {
    db: PgPool,
    versions: Versions,
    sign_in: Option<Arc<SignInSettings>>,
    google: Option<Arc<Google>>,
    owned_spaces_limit: Option<u32>,
    server_name: Option<ServerName>,
    connections: Arc<connections::Connections>,
}

/// The HTTP API described by [`OPENAPI_SPEC`], backed by the PostgreSQL pool `db`.
pub fn app(db: PgPool, settings: Settings) -> Router {
    let versions = settings.versions;
    let server_name = settings.server_name.or_else(|| {
        settings
            .sign_in
            .as_ref()
            .and_then(|sign_in| sign_in.public_url.server_name())
    });
    let checked =
        |statement| middleware::from_fn_with_state((versions, statement), protocol::check);
    Router::new()
        .route(
            "/account/master",
            get(devices::master)
                .put(devices::give_master)
                .layer(checked(Statement::Header)),
        )
        .route(
            "/account/personal-space",
            post(spaces::personal).layer(checked(Statement::Header)),
        )
        .route(
            "/account/sign-in-methods",
            get(links::list).layer(checked(Statement::Header)),
        )
        .route(
            "/account/username",
            put(accounts::set_username).layer(checked(Statement::Header)),
        )
        .route(
            "/auth/device",
            post(device_flow::authorize).layer(checked(Statement::Header)),
        )
        .route(sign_in::START_PATH, get(sign_in::start::<GitHubApp>))
        .route(CALLBACK_PATH, get(sign_in::callback::<GitHubApp>))
        .route(links::GITHUB_PATH, post(links::start::<GitHubApp>))
        .route(google::START_PATH, get(sign_in::start::<Google>))
        .route(google::CALLBACK_PATH, get(sign_in::callback::<Google>))
        .route(links::GOOGLE_PATH, post(links::start::<Google>))
        .route(
            "/auth/token",
            post(device_flow::token).layer(checked(Statement::Header)),
        )
        .route(
            device_flow::VERIFICATION_PATH,
            get(device_flow::page).post(device_flow::decide),
        )
        .route(
            "/devices",
            get(devices::list)
                .post(devices::register)
                .layer(checked(Statement::Header)),
        )
        .route("/health", get(health))
        .route("/openapi.json", get(openapi_spec))
        .route(
            "/protocol",
            get(protocol_versions).layer(checked(Statement::Header)),
        )
        .route("/server", get(server).layer(checked(Statement::Header)))
        .route(
            "/session",
            get(sign_in::session)
                .delete(sign_in::end)
                .layer(checked(Statement::Header)),
        )
        .route(
            "/session/challenge",
            post(connections::challenge).layer(checked(Statement::Header)),
        )
        .route(links::PAGE_PATH, get(links::page))
        .route(
            "/spaces",
            get(spaces::list)
                .post(spaces::create)
                .layer(checked(Statement::Header)),
        )
        .route(
            "/spaces/{space}/channels",
            post(spaces::channel).layer(checked(Statement::Header)),
        )
        .route(
            "/spaces/{space}/devices",
            get(spaces::devices).layer(checked(Statement::Header)),
        )
        .route(
            "/spaces/{space}/revisions",
            get(spaces::revisions)
                .post(spaces::store)
                .layer(checked(Statement::Header)),
        )
        .route(
            "/ws",
            get(connections::open).layer(checked(Statement::Subprotocol)),
        )
        .with_state(AppState {
            db,
            versions,
            owned_spaces_limit: settings.owned_spaces_limit,
            server_name,
            connections: Arc::default(),
            google: settings
                .sign_in
                .as_ref()
                .and_then(|sign_in| sign_in.google.clone())
                .map(|google| Arc::new(Google::new(google))),
            sign_in: settings.sign_in.map(Arc::new),
        })
}

/// `Problem` in the spec, of the type `https://skald.so/problems/<kind>`.
fn problem(status: StatusCode, kind: &str, title: &str, detail: &str) -> Response {
    let body = serde_json::json!({
        "type": format!("https://skald.so/problems/{kind}"),
        "title": title,
        "status": status.as_u16(),
        "detail": detail,
    });
    problem_details(status, body)
}

/// The problem details object `body` with `status`.
fn problem_details(status: StatusCode, body: impl Serialize) -> Response {
    (
        status,
        [(header::CONTENT_TYPE, "application/problem+json")],
        Json(body),
    )
        .into_response()
}

/// The answer when the database fails at `doing`, such as `read a session`; the failure also goes
/// to the server's output.
fn database_failed(doing: &str, err: impl std::fmt::Display) -> Response {
    eprintln!("skald-server: cannot {doing}: {err}");
    problem(
        StatusCode::INTERNAL_SERVER_ERROR,
        "server-error",
        "The server failed",
        "The database failed.",
    )
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

/// `Server` in the spec.
#[derive(Serialize)]
struct Server {
    name: Option<String>,
}

async fn server(State(state): State<AppState>) -> Json<Server> {
    Json(Server {
        name: state.server_name.as_ref().map(ToString::to_string),
    })
}
