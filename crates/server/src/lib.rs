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
mod attachments;
mod connections;
mod delivery;
mod device_flow;
mod devices;
mod error_reports;
mod expiry;
mod github;
mod google;
mod groups;
mod http_client;
mod instances;
mod key_packages;
mod links;
mod mailboxes;
mod notices;
mod oidc;
mod protocol;
mod refusal;
mod revocations;
mod sign_in;
mod sign_up;
mod spaces;

pub use attachments::{AttachmentStorage, BucketEndpoint, BucketSettings};
pub use error_reports::{SentryDsn, report_errors_to};
pub use github::{ClientSecret, GitHubApp, GitHubUrl};
use google::Google;
pub use oidc::{OidcError, OidcIdentity, OidcProvider, OidcSettings, issuer_url};
use protocol::Statement;
pub use protocol::Versions;
pub use sign_in::{CALLBACK_PATH, PublicUrl, SignInSettings};
pub use sign_up::{Listed, SignUp};

/// The OpenAPI spec the server implements: `api/openapi.json`, served at `GET /openapi.json`.
pub const OPENAPI_SPEC: &str = include_str!("../../../api/openapi.json");

/// The schema migrations in `migrations/`, applied at start.
pub static MIGRATOR: Migrator = sqlx::migrate!();

/// The longest time the server keeps a mailbox entry, in seconds: 100 years. It bounds the mailbox
/// retention and an envelope's `ttl`.
pub const MAILBOX_LONGEST: u64 = 100 * 365 * 24 * 3_600;

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
    /// Who may sign up.
    pub sign_up: SignUp,
    /// The number of spaces an account may own (`docs/dev/architecture.md`, "Cloud limits"); no
    /// limit without it.
    pub owned_spaces_limit: Option<u32>,
    /// The server's name in agent addresses; without it, the host of sign-in's public address, if
    /// that is a valid server name.
    pub server_name: Option<ServerName>,
    /// How long an undelivered mailbox entry waits at most, up to 100 years, and the least time the
    /// server keeps an idempotency key (`docs/protocol/delivery.md`, "Send" and "Mailbox"); without
    /// it, entries wait for their device's ack, unless an envelope's `ttl` ends first, and keys are
    /// kept for good.
    pub mailbox_retention: Option<Duration>,
    /// The longest payload of an envelope, in bytes (`docs/dev/architecture.md`, "Cloud limits"); no
    /// limit without it.
    pub message_size_limit: Option<u64>,
    /// The number of messages a space stores per calendar month in UTC (`docs/dev/architecture.md`,
    /// "Cloud limits"); no limit without it.
    pub monthly_messages_limit: Option<u64>,
    /// The number of devices with a certificate an account may have, `service` and revoked devices
    /// aside (`docs/dev/architecture.md`, "Cloud limits"); no limit without it.
    pub devices_limit: Option<u32>,
}

/// The state the handlers share.
#[derive(Clone)]
struct AppState {
    db: PgPool,
    versions: Versions,
    sign_in: Option<Arc<SignInSettings>>,
    google: Option<Arc<Google>>,
    sign_up: Arc<SignUp>,
    owned_spaces_limit: Option<u32>,
    server_name: Option<ServerName>,
    mailbox_retention: Option<Duration>,
    limits: mailboxes::Limits,
    devices_limit: Option<u32>,
    instance: instances::Instance,
    connections: Arc<connections::Connections>,
    wakers: Arc<delivery::Wakers>,
    notices: Arc<notices::Notices>,
}

/// The HTTP API described by [`OPENAPI_SPEC`], backed by the PostgreSQL pool `db`. Expires mailbox
/// entries and listens to the other instances on that database in tasks on the Tokio runtime it is
/// called on.
pub fn app(db: PgPool, settings: Settings) -> Router {
    let versions = settings.versions;
    let instance = instances::Instance::new();
    let wakers = Arc::new(delivery::Wakers::new(instance.clone()));
    let connections = Arc::<connections::Connections>::default();
    let notices = Arc::<notices::Notices>::default();
    tokio::spawn(expiry::run(
        db.clone(),
        Arc::clone(&wakers),
        settings.mailbox_retention,
    ));
    tokio::spawn(instances::listen(
        instance.clone(),
        instances::Local {
            db: db.clone(),
            wakers: Arc::clone(&wakers),
            notices: Arc::clone(&notices),
            connections: Arc::clone(&connections),
        },
    ));
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
            "/account/revocations",
            post(revocations::store).layer(checked(Statement::Header)),
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
        .route(
            "/auth/providers",
            get(sign_in::providers).layer(checked(Statement::Header)),
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
        .route(
            "/devices/{device}/key-packages",
            get(key_packages::status)
                .post(key_packages::upload)
                .layer(checked(Statement::Header)),
        )
        .route("/health", get(health))
        .route(
            "/key-packages",
            post(key_packages::hand_out).layer(checked(Statement::Header)),
        )
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
            "/spaces/{space}/channels/{channel}/group-info",
            get(groups::latest)
                .put(groups::upload)
                .layer(checked(Statement::Header)),
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
        .route_layer(middleware::from_fn(error_reports::report_failures))
        .with_state(AppState {
            db,
            versions,
            owned_spaces_limit: settings.owned_spaces_limit,
            server_name,
            mailbox_retention: settings.mailbox_retention,
            limits: mailboxes::Limits {
                message_size: settings.message_size_limit,
                monthly_messages: settings.monthly_messages_limit,
            },
            devices_limit: settings.devices_limit,
            instance,
            connections,
            wakers,
            notices,
            google: settings
                .sign_in
                .as_ref()
                .and_then(|sign_in| sign_in.google.clone())
                .map(|google| Arc::new(Google::new(google))),
            sign_in: settings.sign_in.map(Arc::new),
            sign_up: Arc::new(settings.sign_up),
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
/// to the server's output and its error reports.
fn database_failed(doing: &str, err: impl error_reports::Cause) -> Response {
    let cause = error_reports::logged(doing, &err);
    error_reports::with_cause(
        problem(
            StatusCode::INTERNAL_SERVER_ERROR,
            "server-error",
            "The server failed",
            "The database failed.",
        ),
        cause,
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
