//! An MCP client starts `agent-inbox` over stdio and calls `inbox`.

use std::path::Path;

use agent_source::Source;
use agent_testing::{assert_carries_once, wait_handed_over};
use rmcp::model::CallToolRequestParams;
use rmcp::service::RunningService;
use rmcp::transport::TokioChildProcess;
use rmcp::{RoleClient, ServiceExt};
use serde_json::{Value, json};

type Client = RunningService<RoleClient, ()>;

async fn start(source: &Path) -> Client {
    let mut command = tokio::process::Command::new(env!("CARGO_BIN_EXE_agent-inbox"));
    // No variable of the test's environment, so none of its credentials reach the server.
    command.arg(source).env_clear();
    ().serve(TokioChildProcess::new(command).unwrap())
        .await
        .unwrap()
}

/// A source in a temporary directory and a server started on it.
async fn started() -> (tempfile::TempDir, Source, Client) {
    let dir = tempfile::tempdir().unwrap();
    let source = Source::open(dir.path()).unwrap();
    let client = start(dir.path()).await;
    (dir, source, client)
}

/// The text of an `inbox` call's result.
async fn inbox(client: &Client) -> String {
    call(client, "inbox", json!({})).await
}

/// The text of the result of the call of `tool` with `arguments`.
async fn call(client: &Client, tool: &'static str, arguments: Value) -> String {
    let Value::Object(arguments) = arguments else {
        panic!("arguments are an object")
    };
    let result = client
        .call_tool(CallToolRequestParams::new(tool).with_arguments(arguments))
        .await
        .unwrap();
    assert_eq!(result.is_error, Some(false));
    let [content] = result.content.as_slice() else {
        panic!("one content block: {result:?}")
    };
    content.as_text().expect("text content").text.clone()
}

#[tokio::test]
async fn lists_the_inbox_tool() {
    let dir = tempfile::tempdir().unwrap();
    let client = start(dir.path()).await;
    let info = client.peer_info().expect("the server initialized");
    assert!(info.capabilities.tools.is_some(), "{info:?}");
    let tools = client.list_all_tools().await.unwrap();
    let names: Vec<_> = tools.iter().map(|tool| tool.name.as_ref()).collect();
    assert_eq!(names, ["inbox", "ack", "reply"]);
    client.cancel().await.unwrap();
}

#[tokio::test]
async fn inbox_returns_each_waiting_message_once() {
    let (_dir, source, client) = started().await;
    assert_eq!(inbox(&client).await, "No messages are waiting.");

    let first = source.add("reviewer of alice", "Tests pass.").unwrap();
    let second = source.add("planner of bob", "Take 4.2 next.").unwrap();
    let text = inbox(&client).await;
    assert_carries_once(&text, &[first.clone(), second.clone()]);
    assert!(
        text.find(&first.msg_id) < text.find(&second.msg_id),
        "oldest first: {text}"
    );
    wait_handed_over(&source, &first.msg_id);
    wait_handed_over(&source, &second.msg_id);

    assert_eq!(inbox(&client).await, "No messages are waiting.");
    let third = source.add("reviewer of alice", "Merged.").unwrap();
    let text = inbox(&client).await;
    assert!(text.contains(&third.msg_id), "{text}");
    assert!(
        !text.contains(&first.msg_id) && !text.contains(&second.msg_id),
        "{text}"
    );
    client.cancel().await.unwrap();
}

#[tokio::test]
async fn two_sessions_on_one_source_get_each_message_once() {
    let dir = tempfile::tempdir().unwrap();
    let source = Source::open(dir.path()).unwrap();
    let sessions = [start(dir.path()).await, start(dir.path()).await];
    let added: Vec<_> = (0..20)
        .map(|i| source.add("alice", &format!("message {i}")).unwrap().msg_id)
        .collect();

    let (a, b) = tokio::join!(inbox(&sessions[0]), inbox(&sessions[1]));
    for msg_id in &added {
        let count = a.matches(msg_id.as_str()).count() + b.matches(msg_id.as_str()).count();
        assert_eq!(count, 1, "{msg_id} in:\n{a}\n{b}");
        wait_handed_over(&source, msg_id);
    }
    for session in sessions {
        session.cancel().await.unwrap();
    }
}

#[tokio::test]
async fn messages_of_a_server_that_died_are_handed_over_again() {
    let dir = tempfile::tempdir().unwrap();
    let source = Source::open(dir.path()).unwrap();
    // A server that took a message and died before writing its result leaves its claim.
    let crashed = Source::open(dir.path()).unwrap();
    let message = source.add("alice", "hi").unwrap();
    assert_eq!(crashed.take().unwrap(), vec![message.clone()]);
    drop(crashed);

    let client = start(dir.path()).await;
    let text = inbox(&client).await;
    assert!(text.contains(&message.msg_id), "{text}");
    wait_handed_over(&source, &message.msg_id);
    client.cancel().await.unwrap();
}

