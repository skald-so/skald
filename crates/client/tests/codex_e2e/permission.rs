use std::thread;
use std::time::{Duration, Instant};

use serde_json::{Value, json};

use crate::session::{PermissionPath, Session, called_after, codex_path, command, user_text};
use crate::stub::{ResponsesReply, Stub};

const CHECK: &str = "Show the fixture approval dialog.";
const COMMAND: &str = "echo fixture-approval-witness";

fn records(session: &Session) -> Vec<Value> {
    let run = session
        .sender()
        .start_in(session.live.dir(), &["recv", "--json"])
        .finish();
    assert!(run.success, "{run:?}");
    run.stdout
        .lines()
        .filter_map(|line| serde_json::from_str::<Value>(line).ok())
        .filter(|value| value["type"] == "permission_prompt")
        .collect()
}

fn detects_prompt(id: u64, login: &str, path: PermissionPath) {
    let Some(codex) = codex_path() else { return };
    let model = Stub::start_responses(|body| {
        if user_text(body).contains(CHECK) && !called_after(body, CHECK) {
            command(
                COMMAND,
                json!({"sandbox_permissions":"require_escalated", "justification":"Show a disposable approval dialog."}),
            )
        } else {
            ResponsesReply::Done
        }
    });
    let hook = matches!(path, PermissionPath::Hook);
    let mut session = Session::capturing(id, login, codex, model, Some(path), None);
    session.wait_idle(0);
    // The initial turn gives the helper a registered thread before any approval is requested.
    thread::sleep(Duration::from_secs(1));
    assert!(
        records(&session).is_empty(),
        "there is no approval dialog yet"
    );
    session.prompt(CHECK);
    session
        .terminal
        .wait_to_show("Would you like to run the following command");
    let shown = Instant::now();
    let pending = loop {
        let pending = records(&session);
        assert!(
            shown.elapsed() < Duration::from_secs(5),
            "Skald did not report the approval prompt"
        );
        if !pending.is_empty() {
            break pending;
        }
        thread::sleep(Duration::from_millis(100));
    };
    let [prompt] = pending.as_slice() else {
        panic!("expected one pending approval: {pending:?}");
    };
    assert!(prompt["session"].as_str().is_some_and(|id| !id.is_empty()));
    assert!(prompt["timestamp"].is_number());
    assert!(prompt["ended"].is_null());
    assert!(
        prompt["agent"]
            .as_str()
            .is_some_and(|agent| session.live.agent().ends_with(agent))
    );
    assert!(
        !prompt.to_string().contains(COMMAND),
        "Skald printed the command"
    );

    let agent = session
        .sender()
        .start_in(
            session.live.dir(),
            &["--agent", "observer", "recv", "--json"],
        )
        .finish();
    assert!(agent.success, "{agent:?}");
    assert!(!agent.stdout.contains("permission_prompt"), "{agent:?}");

    session.terminal.send(b"\x1b");
    session.terminal.wait_to_show("You canceled the request");
    session.wait_idle(1);
    // In the hook path a decline is observed on the next turn.
    session.prompt("Continue the fixture after declining.");
    session.wait_idle(2);
    assert!(
        records(&session).is_empty(),
        "the approval was reported twice"
    );
    let observed: Vec<_> = session
        .hook_stdout()
        .into_iter()
        .filter(|(event, _)| event == "PermissionRequest")
        .collect();
    assert_eq!(observed.len(), usize::from(hook), "permission hook calls");
    assert!(
        observed.iter().all(|(_, bytes)| bytes.is_empty()),
        "permission hook printed output"
    );
    session.model().assert_no_foreign_credential();
}

#[test]
fn the_permission_hook_reports_a_local_codex_approval() {
    detects_prompt(
        5_340_106_001,
        "ada-nightly-codex-permission",
        PermissionPath::Hook,
    );
}

#[cfg(unix)]
#[test]
fn the_background_app_server_reports_a_local_codex_approval() {
    detects_prompt(
        5_340_106_002,
        "ada-nightly-codex-daemon",
        PermissionPath::Daemon,
    );
}
