//! Skald server.

use axum::Json;
use axum::Router;
use axum::http::header;
use axum::response::IntoResponse;
use axum::routing::get;
use serde::Serialize;

/// The OpenAPI spec the server implements: `api/openapi.json`, served at `GET /openapi.json`.
pub const OPENAPI_SPEC: &str = include_str!("../../../api/openapi.json");

/// The HTTP API described by [`OPENAPI_SPEC`].
pub fn app() -> Router {
    Router::new()
        .route("/health", get(health))
        .route("/openapi.json", get(openapi_spec))
}

/// `Health` in the spec.
#[derive(Serialize)]
struct Health {
    status: &'static str,
}

async fn health() -> Json<Health> {
    Json(Health { status: "ok" })
}

async fn openapi_spec() -> impl IntoResponse {
    ([(header::CONTENT_TYPE, "application/json")], OPENAPI_SPEC)
}
