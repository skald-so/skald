//! An MCP client starts `agent-channel` over stdio and gets its channel events.

use std::path::Path;
use std::time::Duration;

use agent_source::{Message, Source};
use agent_testing::{Events, add_many, opened, taken_by_a_dead_server, wait_handed_over};
use rmcp::model::{ClientNotification, CustomNotification, ProtocolVersion};
use rmcp::service::RunningService;
use rmcp::transport::TokioChildProcess;
use rmcp::{RoleClient, ServiceExt};
use serde_json::json;
use tokio::sync::mpsc;

type Client = RunningService<RoleClient, Events>;

async fn start(source: &Path) -> (Client, mpsc::UnboundedReceiver<CustomNotification>) {
    start_with(&[], source).await
}

/// Starts `agent-channel` with `flags` before the source.
async fn start_with(
    flags: &[&Path],
    source: &Path,
) -> (Client, mpsc::UnboundedReceiver<CustomNotification>) {
    let mut command = tokio::process::Command::new(env!("CARGO_BIN_EXE_agent-channel"));
    // No variable of the test's environment, so none of its credentials reach the server.
    command.args(flags).arg(source).env_clear();
    let (events, received) = mpsc::unbounded_channel();
    let client = Events(events)
        .serve(TokioChildProcess::new(command).unwrap())
        .await
        .unwrap();
    (client, received)
}

/// The next event, within 10 s.
async fn next(received: &mut mpsc::UnboundedReceiver<CustomNotification>) -> CustomNotification {
    tokio::time::timeout(Duration::from_secs(10), received.recv())
        .await
        .expect("an event within 10 s")
        .expect("the server runs")
}

/// Asserts that `event` carries `message`.
fn assert_carries(event: &CustomNotification, message: &Message) {
    assert_eq!(event.method, "notifications/claude/channel");
    assert_eq!(
        event.params,
        Some(json!({
            "content": message.text,
            "meta": { "msg_id": message.msg_id, "from": message.sender },
        }))
    );
}

#[tokio::test]
async fn declares_a_channel_on_a_revision_with_a_handshake() {
    let dir = tempfile::tempdir().unwrap();
    let (client, _received) = start(dir.path()).await;
    let info = client.peer_info().expect("the server initialized");
    let experimental = info.capabilities.experimental.clone().unwrap_or_default();
    assert!(experimental.contains_key("claude/channel"), "{info:?}");
    assert!(
        !experimental.contains_key("claude/channel/permission"),
        "no relay without a log: {info:?}"
    );
    assert!(info.capabilities.tools.is_none(), "{info:?}");
    assert!(
        info.protocol_version.as_str() <= ProtocolVersion::V_2025_11_25.as_str(),
        "{info:?}"
    );
    let instructions = info.instructions.clone().unwrap_or_default();
    assert!(
        instructions.contains("msg_id") && instructions.contains("from"),
        "{instructions}"
    );
    client.cancel().await.unwrap();
}

