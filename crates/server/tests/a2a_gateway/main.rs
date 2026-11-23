//! The single-turn A2A gateway contract over loopback HTTPS. The fixture example
//! starts the gateway handler independently of the server's public listener.
#![allow(dead_code)]

#[path = "../common/mod.rs"]
mod common;

#[path = "../../../client/tests/https/front.rs"]
mod front;
#[path = "../../../client/tests/common/tls.rs"]
mod tls;

// The Skald fixture chapters share mailbox, channel and service-token setup.
#[path = "../mailbox/fixture.rs"]
mod fixture;
#[path = "../mailbox_https/https.rs"]
mod https;
#[path = "../mailbox_delivery/link.rs"]
mod link;
#[path = "../mailbox_sse/sse.rs"]
mod sse;
#[path = "../mailbox/wire.rs"]
mod wire;

#[path = "../spaces/api.rs"]
mod api;
#[path = "../presence/kit.rs"]
mod presence;
#[path = "../revocations/kit.rs"]
mod revocations;
#[path = "../spaces/rosters.rs"]
mod rosters;
#[path = "../delivery_service/scene.rs"]
mod scene;

#[path = "../service_requests/bot.rs"]
mod bot;
#[path = "../queues/kit.rs"]
mod kit;
#[path = "../service_tokens/tokens.rs"]
mod tokens;

mod access;
mod gateway;

use common::{Server, TestDb, send};
use serde_json::{Value, json};

const CARD: &str = "/.well-known/agent-card.json";
const RPC: &str = "/a2a/{route}";
const ROUTE: &str = "/a2a/primary";
const SEND_MESSAGE: &str = "SendMessage";
const GET_TASK: &str = "GetTask";

#[test]
fn gateway_rpc_is_in_the_openapi_contract() {
    let db = TestDb::create();
    let server = Server::start(&db);
    let response = server.get("/openapi.json");
    assert_eq!(response.status, 200, "published OpenAPI contract");
    let spec: Value = serde_json::from_slice(&response.body).expect("OpenAPI JSON");
    assert!(
        spec["paths"][RPC]["post"].is_object(),
        "the OpenAPI contract has no gateway JSON-RPC operation"
    );
}

#[test]
fn the_default_server_does_not_expose_gateway_access_or_discovery() {
    let db = TestDb::create();
    let server = Server::start(&db);
    assert_eq!(server.get(CARD).status, 404, "Agent Card");
    let request = rpc(GET_TASK, json!({ "id": "test-task" })).to_string();
    let response = send(
        "post",
        &format!("{}{ROUTE}", server.base()),
        &[
            ("content-type", "application/json"),
            ("A2A-Version", "1.0"),
            ("authorization", "Bearer fixture-client-credential"),
        ],
        request.as_bytes(),
    );
    assert_eq!(response.status, 404, "gateway RPC");
}

fn rpc(method: &str, params: Value) -> Value {
    json!({ "jsonrpc": "2.0", "id": 1, "method": method, "params": params })
}
