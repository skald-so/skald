//! The A2A gateway's isolated HTTP handler. The server listener does not mount it.

mod rpc;
mod skald;

use std::collections::{HashMap, HashSet};
use std::sync::Arc;

use axum::body::to_bytes;
use axum::extract::{Request, State};
use axum::http::{HeaderValue, StatusCode, header};
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use axum::{Json, Router};
use serde_json::{Value, json};
use skald_proto::{ChannelId, DeviceId, Name, QueueId, SpaceId};
use ureq::Agent;
use ureq::tls::Certificate;

use crate::PublicUrl;
use crate::http_client::gateway_agent;
use crate::sign_in::{bearer, digest};

pub use skald::ServiceSecret;

const CARD_PATH: &str = "/.well-known/agent-card.json";
const ROUTE_PREFIX: &str = "/a2a/";
const VERSION: &str = "1.0";
const TEXT: &str = "text/plain";
const BODY_LIMIT: usize = 2 << 20;

/// A route's fixed Skald destination.
pub enum Destination {
    Queue(QueueId),
    Member { device: DeviceId, agent: Name },
}

/// A route's Skald server, channel, destination and service identity.
pub struct Binding {
    pub skald_origin: PublicUrl,
    pub space: SpaceId,
    pub channel: ChannelId,
    pub destination: Destination,
    pub secret: Option<ServiceSecret>,
}

/// A gateway credential's client, allowed aliases and revocation state.
pub struct Credential {
    pub client: String,
    pub routes: HashSet<String>,
    pub revoked: bool,
    hash: Vec<u8>,
}

impl Credential {
    pub fn new(
        client: String,
        bearer: &str,
        routes: HashSet<String>,
        revoked: bool,
    ) -> Option<Self> {
        if client.is_empty()
            || bearer.is_empty()
            || !bearer.bytes().all(|byte| byte.is_ascii_graphic())
        {
            return None;
        }
        Some(Self {
            client,
            routes,
            revoked,
            hash: digest(bearer),
        })
    }
}

/// An existing task correlation. New exchanges do not create correlations yet.
pub struct Correlation {
    pub context_id: String,
    pub client: String,
    pub route: String,
}

/// Inputs to the isolated gateway router. Secrets are supplied by the gateway process.
pub struct Config {
    pub public_origin: PublicUrl,
    pub advertised_route: String,
    pub routes: HashMap<String, Binding>,
    pub credentials: Vec<Credential>,
    pub tasks: HashMap<String, Correlation>,
}

impl Config {
    /// Builds the gateway router for a listener behind HTTPS. The optional roots replace the
    /// platform trust store for the Skald leg.
    pub fn router(self, roots: Option<Vec<Certificate<'static>>>) -> Option<Router> {
        if !self.public_origin.secure()
            || !self.routes.contains_key(&self.advertised_route)
            || !self.routes.iter().all(|(alias, binding)| {
                !alias.is_empty()
                    && alias
                        .bytes()
                        .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_'))
                    && binding.skald_origin.secure()
            })
        {
            return None;
        }

        let mut credentials = HashMap::new();
        for credential in self.credentials {
            if !credential
                .routes
                .iter()
                .all(|alias| self.routes.contains_key(alias))
                || credentials.contains_key(&credential.hash)
            {
                return None;
            }
            credentials.insert(credential.hash.clone(), credential);
        }
        let interface = self
            .public_origin
            .at(&format!("{ROUTE_PREFIX}{}", self.advertised_route));
        let gateway = Arc::new(Gateway {
            card: agent_card(interface),
            routes: self.routes,
            credentials,
            tasks: self.tasks,
            skald: gateway_agent(roots),
        });
        Some(
            Router::new()
                .route(CARD_PATH, get(card))
                .route("/a2a/{route}", post(call))
                .with_state(gateway)
                .layer(axum::middleware::map_response(no_store)),
        )
    }
}

struct Gateway {
    card: Value,
    routes: HashMap<String, Binding>,
    credentials: HashMap<Vec<u8>, Credential>,
    tasks: HashMap<String, Correlation>,
    skald: Agent,
}

async fn no_store(mut response: Response) -> Response {
    response
        .headers_mut()
        .insert(header::CACHE_CONTROL, HeaderValue::from_static("no-store"));
    response
}

fn agent_card(interface: String) -> Value {
    json!({
        "name": "Skald A2A gateway",
        "description": "Single-turn text exchanges through Skald.",
        "version": env!("CARGO_PKG_VERSION"),
        "supportedInterfaces": [{
            "url": interface,
            "protocolBinding": "JSONRPC",
            "protocolVersion": VERSION,
        }],
        "capabilities": {},
        "securitySchemes": {
            "bearer": { "httpAuthSecurityScheme": { "scheme": "Bearer" } },
        },
        "securityRequirements": [{ "schemes": { "bearer": {} } }],
        "defaultInputModes": [TEXT],
        "defaultOutputModes": [TEXT],
        "skills": [],
    })
}

async fn card(State(gateway): State<Arc<Gateway>>) -> Json<Value> {
    Json(gateway.card.clone())
}

async fn call(State(gateway): State<Arc<Gateway>>, request: Request) -> Response {
    let (head, body) = request.into_parts();
    let credential = bearer(&head.headers)
        .flatten()
        .and_then(|token| gateway.credentials.get(&digest(token)))
        .filter(|credential| !credential.revoked);
    let Some(credential) = credential else {
        return (
            StatusCode::UNAUTHORIZED,
            [(header::WWW_AUTHENTICATE, "Bearer")],
        )
            .into_response();
    };
    let route = head
        .uri
        .path()
        .strip_prefix(ROUTE_PREFIX)
        .unwrap_or_default();
    if !credential.routes.contains(route) {
        return StatusCode::FORBIDDEN.into_response();
    }
    let json = head
        .headers
        .get(header::CONTENT_TYPE)
        .and_then(|value| value.to_str().ok())
        .and_then(|value| value.split(';').next())
        .is_some_and(|value| value.trim().eq_ignore_ascii_case("application/json"));
    if !json {
        return StatusCode::UNSUPPORTED_MEDIA_TYPE.into_response();
    }
    let Ok(bytes) = to_bytes(body, BODY_LIMIT).await else {
        return StatusCode::PAYLOAD_TOO_LARGE.into_response();
    };
    let request = match rpc::Request::read(&bytes) {
        Ok(request) => request,
        Err(error) => return error.answer(&Value::Null),
    };
    let mut versions = head.headers.get_all("a2a-version").iter();
    if versions.next().is_none_or(|given| given != VERSION) || versions.next().is_some() {
        return rpc::Error::Version.answer(&request.id);
    }
    let operation = match request.operation() {
        Ok(operation) => operation,
        Err(error) => return error.answer(&request.id),
    };

    let check_gateway = Arc::clone(&gateway);
    let check_route = route.to_owned();
    let granted = tokio::task::spawn_blocking(move || {
        check_gateway.routes[&check_route].granted(&check_gateway.skald)
    })
    .await
    .unwrap_or(false);
    if !granted {
        return StatusCode::FORBIDDEN.into_response();
    }

    let error = match operation {
        rpc::Operation::SendNew => rpc::Error::Unsupported,
        rpc::Operation::Existing { task, context } => {
            match gateway
                .tasks
                .get(&task)
                .filter(|task| task.client == credential.client && task.route == route)
            {
                None => rpc::Error::TaskNotFound,
                Some(task) if context.is_some_and(|context| context != task.context_id) => {
                    rpc::Error::InvalidParams
                }
                Some(_) => rpc::Error::Unsupported,
            }
        }
    };
    error.answer(&request.id)
}