/// The signals `source` recorded for `msg_id`, with their details.
fn signals(source: &Source, msg_id: &str) -> Vec<(String, Option<String>)> {
    source
        .records(msg_id)
        .unwrap()
        .into_iter()
        .map(|record| (record.signal, record.detail))
        .collect()
}

#[tokio::test]
async fn ack_accepts_only_the_secret_a_session_was_handed() {
    let (_dir, source, client) = started().await;
    let message = source
        .add_with_ack("reviewer of alice", "Tests pass.")
        .unwrap();
    let id = message.msg_id.as_str();
    let secret = message.ack.clone().unwrap();
    // Not handed over yet: no session has seen it.
    let early = call(
        &client,
        "ack",
        json!({ "acks": [{ "msg_id": id, "ack": secret }] }),
    )
    .await;
    assert_eq!(
        early,
        format!("Not acknowledged {id}: the message was not handed over.")
    );

    let text = inbox(&client).await;
    assert!(text.contains(&format!("ack=\"{secret}\"")), "{text}");
    assert!(
        text.contains("Confirm each one you read with the ack tool"),
        "{text}"
    );
    wait_handed_over(&source, id);

    // Acks a session that never saw the message would send: injected.
    let injected = call(
        &client,
        "ack",
        json!({ "acks": [
            { "msg_id": id },
            { "msg_id": id, "ack": "0".repeat(32) },
            { "msg_id": "01a1cebe-0000-7000-8000-000000000000", "ack": secret },
            { "msg_id": "../handed", "ack": secret },
        ] }),
    )
    .await;
    assert_eq!(
        injected,
        format!(
            "Not acknowledged {id}: no ack given.\nNot acknowledged {id}: the ack is not the \
             message's.\nNot acknowledged 01a1cebe-0000-7000-8000-000000000000: no such \
             message.\nNot acknowledged ../handed: no such message."
        )
    );
    // The first refusal of a message the source has stays recorded.
    assert_eq!(
        signals(&source, id),
        [(
            "ack-refused".to_owned(),
            Some("the message was not handed over".to_owned())
        )]
    );

    let accepted = call(
        &client,
        "ack",
        json!({ "acks": [{ "msg_id": id, "ack": secret }] }),
    )
    .await;
    assert_eq!(accepted, format!("Acknowledged {id}."));
    assert_eq!(
        signals(&source, id)[1],
        ("ack".to_owned(), Some("ack".to_owned()))
    );
    client.cancel().await.unwrap();
}

#[tokio::test]
async fn any_call_acks_the_messages_it_lists_as_seen_and_a_reply_confirms_its_message() {
    let (_dir, source, client) = started().await;
    let first = source.add_with_ack("alice", "one").unwrap();
    let second = source.add_with_ack("bob", "two").unwrap();
    inbox(&client).await;
    wait_handed_over(&source, &second.msg_id);
    let third = source.add_with_ack("carol", "three").unwrap();
    let seen =
        |message: &agent_source::Message| json!({ "msg_id": message.msg_id, "ack": message.ack });

    // Two acks ride on the next inbox call, which hands over the third message.
    let text = call(
        &client,
        "inbox",
        json!({ "seen": [seen(&first), seen(&second)] }),
    )
    .await;
    let (acks, rest) = text.split_once("\n1 message(s)").unwrap();
    assert_eq!(
        acks,
        format!(
            "Acknowledged {}.\nAcknowledged {}.",
            first.msg_id, second.msg_id
        )
    );
    assert!(rest.contains(&third.msg_id), "{text}");
    wait_handed_over(&source, &third.msg_id);
    for message in [&first, &second] {
        assert_eq!(
            signals(&source, &message.msg_id),
            [("ack".to_owned(), Some("inbox".to_owned()))]
        );
    }

    // A reply confirms the message it answers; a reply with a wrong secret is refused.
    let wrong = call(
        &client,
        "reply",
        json!({ "reply_to": third.msg_id, "ack": first.ack, "text": "No." }),
    )
    .await;
    assert_eq!(
        wrong,
        format!(
            "Not replied to {}: the ack is not the message's.",
            third.msg_id
        )
    );
    let replied = call(
        &client,
        "reply",
        json!({ "reply_to": third.msg_id, "ack": third.ack, "text": "Done.", "seen": [seen(&first)] }),
    )
    .await;
    assert_eq!(
        replied,
        format!(
            "Acknowledged {}.\nReplied to {}.",
            first.msg_id, third.msg_id
        )
    );
    assert_eq!(
        signals(&source, &third.msg_id),
        [
            (
                "ack-refused".to_owned(),
                Some("the ack is not the message's".to_owned())
            ),
            ("reply".to_owned(), Some("Done.".to_owned())),
        ]
    );
    let error = client
        .call_tool(CallToolRequestParams::new("outbox"))
        .await
        .unwrap_err();
    assert!(
        error.to_string().contains("no tool named outbox"),
        "{error}"
    );
    client.cancel().await.unwrap();
}
