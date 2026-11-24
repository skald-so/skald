//! Skald server.

use std::io;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use axum::extract::State;
use axum::http::{StatusCode, header};
use axum::response::{IntoResponse, Response};
use axum::routing::{delete, get, patch, post, put};
use axum::{Json, Router, middleware};
use serde::Serialize;
use skald_proto::ServerName;
use sqlx::migrate::Migrator;
use sqlx::postgres::{PgConnectOptions, PgPoolOptions};
use sqlx::{Connection, PgConnection, PgPool};

mod abuse;
mod accounts;
mod anchors;
mod attachments;
mod claims;
mod connections;
mod deletion;
mod delivery;
mod device_certs;
mod device_flow;
mod device_sessions;
mod devices;
mod erasure;
mod error_reports;
mod events;
mod expiry;
mod github;
mod groups;
mod history;
mod history_retention;
mod http_client;
mod instances;
mod invites;
mod key_packages;
mod link_runs;
mod links;
mod mailboxes;
mod notices;
mod oidc;
mod oidc_browser;
mod presence;
mod protocol;
mod queues;
mod refusal;
mod requests;
mod revocations;
mod service_tokens;
mod sessions;
mod sign_in;
mod sign_up;
mod space_plans;
mod spaces;
mod sweeps;
mod unread_bodies;
mod usage;
mod user_agent;

pub use attachments::{AttachmentStorage, BucketEndpoint, BucketSettings};
pub use error_reports::{SentryDsn, report_errors_to};
pub use events::SSE_KEEP_ALIVE;
pub use github::{ClientSecret, GitHubApp, GitHubUrl};
pub use link_runs::LINK_CODE_LIFETIME;
pub use oidc::{OidcError, OidcIdentity, OidcProvider, OidcSettings, issuer_url};
use oidc_browser::{Corporate, Google, OidcSignIn};
use protocol::Statement;
pub use protocol::Versions;
pub use requests::LONG_POLL_MAX_WAIT;
pub use sign_in::{CALLBACK_PATH, PrivacyPolicyUrl, PublicUrl, SignInSettings};
pub use sign_up::{Listed, SignUp};

/// The OpenAPI spec the server implements: `api/openapi.json`, served at `GET /openapi.json`.
pub const OPENAPI_SPEC: &str = include_str!("../../../api/openapi.json");

/// The schema migrations in `migrations/`, applied at start.
pub static MIGRATOR: Migrator = sqlx::migrate!();

/// The longest time the server keeps a mailbox entry, in seconds: 100 years. It bounds the mailbox
/// retention and an envelope's `ttl`.
pub const MAILBOX_LONGEST: u64 = 100 * 365 * 24 * 3_600;

/// The longest history retention, in seconds: 100 years.
pub const HISTORY_LONGEST: u64 = MAILBOX_LONGEST;

