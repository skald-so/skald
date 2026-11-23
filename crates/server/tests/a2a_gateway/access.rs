use std::time::Duration;

use serde_json::{Value, json};
use url::Url;

use super::gateway::{Answer, Lab, example};
use super::{GET_TASK, SEND_MESSAGE, rpc};
use crate::common::devices;
use crate::https::Https;
use crate::kit::World;
use crate::tokens::{QUEUES, RECEIVE, SEND, grant};
use crate::wire::now;

const ALICE: &str = "fixture-client-credential-1";
const BOB: &str = "fixture-client-credential-2";
const REVOKED: &str = "fixture-client-credential-3";
const INVALID: &str = "fixture-client-credential-4";
const TEXT: &str = "The fixture's private message text.";
const QUEUE: &str = "private-fixture-queue";
const TASK: &str = "test-task-primary";
const OTHER_TASK: &str = "test-task-secondary";
const UNKNOWN_TASK: &str = "test-task-unknown";
const CONTEXT: &str = "test-context-primary";
const WRONG_CONTEXT: &str = "test-context-different";
const VERSION: &str = "1.0";

const INVALID_PARAMS: i64 = -32602;
const TASK_NOT_FOUND: i64 = -32001;
const PUSH_UNSUPPORTED: i64 = -32003;
const UNSUPPORTED: i64 = -32004;
const CONTENT_UNSUPPORTED: i64 = -32005;
const VERSION_UNSUPPORTED: i64 = -32009;

const PERMISSIONS: &[&str] = &[SEND, RECEIVE, QUEUES];

#[test]
fn gateway_checks_access_over_https_before_resolving_or_sending_tasks() {
    let example = example();
    let mut world = World::new(3_856_100_010);
    let other_channel = crate::bot::ops(&mut world.stage);
    let stage = &world.stage;
    let alice = &stage.f.team.alice;
    let member = world.laptop_on(world.server(), alice);
    let mailbox = Https::to(world.server(), &stage.f.team.flow, alice, &alice.laptop);
    let primary = member.queue("default").id;
    let secondary = member.creates(&json!({ "name": QUEUE })).id;
    let initial_tasks = [primary, secondary].map(|queue| member.tasks(queue));
    let initial_mailbox = mailbox.poll_now(0).entries;
    let lab = Lab::new(&world);

    let live = crate::bot::token(stage, PERMISSIONS);
    let revoked = crate::bot::token(stage, PERMISSIONS);
    let no_send = crate::bot::token(stage, &[RECEIVE, QUEUES]);
    let no_receive = crate::bot::token(stage, &[SEND, QUEUES]);
    let no_queues = crate::bot::token(stage, &[SEND, RECEIVE]);
    let other_grant = grant(
        stage.bot.id,
        &[other_channel],
        &alice.laptop,
        now() + 86_400,
    );
    let wrong_channel = stage.create_as(alice, stage.bot.id, &other_grant, PERMISSIONS);

    let binding = |token: &str| lab.queue_binding(&world, primary, Some(token));
    let mut wrong_proof = binding(&live.token);
    let wrong_signing_key = devices::b64(&alice.laptop.signing.secret);
    wrong_proof["signingKey"] = json!(wrong_signing_key);
    let secrets = [
        &live,
        &revoked,
        &no_send,
        &no_receive,
        &no_queues,
        &wrong_channel,
    ];
    let mut private = vec![
        ("upstream origin", lab.skald.clone()),
        ("space", world.space.to_string()),
        ("channel", world.channel.to_string()),
        ("other channel", other_channel.to_string()),
        ("service device", stage.bot.id.to_string()),
        ("queue", primary.to_string()),
        ("other queue", secondary.to_string()),
        ("other queue name", QUEUE.to_owned()),
        ("service key", devices::b64(&stage.bot.signing.secret)),
        ("wrong service key", wrong_signing_key),
        ("request text", TEXT.to_owned()),
    ];
    private.extend(
        [ALICE, BOB, REVOKED, INVALID].map(|value| ("gateway credential", value.to_owned())),
    );
    private.extend(secrets.map(|created| ("service credential", created.token.clone())));

    let expires = now() + 5;
    let short_grant = grant(stage.bot.id, &[world.channel], &alice.laptop, expires);
    let expired = stage.create_as(alice, stage.bot.id, &short_grant, PERMISSIONS);
    private.push(("expiring service credential", expired.token.clone()));

    let routes = json!({
        "primary": binding(&live.token),
        "secondary": lab.queue_binding(&world, secondary, Some(&live.token)),
        "missing-grant": lab.queue_binding(&world, primary, None),
        "expired-grant": binding(&expired.token),
        "revoked-grant": binding(&revoked.token),
        "wrong-channel": binding(&wrong_channel.token),
        "wrong-proof": wrong_proof,
        "no-send": binding(&no_send.token),
        "no-receive": binding(&no_receive.token),
        "no-queues": binding(&no_queues.token),
    });
    let credentials = json!([
        {
            "client": "alice", "bearer": ALICE, "revoked": false,
            "routes": routes.as_object().expect("fixture routes").keys().collect::<Vec<_>>(),
        },
        { "client": "bob", "bearer": BOB, "routes": ["primary"], "revoked": false },
        { "client": "alice", "bearer": REVOKED, "routes": ["primary"], "revoked": true },
    ]);
    let tasks = json!([
        { "id": TASK, "contextId": CONTEXT, "client": "alice", "route": "primary" },
        { "id": OTHER_TASK, "contextId": CONTEXT, "client": "alice", "route": "secondary" },
    ]);
    let process = lab.start(&example, routes, credentials, tasks);
    let exchange = Exchange {
        lab: &lab,
        private: &private,
    };
    exchange.card();
    let unknown = send_message(Some(UNKNOWN_TASK), None);
    assert_error(
        &exchange.request("revoked-grant", Some(ALICE), VERSION, &unknown),
        TASK_NOT_FOUND,
        "the revocable grant before revocation",
    );
    assert_eq!(stage.revoke_as(alice, &revoked.id).status, 204);

    exchange.credentials();
    exchange.local_validation();
    exchange.task_scope();
    while now() <= expires {
        std::thread::sleep(Duration::from_millis(100));
    }
    exchange.grants();
    assert_private(process.stop().as_bytes(), &private);

    let seen = lab.upstream.plain();
    assert!(!seen.contains("post /mailbox/send "), "a refusal sent mail");
    assert!(
        !seen
            .lines()
            .any(|line| line.contains("post /spaces/") && line.contains("/tasks ")),
        "a refusal attempted to enqueue"
    );
    assert_eq!(
        [primary, secondary].map(|queue| member.tasks(queue)),
        initial_tasks
    );
    assert_eq!(mailbox.poll_now(0).entries, initial_mailbox);
}

