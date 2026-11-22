//! The line on the paths the acceptance tests leave open: `skald link`, `skald held` and `skald
//! fingerprint` at a terminal; `--json` of `skald reply`, `skald message-status` and `skald
//! delivery-modes`; an MCP `inbox` that holds messages; and the person's `skald recv --wait`, which
//! ends on the line as on a warning.

use std::thread;
use std::time::{Duration, Instant};

use serde_json::Value;

use crate::agents::{Agents, CLAUDE};
use crate::kit::{RECV, received};
use crate::lines::{JSON_FLAG, json_objects};
use crate::machine::{Finished, Machine, Run};
use crate::mcp_client::{CLIENT, MCP, Session};
use crate::talk::Line;
use crate::terminal::Terminal;
use crate::waits::{AT_ONCE, WAIT_FLAG, Waits};
use crate::world::{ACME, AGENT, run};
use crate::{SHOWN_BY, assert_says_none, assert_says_waiting, bobs_two_masters, numbers_in};

/// The commands these tests run beside those of the acceptance tests.
const LINK: &str = "link";
const HELD: &str = "held";
const FINGERPRINT: &str = "fingerprint";
const REPLY: &str = "reply";
const MESSAGE_STATUS: &str = "message-status";
const DELIVERY_MODES: &str = "delivery-modes";

/// What `skald fingerprint` asks its person at a terminal ends with.
const YES_NO: &str = "[yes/no]";

/// Fails unless `shown` holds the line once, saying one key change waits.
#[track_caller]
fn says_one(shown: &str) {
    assert_says_waiting(shown, Some(1));
}

/// One key change waits on Ada's machine. `skald link` and `skald held` from a pipe, which refuse,
/// and `skald fingerprint acme <bob>` at a terminal, answered no, each print the line once.
#[test]
fn link_held_and_fingerprint_say_how_many_key_changes_wait() {
    let agents = Agents::new(
        [4_188_951_001, 4_188_951_002],
        ["ada-links-with-a-key-change", "bob-two-masters-beside-link"],
    );
    bobs_two_masters(&agents);
    let machine = &agents.talk.ada.machine;
    let bob = agents.talk.played[0].person.handle.to_string();

    for args in [[LINK], [HELD]] {
        says_one(&run(machine, &args).shown());
    }
    let mut terminal = Terminal::start(machine, &[FINGERPRINT, ACME, &bob]);
    terminal.wait_to_show(YES_NO);
    terminal.type_line("no");
    let shown = terminal.finish();
    assert!(shown.success, "{}", shown.text);
    says_one(&shown.text);
}

/// Fails unless `finished`, of `what`, succeeded, printed no line on its standard error and holds
/// only JSON objects on its standard output, one of which holds the command that shows the key
/// changes and, as a word, that one waits.
#[track_caller]
fn carries_one(what: &str, finished: &Finished) {
    assert!(finished.success, "{what}: {finished:?}");
    assert_says_none(&finished.stderr);
    let carrying: Vec<Value> = (json_objects(&finished.stdout).into_iter())
        .filter(|object| object.to_string().contains(SHOWN_BY))
        .collect();
    let [object] = carrying.as_slice() else {
        panic!(
            "{what}: not one object of the key changes: {}",
            finished.stdout
        );
    };
    let numbers = numbers_in(&object.to_string());
    assert!(numbers.contains(&1), "{what}: {object}");
}

/// One key change waits on Ada's machine, and Bob's device sent a message her `skald recv` took
/// in. Her `skald reply --json` to it, `skald message-status --json` of her reply and `skald
/// delivery-modes --json` carry the key change waiting in their JSON.
#[test]
fn json_of_reply_status_and_delivery_modes_carries_how_many_key_changes_wait() {
    let mut agents = Agents::new(
        [4_188_952_001, 4_188_952_002],
        ["ada-replies-in-json", "bob-two-masters-replied-to"],
    );
    bobs_two_masters(&agents);
    let bob = agents.bob();
    let asked = agents.bob_sends(&Line::new(&bob, 1, "key changes test: Bob asks"));
    let machine = &agents.talk.ada.machine;
    received(machine);

    let asked = asked.to_string();
    let replied = run(
        machine,
        &[REPLY, JSON_FLAG, &asked, "key changes test: Ada replies"],
    );
    carries_one("skald reply --json", &replied);
    let reply = (json_objects(&replied.stdout).into_iter())
        .find_map(|object| object["msg_id"].as_str().map(str::to_owned))
        .unwrap_or_else(|| panic!("no msg_id of the reply: {}", replied.stdout));
    let status = run(machine, &[MESSAGE_STATUS, JSON_FLAG, &reply]);
    carries_one("skald message-status --json", &status);
    carries_one(
        "skald delivery-modes --json",
        &run(machine, &[DELIVERY_MODES, JSON_FLAG]),
    );
}

/// A session of `skald mcp` as claude in `~/work/acme` starts; then one key change comes to wait on
/// Ada's machine, and Bob's device sends a message to the channel. The session's `inbox`, which
/// holds that message, says once that one key change waits.
#[test]
fn an_inbox_with_messages_says_how_many_key_changes_wait() {
    let mut agents = Agents::new(
        [4_188_953_001, 4_188_953_002],
        ["ada-inbox-with-a-key-change", "bob-two-masters-writes-in"],
    );
    agents.run_as(CLAUDE);
    let machine = &agents.talk.ada.machine;
    let mut session = Session::start(
        &mut machine.command_in(&agents.folder, &[AGENT, CLAUDE, MCP]),
        CLIENT,
    );
    bobs_two_masters(&agents);
    let bob = agents.bob();
    let text = "key changes test: a message in the inbox";
    agents.bob_sends(&Line::new(&bob, 1, text));

    let inbox = session.inbox();

    assert!(inbox.contains(text), "{inbox}");
    says_one(&inbox);
}

/// Fails unless `run`, the person's `skald recv --wait`, prints the line once, saying one key
/// change waits, and then ends with success within [`AT_ONCE`].
#[track_caller]
fn ends_on_the_line(mut run: Run) {
    run.wait_to_show(&[SHOWN_BY]);
    let until = Instant::now() + AT_ONCE;
    while !run.exited() {
        if Instant::now() > until {
            panic!("still waits after the line: {:?}", run.kill());
        }
        thread::sleep(Duration::from_millis(50));
    }
    let finished = run.finish();
    assert!(finished.success, "{finished:?}");
    says_one(&finished.shown());
}

/// One key change waits on Ada's machine. Her `skald recv --wait`, one-shot, then through a
/// helper, prints the line and ends, as on a warning.
#[test]
fn the_persons_waiting_recv_ends_on_the_line() {
    let waits = Waits::new(
        [4_188_954_001, 4_188_954_002],
        ["ada-wait-ends-on-a-key-change", "bob-two-masters-ends-wait"],
    );
    bobs_two_masters(&waits.agents);
    let machine: &Machine = &waits.agents.talk.ada.machine;

    ends_on_the_line(machine.start(&[RECV, WAIT_FLAG]));
    ends_on_the_line(waits.helped.start(&[], &[RECV, WAIT_FLAG]));
}