/// The window of the send rate limit without a setting: a minute.
pub const SEND_RATE_WINDOW: Duration = Duration::from_secs(60);

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
    /// The privacy policy that the answer on sign-in and the server's pages that start one link;
    /// none without it.
    pub privacy_policy_url: Option<PrivacyPolicyUrl>,
    /// Show effective cloud plans and their limits in space usage.
    pub space_plans: bool,
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
    /// How long the server keeps a channel's history entries, up to 100 years
    /// (`docs/protocol/membership.md`, "Uploads"); without it, the history is kept for good.
    pub history_retention: Option<Duration>,
    /// The longest payload of an envelope, and `ct` of a task, in bytes (`docs/dev/architecture.md`,
    /// "Cloud limits"); no limit without it.
    pub message_size_limit: Option<u64>,
    /// The number of messages a space stores per calendar month in UTC (`docs/dev/architecture.md`,
    /// "Cloud limits"); no limit without it.
    pub monthly_messages_limit: Option<u64>,
    /// The number of messages one device stores within [`Settings::send_rate_window`]
    /// (`docs/dev/architecture.md`, "Cloud limits"); no limit without it.
    pub send_rate_limit: Option<u64>,
    /// The sliding window of [`Settings::send_rate_limit`]; [`SEND_RATE_WINDOW`] without it.
    pub send_rate_window: Option<Duration>,
    /// Optional file of private rules for review findings.
    pub abuse_rules_file: Option<PathBuf>,
    /// The number of devices with a certificate an account may have, `service` and revoked devices
    /// aside (`docs/dev/architecture.md`, "Cloud limits"); no limit without it.
    pub devices_limit: Option<u32>,
    /// The number of service agent entries a space's latest revision may list
    /// (`docs/dev/architecture.md`, "Cloud limits"); no limit without it.
    pub service_agents_limit: Option<u32>,
    /// The longest wait of a long-poll (`docs/protocol/delivery.md`, "Mailbox");
    /// [`LONG_POLL_MAX_WAIT`] without it.
    pub long_poll_max_wait: Option<Duration>,
    /// How often an idle mailbox stream sends a keep-alive (`docs/protocol/delivery.md`,
    /// "Frames"); [`SSE_KEEP_ALIVE`] without it.
    pub sse_keep_alive: Option<Duration>,
    /// How long after its hello a link code's run ends (`docs/protocol/membership.md`, "Link
    /// code"); [`LINK_CODE_LIFETIME`] without it.
    pub link_code_lifetime: Option<Duration>,
}

/// The state the handlers share.
#[derive(Clone)]
struct AppState {
    db: PgPool,
    versions: Versions,
    sign_in: Option<Arc<SignInSettings>>,
    google: Option<Arc<OidcSignIn<Google>>>,
    corporate: Option<Arc<OidcSignIn<Corporate>>>,
    sign_up: Arc<SignUp>,
    privacy_policy_url: Option<Arc<PrivacyPolicyUrl>>,
    space_plans: bool,
    owned_spaces_limit: Option<u32>,
    server_name: Option<ServerName>,
    mailbox_retention: Option<Duration>,
    history_retention: Option<Duration>,
    limits: mailboxes::Limits,
    devices_limit: Option<u32>,
    service_agents_limit: Option<u32>,
    long_poll_max_wait: Duration,
    sse_keep_alive: Duration,
    link_code_lifetime: Duration,
    instance: instances::Instance,
    connections: Arc<connections::Connections>,
    wakers: Arc<delivery::Wakers>,
    notices: Arc<notices::Notices>,
}