struct Exchange<'a> {
    lab: &'a Lab,
    private: &'a [(&'static str, String)],
}

impl Exchange<'_> {
    fn request(&self, route: &str, token: Option<&str>, version: &str, request: &Value) -> Answer {
        let answer = self.lab.rpc(route, token, version, request);
        assert_private(&answer.bytes, self.private);
        answer
    }

    fn card(&self) {
        let answer = self.lab.card();
        assert_eq!(answer.status, 200, "the isolated Agent Card");
        assert_private(&answer.bytes, self.private);
        let card = answer.json();
        assert_eq!(card["defaultInputModes"], json!(["text/plain"]));
        assert_eq!(card["defaultOutputModes"], json!(["text/plain"]));
        for capability in ["pushNotifications", "streaming", "extendedAgentCard"] {
            assert_ne!(card["capabilities"][capability], true, "{capability}");
        }
        let interfaces = card["supportedInterfaces"]
            .as_array()
            .expect("Agent Card interfaces");
        assert!(!interfaces.is_empty(), "a JSON-RPC HTTPS interface");
        let mut offers_primary = false;
        for interface in interfaces {
            assert_eq!(interface["protocolBinding"], "JSONRPC");
            assert_eq!(interface["protocolVersion"], VERSION);
            let url =
                Url::parse(interface["url"].as_str().expect("interface URL")).expect("valid URL");
            assert_eq!(url.origin().ascii_serialization(), self.lab.public);
            offers_primary |= url.path() == super::ROUTE;
        }
        assert!(
            offers_primary,
            "the card advertises the fixture's primary endpoint"
        );
        let schemes = card["securitySchemes"]
            .as_object()
            .expect("Agent Card security schemes");
        let requirements = card["securityRequirements"]
            .as_array()
            .expect("Agent Card security requirements");
        assert!(!requirements.is_empty(), "the card requires authentication");
        for requirement in requirements {
            let names = requirement["schemes"]
                .as_object()
                .expect("required scheme names");
            assert!(
                names
                    .keys()
                    .any(|name| schemes[name]["httpAuthSecurityScheme"]["scheme"]
                        .as_str()
                        .is_some_and(|scheme| scheme.eq_ignore_ascii_case("bearer"))),
                "each listed authentication alternative requires a bearer credential"
            );
        }
    }

    fn credentials(&self) {
        let before = self.lab.upstream.plain().len();
        for (route, bearer) in [
            ("primary", None),
            ("primary", Some(INVALID)),
            ("primary", Some(REVOKED)),
            ("secondary", Some(BOB)),
        ] {
            let send = self.request(route, bearer, VERSION, &send_message(None, None));
            let malformed = self.request(route, bearer, VERSION, &rpc(SEND_MESSAGE, json!({})));
            let get = self.request(
                route,
                bearer,
                VERSION,
                &rpc(GET_TASK, json!({ "id": TASK })),
            );
            let missing_id = self.request(route, bearer, VERSION, &rpc(GET_TASK, json!({})));
            refused(&send, "SendMessage credential or route");
            refused(&get, "GetTask credential or route");
            assert_eq!(
                send.status, malformed.status,
                "credential/route checked before syntax"
            );
            assert!(
                send.bytes == malformed.bytes,
                "syntax affects an unauthorized response"
            );
            assert_eq!(
                get.status, missing_id.status,
                "credential/route checked before task ID"
            );
            assert!(
                get.bytes == missing_id.bytes,
                "task ID affects an unauthorized response"
            );
        }
        assert_eq!(
            self.lab.upstream.plain().len(),
            before,
            "unauthorized upstream I/O"
        );
    }

    fn local_validation(&self) {
        let before = self.lab.upstream.plain().len();
        let mut callback = send_message(Some(TASK), None);
        callback["params"]["configuration"] =
            json!({ "taskPushNotificationConfig": { "url": self.lab.skald } });
        let mut file = send_message(Some(TASK), None);
        file["params"]["message"]["parts"] =
            json!([{ "raw": "Zml4dHVyZQ==", "mediaType": "application/octet-stream" }]);
        let mut data = send_message(Some(TASK), None);
        data["params"]["message"]["parts"] =
            json!([{ "data": { "sample": "text-only fixture" }, "mediaType": "application/json" }]);
        let mut references = send_message(Some(TASK), None);
        references["params"]["message"]["referenceTaskIds"] = json!([OTHER_TASK]);
        let mut streaming = send_message(None, None);
        streaming["method"] = json!("SendStreamingMessage");
        for (request, version, codes, label) in [
            (callback, VERSION, &[PUSH_UNSUPPORTED][..], "callback"),
            (file, VERSION, &[CONTENT_UNSUPPORTED], "file part"),
            (data, VERSION, &[CONTENT_UNSUPPORTED], "data part"),
            (
                references,
                VERSION,
                &[INVALID_PARAMS, UNSUPPORTED],
                "task references",
            ),
            (streaming, VERSION, &[UNSUPPORTED], "streaming"),
            (
                send_message(None, Some(CONTEXT)),
                VERSION,
                &[INVALID_PARAMS],
                "context alone",
            ),
            (
                rpc(SEND_MESSAGE, json!({})),
                VERSION,
                &[INVALID_PARAMS],
                "missing message",
            ),
            (
                send_message(None, None),
                "9.9",
                &[VERSION_UNSUPPORTED],
                "unsupported version",
            ),
        ] {
            error_with_code(
                &self.request("primary", Some(ALICE), version, &request),
                codes,
                label,
            );
        }
        assert_eq!(
            self.lab.upstream.plain().len(),
            before,
            "invalid requests reached Skald"
        );
    }

    fn task_scope(&self) {
        let before = self.lab.upstream.plain().len();
        let continuation = |client, task, context| {
            self.request(
                "primary",
                Some(client),
                VERSION,
                &send_message(Some(task), context),
            )
        };
        let unknown = assert_error(
            &continuation(ALICE, UNKNOWN_TASK, Some(WRONG_CONTEXT)),
            TASK_NOT_FOUND,
            "unknown task before context check",
        );
        for (client, task) in [(BOB, TASK), (ALICE, OTHER_TASK)] {
            let inaccessible = assert_error(
                &continuation(client, task, Some(WRONG_CONTEXT)),
                TASK_NOT_FOUND,
                "inaccessible task before context check",
            );
            assert_eq!(
                inaccessible, unknown,
                "continuation discloses task visibility"
            );
        }
        for (context, code) in [
            (None, UNSUPPORTED),
            (Some(CONTEXT), UNSUPPORTED),
            (Some(WRONG_CONTEXT), INVALID_PARAMS),
        ] {
            assert_error(&continuation(ALICE, TASK, context), code, "visible task");
        }
        assert_error(
            &self.request(
                "secondary",
                Some(ALICE),
                VERSION,
                &send_message(Some(OTHER_TASK), None),
            ),
            UNSUPPORTED,
            "the owner can reach the second configured queue route",
        );
        assert!(
            self.lab.upstream.plain().len() > before,
            "no live Skald access check"
        );
    }

    fn grants(&self) {
        for route in [
            "missing-grant",
            "expired-grant",
            "revoked-grant",
            "wrong-channel",
            "wrong-proof",
            "no-send",
            "no-receive",
            "no-queues",
        ] {
            let request = send_message(Some(UNKNOWN_TASK), Some(WRONG_CONTEXT));
            let send = self.request(route, Some(ALICE), VERSION, &request);
            let fresh = self.request(route, Some(ALICE), VERSION, &send_message(None, None));
            let get = self.request(
                route,
                Some(ALICE),
                VERSION,
                &rpc(GET_TASK, json!({ "id": UNKNOWN_TASK })),
            );
            refused(&send, route);
            refused(&get, route);
            assert_eq!(
                send.status, fresh.status,
                "{route}: grant checked before exchange type"
            );
            assert!(
                send.bytes == fresh.bytes,
                "{route}: exchange type affects a denied grant"
            );
            for answer in [send, get] {
                if let Ok(body) = serde_json::from_slice::<Value>(&answer.bytes) {
                    assert_ne!(
                        body["error"]["code"], TASK_NOT_FOUND,
                        "{route}: task lookup before grant"
                    );
                }
            }
        }
    }
}

