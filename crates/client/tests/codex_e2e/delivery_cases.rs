use std::fs;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Condvar, Mutex};
use std::thread;
use std::time::{Duration, Instant};

use serde_json::{Value, json};

use crate::delivery::{
    BATCH, INLINE, PULL, assert_wakes, bob_joins_other_channel, other_channel, set_mode, set_text,
};
use crate::hooks::Bob;
use crate::host::woken;
use crate::receipts::{shows_no_read, shows_read_once};
use crate::senders::{OWN, PEER};
use crate::session::{
    QUIET, READY, SIGNAL, Session, SessionOptions, after_prompt, called_after, codex_path, command,
    inbox, input_items, user_text,
};
use crate::stub::{ResponsesReply, Stub, output_of, played};
use crate::talk::{delivered, received_until};

fn tool_result(body: &Value, text: &str) -> bool {
    (0..played(body)).any(|step| output_of(body, step).is_some_and(|output| output.contains(text)))
}

fn assert_private(model: &Stub, text: &str) {
    assert!(
        model.requests().iter().any(|body| tool_result(body, text)),
        "inbox did not return the message"
    );
    for mut body in model.requests() {
        if let Some(items) = body["input"].as_array_mut() {
            let calls: Vec<Value> = items
                .iter()
                .filter(|item| {
                    item["type"] == "function_call"
                        && item["namespace"] == "mcp__skald"
                        && item["name"] == "inbox"
                })
                .map(|item| item["call_id"].clone())
                .collect();
            items.retain(|item| {
                item["type"] != "function_call_output" || !calls.contains(&item["call_id"])
            });
        }
        assert!(
            !body.to_string().contains(text),
            "message appeared outside the inbox result"
        );
    }
    model.assert_no_foreign_credential();
}

fn assert_queue_private(session: &Session, text: &str) {
    let db = session.live.home().join("queue_1.sqlite");
    assert!(db.is_file(), "Codex did not create its queue");
    for file in [db.clone(), db.with_file_name("queue_1.sqlite-wal")] {
        let bytes = match fs::read(file) {
            Ok(bytes) => bytes,
            Err(err) if err.kind() == std::io::ErrorKind::NotFound => continue,
            Err(err) => panic!("read the fixture queue: {err}"),
        };
        assert!(
            !bytes
                .windows(text.len())
                .any(|window| window == text.as_bytes()),
            "queued the message text"
        );
    }
}

fn message_script(body: &Value) -> ResponsesReply {
    if woken(body) && !called_after(body, SIGNAL) {
        return inbox();
    }
    if user_text(body).contains("Check the fixture inbox.")
        && !called_after(body, "Check the fixture inbox.")
    {
        return inbox();
    }
    ResponsesReply::Done
}

#[test]
fn an_idle_codex_gets_only_a_queue_signal_even_under_inline() {
    let Some(codex) = codex_path() else { return };
    let session = Session::start(
        5_340_100_001,
        "ada-nightly-codex-idle",
        codex,
        Stub::start_responses(message_script),
        SessionOptions::default(),
    );
    session.wait_idle(0);
    let solo = &session.live.codex.web.solo;
    let bob = Bob::join(solo, 5_340_100_002, "bob-nightly-codex-idle");
    let channel = solo.default_channel();
    set_text(solo, channel, PEER, INLINE);
    let token = "fixture-idle-peer-9871";

    let start = session.model().requests().len();
    let msg = session.send_in(channel, Some(&bob), token);
    let wake = session.wait_request(start, woken);
    assert!(!wake.to_string().contains(token));
    session.wait_request(start, |body| tool_result(body, token));
    shows_read_once(&bob.person.machine, msg);

    assert_private(session.model(), token);
    assert_queue_private(&session, token);
    assert_wakes(solo, channel, 1);
}

#[derive(Clone, Default)]
struct Gate(Arc<(Mutex<bool>, Condvar)>);

impl Gate {
    fn await_release(&self) {
        let (ready, changed) = &*self.0;
        let (ready, _) = changed
            .wait_timeout_while(
                ready.lock().expect("gate"),
                Duration::from_secs(60),
                |ready| !*ready,
            )
            .expect("gate wait");
        assert!(*ready, "test did not release the fixture model");
    }

    fn release(&self) {
        let (ready, changed) = &*self.0;
        *ready.lock().expect("gate") = true;
        changed.notify_all();
    }
}

