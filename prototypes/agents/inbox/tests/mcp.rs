//! An MCP client starts `agent-inbox` over stdio and calls `inbox`.

use std::path::Path;
use std::time::{Duration, Instant};

use agent_source::{Source, State};
use rmcp::model::CallToolRequestParams;
use rmcp::service::RunningService;
use rmcp::transport::TokioChildProcess;
use rmcp::{RoleClient, ServiceExt};

type Client = RunningService<RoleClient, ()>;

async fn start(source: &Path) -> Client {
    let mut command = tokio::process::Command::new(env!("CARGO_BIN_EXE_agent-inbox"));
    command.arg(source);
    ().serve(TokioChildProcess::new(command).unwrap())
        .await
        .unwrap()
}

/// The text of an `inbox` call's result.
async fn inbox(client: &Client) -> String {
    let result = client
        .call_tool(CallToolRequestParams::new("inbox"))
        .await
        .unwrap();
    assert_eq!(result.is_error, Some(false));
    let [content] = result.content.as_slice() else {
        panic!("one content block: {result:?}")
    };
    content.as_text().expect("text content").text.clone()
}

/// Waits until the server marks `msg_id` handed over: it does so after writing the result.
async fn handed_over(source: &Source, msg_id: &str) {
    let start = Instant::now();
    while !matches!(
        source.state(msg_id).unwrap(),
        Some(State::HandedOver { .. })
    ) {
        assert!(
            start.elapsed() < Duration::from_secs(10),
            "{msg_id} is not marked handed over: {:?}",
            source.state(msg_id).unwrap()
        );
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
}

#[tokio::test]
async fn lists_the_inbox_tool() {
    let dir = tempfile::tempdir().unwrap();
    let client = start(dir.path()).await;
    let info = client.peer_info().expect("the server initialized");
    assert!(info.capabilities.tools.is_some(), "{info:?}");
    let tools = client.list_all_tools().await.unwrap();
    let names: Vec<_> = tools.iter().map(|tool| tool.name.as_ref()).collect();
    assert_eq!(names, ["inbox"]);
    client.cancel().await.unwrap();
}

#[tokio::test]
async fn inbox_returns_each_waiting_message_once() {
    let dir = tempfile::tempdir().unwrap();
    let source = Source::open(dir.path()).unwrap();
    let client = start(dir.path()).await;
    assert_eq!(inbox(&client).await, "No messages are waiting.");

    let first = source.add("reviewer of alice", "Tests pass.").unwrap();
    let second = source.add("planner of bob", "Take 4.2 next.").unwrap();
    let text = inbox(&client).await;
    for message in [&first, &second] {
        let element = format!(
            "<message msg_id=\"{}\" from=\"{}\">\n{}\n</message>",
            message.msg_id, message.sender, message.text
        );
        assert_eq!(text.matches(&element).count(), 1, "{text}");
        assert_eq!(text.matches(&message.msg_id).count(), 1, "{text}");
    }
    assert!(
        text.find(&first.msg_id) < text.find(&second.msg_id),
        "oldest first: {text}"
    );
    handed_over(&source, &first.msg_id).await;
    handed_over(&source, &second.msg_id).await;

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
        handed_over(&source, msg_id).await;
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
    handed_over(&source, &message.msg_id).await;
    client.cancel().await.unwrap();
}