fn send_message(task: Option<&str>, context: Option<&str>) -> Value {
    let mut message = json!({
        "messageId": "test-message", "role": "ROLE_USER", "parts": [{ "text": TEXT }],
    });
    if let Some(task) = task {
        message["taskId"] = json!(task);
    }
    if let Some(context) = context {
        message["contextId"] = json!(context);
    }
    rpc(SEND_MESSAGE, json!({ "message": message }))
}

#[track_caller]
fn refused(answer: &Answer, label: &str) {
    let parsed = serde_json::from_slice::<Value>(&answer.bytes);
    if (400..500).contains(&answer.status) {
        assert!(
            !parsed.is_ok_and(|body| body.get("result").is_some()),
            "{label}: exposed a result"
        );
        return;
    }
    assert!(
        (200..300).contains(&answer.status),
        "{label}: gateway HTTP status {}",
        answer.status
    );
    let body = parsed.expect("a JSON-RPC refusal");
    assert!(
        body["error"]["code"].is_i64() && body.get("result").is_none(),
        "{label}: no refusal"
    );
}

#[track_caller]
fn assert_error(answer: &Answer, code: i64, label: &str) -> Value {
    error_with_code(answer, &[code], label)
}

#[track_caller]
fn error_with_code(answer: &Answer, codes: &[i64], label: &str) -> Value {
    let body = answer.json();
    assert_eq!(body["jsonrpc"], "2.0", "{label}");
    assert_eq!(body["id"], 1, "{label}");
    assert!(body.get("result").is_none(), "{label}: exposed a result");
    let code = body["error"]["code"].as_i64().expect("JSON-RPC error code");
    assert!(
        codes.contains(&code),
        "{label}: error {code}, expected one of {codes:?}"
    );
    body["error"].clone()
}