fn busy_hook(id: u64, login: &str, token: &str, after_tool: bool) {
    let Some(codex) = codex_path() else { return };
    let gate = Gate::default();
    let model_gate = gate.clone();
    let stub = Stub::start_responses(move |body| {
        if body.to_string().contains(SIGNAL) && played(body) == usize::from(after_tool) {
            inbox()
        } else if user_text(body).contains(READY) && played(body) == 0 {
            model_gate.await_release();
            if after_tool {
                command("echo fixture-tool-finished", json!({}))
            } else {
                ResponsesReply::Done
            }
        } else {
            ResponsesReply::Done
        }
    });
    let session = Session::start(id, login, codex, stub, SessionOptions::CAPTURE);
    session.wait_request(0, |body| user_text(body).contains(READY));
    let solo = &session.live.codex.web.solo;
    let bob = Bob::join(solo, id + 1, &format!("{login}-peer"));
    let channel = solo.default_channel();
    set_text(solo, channel, PEER, INLINE);
    let text = if after_tool {
        format!("{token} {}", "boundary ".repeat(420))
    } else {
        token.to_owned()
    };
    let msg = session.send_in(channel, Some(&bob), &text);
    // Keep the turn busy until the receiving device has recorded the message.
    received_until(&bob.person.machine, |shown| {
        delivered(shown, msg, solo.talk.ada.device) > 0
    });
    gate.release();

    let expected_event = if after_tool { "PostToolUse" } else { "Stop" };
    let bytes = session.wait_hook_stdout(expected_event);
    assert_hook_output(expected_event, &bytes, token);
    assert!(
        String::from_utf8_lossy(&bytes).contains(SIGNAL),
        "{expected_event} printed no signal"
    );

    let signal = session.wait_request(0, |body| {
        if after_tool {
            tool_result(body, "fixture-tool-finished")
        } else {
            body.to_string().contains(SIGNAL)
        }
    });
    assert!(
        signal.to_string().contains(SIGNAL),
        "the busy hook gave no signal"
    );
    assert!(!signal.to_string().contains(token));
    if after_tool {
        assert!(
            !user_text(&signal).contains(SIGNAL),
            "a queue turn supplied the signal while the tool ran"
        );
    } else {
        assert_queue_private(&session, SIGNAL);
    }
    let supplied = input_items(&signal)
        .iter()
        .find(|item| item.to_string().contains(SIGNAL))
        .expect("hook output");
    assert!(
        supplied.to_string().len() <= 2_500,
        "hook context exceeded the limit"
    );
    session.wait_request(0, |body| tool_result(body, &text));
    shows_read_once(&bob.person.machine, msg);
    assert_private(session.model(), token);

    for (event, bytes) in session.hook_stdout() {
        assert_hook_output(&event, &bytes, token);
    }
}

fn assert_hook_output(event: &str, bytes: &[u8], token: &str) {
    assert!(
        bytes.len() <= 2_500,
        "{event} raw stdout exceeded 2,500 bytes"
    );
    assert!(
        !String::from_utf8_lossy(bytes).contains(token),
        "{event} printed the message outside inbox"
    );
}

#[test]
fn a_busy_codex_gets_a_bounded_post_tool_use_signal() {
    busy_hook(
        5_340_101_001,
        "ada-nightly-codex-tool",
        "fixture-post-tool-5619",
        true,
    );
}

#[test]
fn stop_continues_a_busy_codex_with_a_bounded_signal() {
    busy_hook(
        5_340_102_001,
        "ada-nightly-codex-stop",
        "fixture-stop-7613",
        false,
    );
}

#[test]
fn with_no_message_codex_hooks_supply_no_context_and_do_not_continue_the_turn() {
    let Some(codex) = codex_path() else { return };
    let stub = Stub::start_responses(|body| {
        if user_text(body).contains(READY) && played(body) == 0 {
            command("echo fixture-quiet", json!({}))
        } else {
            ResponsesReply::Done
        }
    });
    let session = Session::start(
        5_340_103_001,
        "ada-nightly-codex-quiet",
        codex,
        stub,
        SessionOptions::CAPTURE,
    );
    session.wait_idle(0);
    let before = session.model().requests().len();
    thread::sleep(QUIET);
    assert_eq!(session.completions(), 1);
    assert_eq!(
        session.model().requests().len(),
        before,
        "an empty Stop continued the turn"
    );
    assert!(
        session
            .model()
            .requests()
            .iter()
            .all(|body| !body.to_string().contains(SIGNAL))
    );
    let outputs = session.hook_stdout();
    for expected in ["SessionStart", "UserPromptSubmit", "PostToolUse", "Stop"] {
        assert!(
            outputs.iter().any(|(event, _)| event == expected),
            "{expected} did not run. Hook wrapper statuses: {:?}",
            session.hook_diagnostics()
        );
    }
    for (event, bytes) in outputs {
        assert!(
            bytes.is_empty(),
            "{event} printed {bytes:?} with nothing waiting"
        );
    }
    assert_wakes(
        &session.live.codex.web.solo,
        session.live.codex.web.solo.default_channel(),
        0,
    );
    session.model().assert_no_foreign_credential();
}

fn retry_has_signal(body: &Value) -> bool {
    user_text(body).contains("Retry the fixture.")
        && after_prompt(body, "Retry the fixture.")
            .iter()
            .any(|item| item.to_string().contains(SIGNAL))
}