/// The HTTP API described by [`OPENAPI_SPEC`], backed by the PostgreSQL pool `db`. Expires mailbox
/// entries, deletes history past its retention, ended link runs and expired invites, applies tasks'
/// lease ends and deadlines, and listens to the other instances on that database in tasks on the
/// Tokio runtime it is called on.
pub fn app(db: PgPool, settings: Settings) -> Router {
    let versions = settings.versions;
    let instance = instances::Instance::new();
    let wakers = Arc::new(delivery::Wakers::new(instance.clone()));
    let connections = Arc::<connections::Connections>::default();
    let notices = Arc::<notices::Notices>::default();
    let send_rate = settings
        .send_rate_limit
        .map(|messages| mailboxes::SendRate {
            messages,
            window: settings.send_rate_window.unwrap_or(SEND_RATE_WINDOW),
        });
    let counter_retention = settings
        .mailbox_retention
        .map(|retention| retention.max(send_rate.map_or(Duration::ZERO, |rate| rate.window)));
    tokio::spawn(expiry::run(
        db.clone(),
        Arc::clone(&wakers),
        settings.mailbox_retention,
        counter_retention,
    ));
    tokio::spawn(abuse::expire(db.clone()));
    if let Some(path) = settings.abuse_rules_file {
        tokio::spawn(abuse::run(db.clone(), path, counter_retention));
    }
    tokio::spawn(link_runs::sweep(db.clone()));
    tokio::spawn(invites::sweep(db.clone()));
    tokio::spawn(claims::run(db.clone(), settings.history_retention));
    tokio::spawn(erasure::run(db.clone()));
    if let Some(retention) = settings.history_retention {
        tokio::spawn(history_retention::run(db.clone(), retention));
    }
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
            "/account",
            delete(deletion::delete).layer(checked(Statement::Header)),
        )
        .route(
            "/account/anchors",
            get(anchors::list)
                .post(anchors::store)
                .layer(checked(Statement::Header)),
        )
        .route(
            "/account/deletion",
            get(deletion::preview).layer(checked(Statement::Header)),
        )
        .route(
            "/account/devices",
            get(devices::shown).layer(checked(Statement::Header)),
        )
        .route(
            "/account/links",
            post(link_runs::open).layer(checked(Statement::Header)),
        )
        .route(
            "/account/links/hello",
            get(link_runs::hello).layer(checked(Statement::Header)),
        )
        .route(
            "/account/links/messages",
            post(link_runs::relay).layer(checked(Statement::Header)),
        )
        .route(
            "/account/links/messages/{session}",
            get(link_runs::messages).layer(checked(Statement::Header)),
        )
        .route(
            "/account/links/pairing",
            post(link_runs::pair).layer(checked(Statement::Header)),
        )
        .route(
            "/account/links/waiting",
            get(link_runs::waiting).layer(checked(Statement::Header)),
        )
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
            "/account/sessions",
            get(sessions::list).layer(checked(Statement::Header)),
        )
        .route(
            "/account/sessions/end-others",
            post(sessions::end_others).layer(checked(Statement::Header)),
        )
        .route(
            "/account/sessions/{session}",
            delete(sessions::end_one).layer(checked(Statement::Header)),
        )
        .route(
            "/account/sign-in-methods",
            get(links::list).layer(checked(Statement::Header)),
        )
        .route(
            "/account/sign-in-methods/{method}",
            delete(links::remove).layer(checked(Statement::Header)),
        )
        .route(
            "/account/usage",
            get(usage::account).layer(checked(Statement::Header)),
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
            "/auth/device/invite",
            put(device_flow::attach_invite).layer(checked(Statement::Header)),
        )
        .route(
            "/auth/providers",
            get(sign_in::providers).layer(checked(Statement::Header)),
        )
        .route(sign_in::START_PATH, get(sign_in::start::<GitHubApp>))
        .route(CALLBACK_PATH, get(sign_in::callback::<GitHubApp>))
        .route(links::GITHUB_PATH, post(links::start::<GitHubApp>))
        .route(
            oidc_browser::GOOGLE_START,
            get(sign_in::start::<OidcSignIn<Google>>),
        )
        .route(
            oidc_browser::GOOGLE_CALLBACK,
            get(sign_in::callback::<OidcSignIn<Google>>),
        )
        .route(links::GOOGLE_PATH, post(links::start::<OidcSignIn<Google>>))
        .route(
            oidc_browser::CORPORATE_START,
            get(sign_in::start::<OidcSignIn<Corporate>>),
        )
        .route(
            oidc_browser::CORPORATE_CALLBACK,
            get(sign_in::callback::<OidcSignIn<Corporate>>),
        )
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
            "/devices/{device}/cert",
            put(device_certs::store).layer(checked(Statement::Header)),
        )
        .route(
            "/devices/{device}/challenge",
            post(device_sessions::challenge).layer(checked(Statement::Header)),
        )
        .route(
            "/devices/{device}/session",
            post(device_sessions::start).layer(checked(Statement::Header)),
        )
        .route(
            "/devices/{device}/key-packages",
            get(key_packages::status)
                .post(key_packages::upload)
                .layer(checked(Statement::Header)),
        )
        .route("/health", get(health))
        .route(
            "/invites/{invite}",
            get(invites::given).layer(checked(Statement::Header)),
        )
        .route(
            "/invites/{invite}/acceptance",
            put(invites::accept).layer(checked(Statement::Header)),
        )
        .route(
            "/key-packages",
            post(key_packages::hand_out).layer(checked(Statement::Header)),
        )
        .route(
            "/links",
            post(link_runs::open_sessionless).layer(checked(Statement::Header)),
        )
        .route(
            "/links/messages",
            get(link_runs::run_messages)
                .post(link_runs::run_relay)
                .layer(checked(Statement::Header)),
        )
        .route(
            "/mailbox/ack",
            post(requests::ack).layer(checked(Statement::Header)),
        )
        .route(
            "/mailbox/events",
            get(events::events).layer(checked(Statement::Header)),
        )
        .route(
            "/mailbox/poll",
            post(requests::poll).layer(checked(Statement::Header)),
        )
        .route(
            "/mailbox/send",
            post(requests::send).layer(checked(Statement::Header)),
        )
        .route("/openapi.json", get(openapi_spec))
        .route(
            "/protocol",
            get(protocol_versions).layer(checked(Statement::Header)),
        )
        .route("/server", get(server).layer(checked(Statement::Header)))
        .route(
            "/service-token",
            get(service_tokens::own).layer(checked(Statement::Header)),
        )
        .route(
            "/session",
            get(sign_in::session)
                .delete(sessions::sign_out)
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
            "/spaces/{space}/acceptances",
            get(invites::waiting).layer(checked(Statement::Header)),
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
            "/spaces/{space}/channels/{channel}/history/entries",
            get(history::given::<{ history::ENTRIES }>).layer(checked(Statement::Header)),
        )
        .route(
            "/spaces/{space}/channels/{channel}/history/links",
            get(history::given::<{ history::LINKS }>).layer(checked(Statement::Header)),
        )
        .route(
            "/spaces/{space}/channels/{channel}/history/wraps",
            get(history::given::<{ history::WRAPS }>).layer(checked(Statement::Header)),
        )
        .route(
            "/spaces/{space}/channels/{channel}/presence",
            get(presence::given)
                .put(presence::upload)
                .layer(checked(Statement::Header)),
        )
        .route(
            "/spaces/{space}/channels/{channel}/queue-keys/{epoch}",
            get(history::queue_key).layer(checked(Statement::Header)),
        )
        .route(
            "/spaces/{space}/channels/{channel}/queues",
            get(queues::list)
                .post(queues::create)
                .layer(checked(Statement::Header)),
        )
        .route(
            "/spaces/{space}/channels/{channel}/queues/{queue}",
            patch(queues::change)
                .delete(queues::delete)
                .layer(checked(Statement::Header)),
        )
        .route(
            "/spaces/{space}/channels/{channel}/queues/{queue}/claims",
            post(claims::claim).layer(checked(Statement::Header)),
        )
        .route(
            "/spaces/{space}/channels/{channel}/queues/{queue}/tasks",
            get(queues::tasks)
                .post(queues::enqueue)
                .layer(checked(Statement::Header)),
        )
        .route(
            "/spaces/{space}/channels/{channel}/queues/{queue}/tasks/{task}",
            get(queues::task).layer(checked(Statement::Header)),
        )
        .route(
            "/spaces/{space}/channels/{channel}/queues/{queue}/tasks/{task}/cancel",
            post(claims::change::<{ claims::CANCEL }>).layer(checked(Statement::Header)),
        )
        .route(
            "/spaces/{space}/channels/{channel}/queues/{queue}/tasks/{task}/complete",
            post(claims::act::<{ claims::COMPLETE }>).layer(checked(Statement::Header)),
        )
        .route(
            "/spaces/{space}/channels/{channel}/queues/{queue}/tasks/{task}/extend",
            post(claims::act::<{ claims::EXTEND }>).layer(checked(Statement::Header)),
        )
        .route(
            "/spaces/{space}/channels/{channel}/queues/{queue}/tasks/{task}/fail",
            post(claims::act::<{ claims::FAIL }>).layer(checked(Statement::Header)),
        )
        .route(
            "/spaces/{space}/channels/{channel}/queues/{queue}/tasks/{task}/release",
            post(claims::act::<{ claims::RELEASE }>).layer(checked(Statement::Header)),
        )
        .route(
            "/spaces/{space}/channels/{channel}/queues/{queue}/tasks/{task}/restart",
            post(claims::change::<{ claims::RESTART }>).layer(checked(Statement::Header)),
        )
        .route(
            "/spaces/{space}/devices",
            get(spaces::devices).layer(checked(Statement::Header)),
        )
        .route(
            "/spaces/{space}/invites",
            get(invites::list)
                .post(invites::issue)
                .layer(checked(Statement::Header)),
        )
        .route(
            "/spaces/{space}/invites/{invite}",
            delete(invites::revoke)
                .put(invites::store)
                .layer(checked(Statement::Header)),
        )
        .route(
            "/spaces/{space}/members",
            get(spaces::members).layer(checked(Statement::Header)),
        )
        .route(
            "/spaces/{space}/revisions",
            get(spaces::revisions)
                .post(spaces::store)
                .layer(checked(Statement::Header)),
        )
        .route(
            "/spaces/{space}/service-grants",
            get(service_tokens::grants).layer(checked(Statement::Header)),
        )
        .route(
            "/spaces/{space}/service-tokens",
            get(service_tokens::list)
                .post(service_tokens::create)
                .layer(checked(Statement::Header)),
        )
        .route(
            "/spaces/{space}/service-tokens/{token}",
            delete(service_tokens::revoke).layer(checked(Statement::Header)),
        )
        .route(
            "/spaces/{space}/usage",
            get(usage::space).layer(checked(Statement::Header)),
        )
        .route(
            "/ws",
            get(connections::open).layer(checked(Statement::Subprotocol)),
        )
        .route_layer(middleware::from_fn(error_reports::report_failures))
        .layer(middleware::from_fn(unread_bodies::close_unread))
        .with_state(AppState {
            db,
            versions,
            space_plans: settings.space_plans,
            owned_spaces_limit: settings.owned_spaces_limit,
            server_name,
            mailbox_retention: settings.mailbox_retention,
            history_retention: settings.history_retention,
            limits: mailboxes::Limits {
                message_size: settings.message_size_limit,
                monthly_messages: settings.monthly_messages_limit,
                send_rate,
            },
            devices_limit: settings.devices_limit,
            service_agents_limit: settings.service_agents_limit,
            long_poll_max_wait: settings.long_poll_max_wait.unwrap_or(LONG_POLL_MAX_WAIT),
            sse_keep_alive: settings.sse_keep_alive.unwrap_or(SSE_KEEP_ALIVE),
            link_code_lifetime: settings.link_code_lifetime.unwrap_or(LINK_CODE_LIFETIME),
            instance,
            connections,
            wakers,
            notices,
            google: settings
                .sign_in
                .as_ref()
                .and_then(|sign_in| sign_in.google.clone())
                .map(|settings| Arc::new(OidcSignIn::new(settings))),
            corporate: settings
                .sign_in
                .as_ref()
                .and_then(|sign_in| sign_in.corporate.clone())
                .map(|settings| Arc::new(OidcSignIn::new(settings))),
            sign_in: settings.sign_in.map(Arc::new),
            sign_up: Arc::new(settings.sign_up),
            privacy_policy_url: settings.privacy_policy_url.map(Arc::new),
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
    #[serde(skip_serializing_if = "Option::is_none")]
    public: Option<String>,
}

async fn server(State(state): State<AppState>) -> Json<Server> {
    Json(Server {
        name: state.server_name.as_ref().map(ToString::to_string),
        public: sign_in::settings(&state).map(|settings| settings.public_url.origin()),
    })
}