#[track_caller]
fn assert_private(bytes: &[u8], private: &[(&str, String)]) {
    let text = String::from_utf8_lossy(bytes);
    for (label, value) in private {
        assert!(!text.contains(value), "disclosed fixture {label}");
    }
}

#[test]
fn polling_checks_the_client_and_the_configured_queue() {
    let example = example();
    let world = World::new(3_856_100_020);
    let stage = &world.stage;
    let member = world.laptop_on(world.server(), &stage.f.team.alice);
    let grant = crate::bot::token(stage, PERMISSIONS);
    let queue = member.queue("default").id;
    let missing = skald_proto::QueueId::generate();
    let lab = Lab::new(&world);
    let routes = json!({
        "primary": lab.queue_binding(&world, queue, Some(&grant.token)),
        "missing-queue": lab.queue_binding(&world, missing, Some(&grant.token)),
    });
    let credentials = json!([
        { "client": "alice", "bearer": ALICE, "routes": ["primary", "missing-queue"], "revoked": false },
        { "client": "bob", "bearer": BOB, "routes": ["primary"], "revoked": false },
    ]);
    let tasks = json!([
        { "id": TASK, "contextId": CONTEXT, "client": "alice", "route": "primary" },
        { "id": OTHER_TASK, "contextId": CONTEXT, "client": "alice", "route": "missing-queue" },
    ]);
    let _process = lab.start(&example, routes, credentials, tasks);
    let poll = |route, bearer, id| {
        lab.rpc(
            route,
            Some(bearer),
            VERSION,
            &rpc(GET_TASK, json!({ "id": id })),
        )
    };
    assert_error(
        &poll("primary", ALICE, TASK),
        UNSUPPORTED,
        "the owner's polling reaches the fixture correlation",
    );
    let unknown = assert_error(
        &poll("primary", ALICE, UNKNOWN_TASK),
        TASK_NOT_FOUND,
        "unknown task",
    );
    for (bearer, id) in [(BOB, TASK), (ALICE, OTHER_TASK)] {
        let inaccessible = assert_error(
            &poll("primary", bearer, id),
            TASK_NOT_FOUND,
            "out-of-scope task",
        );
        assert_eq!(inaccessible, unknown);
    }
    assert_eq!(
        poll("missing-queue", ALICE, OTHER_TASK).status,
        403,
        "the configured queue must exist even with a live channel grant"
    );
}