#[tokio::test]
async fn with_a_permission_log_records_each_notification_and_answers_none() {
    let dir = tempfile::tempdir().unwrap();
    let log = dir.path().join("permission.jsonl");
    let source = dir.path().join("source");
    let flag = Path::new("--permission-log");
    let (client, mut received) = start_with(&[flag, &log], &source).await;
    let info = client.peer_info().expect("the server initialized");
    let experimental = info.capabilities.experimental.clone().unwrap_or_default();
    assert!(
        experimental.contains_key("claude/channel")
            && experimental.contains_key("claude/channel/permission"),
        "{info:?}"
    );

    let params = json!({
        "request_id": "abcde",
        "tool_name": "Bash",
        "description": "Run shell command",
        "input_preview": "{\"command\": \"touch prompted.txt\"}",
    });
    for (method, params) in [
        ("notifications/claude/channel/permission_request", &params),
        ("notifications/other", &json!({})),
    ] {
        let notification = CustomNotification::new(method, Some(params.clone()));
        client
            .send_notification(ClientNotification::CustomNotification(notification))
            .await
            .unwrap();
    }
    let deadline = std::time::Instant::now() + Duration::from_secs(10);
    let lines = loop {
        let text = std::fs::read_to_string(&log).unwrap_or_default();
        if text.lines().count() >= 3 || std::time::Instant::now() > deadline {
            break text;
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    };
    let records: Vec<serde_json::Value> = lines
        .lines()
        .map(|line| serde_json::from_str(line).unwrap())
        .collect();
    assert_eq!(records.len(), 3, "{lines}");
    // The handshake's end, then each notification the client sent.
    assert_eq!(records[0]["method"], "notifications/initialized");
    assert_eq!(
        records[1]["method"],
        "notifications/claude/channel/permission_request"
    );
    assert_eq!(records[1]["params"], params);
    assert!(records[1]["at_ms"].as_u64().unwrap() > 0, "{lines}");
    assert_eq!(records[2]["method"], "notifications/other");
    // It never answers: no verdict, nor any other notification, goes to the client.
    tokio::time::sleep(Duration::from_millis(300)).await;
    assert!(received.try_recv().is_err(), "no notification to the CLI");
    client.cancel().await.unwrap();
}

#[tokio::test]
async fn pushes_one_event_per_waiting_message_once() {
    let (dir, source) = opened();
    let first = source.add("reviewer of alice", "Tests pass.").unwrap();
    let second = source.add("planner of bob", "Take 4.3 next.").unwrap();
    let (client, mut received) = start(dir.path()).await;

    // The messages that waited before the session, oldest first.
    for message in [&first, &second] {
        assert_carries(&next(&mut received).await, message);
        wait_handed_over(&source, &message.msg_id);
    }
    // A message added later, while the session runs.
    let third = source.add("reviewer of alice", "Merged.").unwrap();
    assert_carries(&next(&mut received).await, &third);
    wait_handed_over(&source, &third.msg_id);

    // Nothing is pushed again: the server polls every 100 ms.
    tokio::time::sleep(Duration::from_millis(500)).await;
    assert!(received.try_recv().is_err(), "an event came twice");
    client.cancel().await.unwrap();
}

#[tokio::test]
async fn two_sessions_on_one_source_get_each_message_once() {
    let (dir, source) = opened();
    let (a, mut received_a) = start(dir.path()).await;
    let (b, mut received_b) = start(dir.path()).await;
    let added = add_many(&source, 20);
    let mut got = Vec::new();
    while got.len() < added.len() {
        let event = tokio::select! {
            Some(event) = received_a.recv() => event,
            Some(event) = received_b.recv() => event,
            () = tokio::time::sleep(Duration::from_secs(10)) => panic!("events missing: {got:?}"),
        };
        got.push(event.params.unwrap()["meta"]["msg_id"].clone());
    }
    for msg_id in &added {
        wait_handed_over(&source, msg_id);
    }
    tokio::time::sleep(Duration::from_millis(500)).await;
    assert!(
        received_a.try_recv().is_err() && received_b.try_recv().is_err(),
        "an event came twice"
    );
    got.sort_by_key(|id| id.to_string());
    let expected: Vec<_> = added.iter().map(|id| json!(id)).collect();
    assert_eq!(got, expected, "each message once, to one of the sessions");
    a.cancel().await.unwrap();
    b.cancel().await.unwrap();
}

#[tokio::test]
async fn messages_of_a_server_that_died_are_pushed_again() {
    let (dir, source) = opened();
    // A server that took a message and died before writing its event leaves its claim.
    let message = taken_by_a_dead_server(dir.path(), &source);
    assert_pushed_again(&source, &message).await;
}

#[tokio::test]
async fn a_server_whose_source_fails_ends_and_its_messages_are_pushed_again() {
    let (dir, source) = opened();
    let (client, mut received) = start(dir.path()).await;
    // The server cannot mark a message handed over: it writes it into `handed`.
    let handed = dir.path().join("handed");
    std::fs::remove_dir_all(&handed).unwrap();
    let message = source.add("alice", "hi").unwrap();
    assert_carries(&next(&mut received).await, &message);
    tokio::time::timeout(Duration::from_secs(10), client.waiting())
        .await
        .expect("the server ends within 10 s")
        .unwrap();

    // Its claim is free, so the next session gets the message again.
    std::fs::create_dir(&handed).unwrap();
    assert_pushed_again(&source, &message).await;
}

/// Asserts that a server started on `source` pushes `message` and marks it handed over.
async fn assert_pushed_again(source: &Source, message: &Message) {
    let (client, mut received) = start(source.dir()).await;
    assert_carries(&next(&mut received).await, message);
    wait_handed_over(source, &message.msg_id);
    client.cancel().await.unwrap();
}