#[test]
fn a_failed_signal_turn_retries_by_the_next_prompt_and_read_waits_for_confirmation() {
    let Some(codex) = codex_path() else { return };
    let gate = Gate::default();
    let model_gate = gate.clone();
    let failed = Arc::new(AtomicBool::new(false));
    let model_failed = Arc::clone(&failed);
    let stub = Stub::start_responses(move |body| {
        if woken(body) {
            model_failed.store(true, Ordering::Release);
            return ResponsesReply::Fail;
        }
        if !retry_has_signal(body) {
            return ResponsesReply::Done;
        }
        if !called_after(body, "Retry the fixture.") {
            return inbox();
        }
        model_gate.await_release();
        ResponsesReply::Done
    });
    let session = Session::start(
        5_340_104_001,
        "ada-nightly-codex-retry",
        codex,
        stub,
        SessionOptions {
            resend: Some(10),
            ..SessionOptions::CAPTURE
        },
    );
    session.wait_idle(0);
    let token = "fixture-retry-4078";
    let start = session.model().requests().len();
    let msg = session.send(token);
    session.wait_request(start, woken);
    let first_signal = Instant::now();
    let deadline = first_signal + Duration::from_secs(10);
    while !failed.load(Ordering::Acquire) {
        assert!(
            Instant::now() < deadline,
            "fixture model did not fail the signal turn"
        );
        thread::sleep(Duration::from_millis(100));
    }
    session.wait_idle(1);
    shows_no_read(session.sender(), msg, "the signal turn failed");
    thread::sleep(Duration::from_secs(10).saturating_sub(first_signal.elapsed()));
    let retry = session.model().requests().len();
    session.prompt("Retry the fixture.");
    let bytes = session.wait_hook_stdout("UserPromptSubmit");
    assert_hook_output("UserPromptSubmit", &bytes, token);
    assert!(String::from_utf8_lossy(&bytes).contains(SIGNAL));
    let signal = session.wait_request(retry, retry_has_signal);
    assert!(!signal.to_string().contains(token) || tool_result(&signal, token));
    session.wait_request(retry, |body| tool_result(body, token));
    shows_no_read(
        session.sender(),
        msg,
        "the model has not confirmed the inbox result",
    );
    gate.release();
    shows_read_once(session.sender(), msg);
    assert_private(session.model(), token);
}

#[test]
fn codex_applies_batch_and_pull_per_channel_and_trust_level() {
    let Some(codex) = codex_path() else { return };
    let session = Session::start(
        5_340_105_001,
        "ada-nightly-codex-modes",
        codex,
        Stub::start_responses(message_script),
        SessionOptions::default(),
    );
    session.wait_idle(0);
    let solo = &session.live.codex.web.solo;
    let main = solo.default_channel();
    let other = other_channel(solo);
    let bob = Bob::join(solo, 5_340_105_002, "bob-nightly-codex-modes");
    bob_joins_other_channel(solo, &bob);

    let window = Duration::from_secs(30);
    set_mode(solo, main, PEER, BATCH, Some("30s"));
    let batch_start = Instant::now();
    let batched: Vec<String> = (0..10).map(|n| format!("fixture-batch-{n:02}")).collect();
    for text in &batched {
        session.send_in(main, Some(&bob), text);
    }
    assert!(
        batch_start.elapsed() < window,
        "fixture sends missed the batch window"
    );
    assert_wakes(solo, main, 0);
    assert_wakes(solo, other, 0);
    let start = session.model().requests().len();
    session.wait_request(start, |body| {
        woken(body) && batched.iter().all(|text| tool_result(body, text))
    });
    assert!(
        batch_start.elapsed() >= window,
        "batch woke before the window elapsed"
    );
    assert_wakes(solo, main, 1);
    session.wait_idle(1);

    set_mode(solo, main, PEER, PULL, None);
    let pull = ["fixture-pull-one-256", "fixture-pull-two-257"];
    for text in pull {
        session.send_in(main, Some(&bob), text);
    }
    thread::sleep(QUIET);
    assert_wakes(solo, main, 1);
    let pull_start = session.model().requests().len();
    session.prompt("Check the fixture inbox.");
    session.wait_request(pull_start, |body| {
        pull.iter().all(|text| tool_result(body, text))
    });
    assert_wakes(solo, main, 1);
    session.wait_idle(2);

    for (channel, peer, text, count) in [
        (main, None, "fixture-now-own-768", 2),
        (other, Some(&bob), "fixture-now-other-channel-769", 1),
    ] {
        let start = session.model().requests().len();
        let completed = session.completions();
        session.send_in(channel, peer, text);
        session.wait_request(start, |body| woken(body) && tool_result(body, text));
        assert_wakes(solo, channel, count);
        session.wait_idle(completed);
    }
    for text in batched.iter().map(String::as_str).chain(pull) {
        assert_private(session.model(), text);
    }
    assert_wakes(solo, main, 2);
    assert_wakes(solo, other, 1);
    assert_eq!(
        crate::delivery::mode_of(
            crate::delivery::of_channel(&crate::delivery::listed(solo), main),
            OWN
        )
        .0,
        "now"
    );
}
