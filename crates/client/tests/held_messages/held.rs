//! What the tests share: the person's decisions at a terminal, the line of the person's
//! `skald recv` that says how many messages wait, what the sending device shows of a status, and
//! the receipts C reads.

use std::{
    path::Path,
    thread,
    time::{Duration, Instant},
};

use skald_proto::{MessageId, ReceiptStatus};

use crate::agents::CLAUDE;
use crate::cloud::WAIT;
use crate::inbound::claude_set_to;
use crate::kit::received;
use crate::lines::{agent_line, assert_no_uuid, lines_referring_to};
use crate::machine::Machine;
use crate::receipts::{Got, Roles, assert_shows_once, shows_read_once};
use crate::senders::holds_word;
use crate::terminal::Terminal;

/// The mode that keeps a message for the person to decide.
pub const HOLD: &str = "hold";

/// The status the CLI shows while the `to` agent's device holds the message.
pub const HELD: &str = "held";

/// The command that lists the held messages and takes the person's decisions; the person's
/// `skald recv` names it on the line that says how many wait.
pub const DECIDE: &str = "held";
pub const DECIDE_LINE: &str = "skald held";

/// What the person types to accept a held message, or to refuse it.
pub const ACCEPTS: &str = "accept";
pub const REFUSES: &str = "refuse";

/// What a `refused` says when no one decided before the message's wait ended.
pub const NO_ONE_DECIDED: &str = "no one decided";

/// The person's decision on a held message.
#[derive(Debug, Clone, Copy)]
pub enum Decision<'a> {
    Accept,
    /// With the reason the person types; empty, none.
    Refuse(&'a str),
}

/// Types `decision` at `terminal`: `accept`, or `refuse` and the reason on the next line.
pub fn answer(terminal: &Terminal, decision: Decision) {
    match decision {
        Decision::Accept => terminal.type_line(ACCEPTS),
        Decision::Refuse(reason) => {
            terminal.type_line(REFUSES);
            terminal.type_line(reason);
        }
    }
}

/// Starts the person's `skald held` at a terminal on `machine` in `dir`.
pub fn deciding(machine: &Machine, dir: &Path) -> Terminal {
    Terminal::start_in(machine, dir, &[DECIDE])
}

/// Runs the person's `skald held` at a terminal on `machine` in `dir` and answers each message of
/// `decisions`, given by its text, as the run shows it: one at a time, in the order the run takes
/// them. Fails unless it shows each, never two undecided at once, and exits successfully once all
/// are decided. Gives what it showed.
#[track_caller]
pub fn decide(machine: &Machine, dir: &Path, decisions: &[(&str, Decision)]) -> String {
    let mut terminal = deciding(machine, dir);
    let mut left = decisions.to_vec();
    while !left.is_empty() {
        let shown = terminal.wait_for("a held message", |shown| {
            left.iter().any(|(text, _)| shown.contains(text))
        });
        let showing: Vec<(&str, Decision)> = (left.iter().copied())
            .filter(|(text, _)| shown.contains(text))
            .collect();
        let [(text, decision)] = showing.as_slice() else {
            panic!("not one undecided message at a time: {shown}");
        };
        answer(&terminal, *decision);
        let text = *text;
        left.retain(|(other, _)| *other != text);
    }
    let done = terminal.finish();
    assert!(done.success, "{}", done.text);
    done.text
}

/// Runs the person's `skald held` at a terminal on `machine` in `dir`; fails unless it exits
/// successfully, asking nothing, and shows none of `texts`.
#[track_caller]
pub fn nothing_to_decide(machine: &Machine, dir: &Path, texts: &[&str]) {
    let done = deciding(machine, dir).finish();
    assert!(done.success, "{}", done.text);
    for text in texts {
        assert!(
            !done.text.contains(text),
            "{text:?} still waits: {}",
            done.text
        );
    }
}

/// The lines of `shown`, a person's output, that say how many messages wait for a decision.
pub fn waiting_lines(shown: &str) -> Vec<&str> {
    (shown.lines())
        .filter(|line| line.contains(DECIDE_LINE))
        .collect()
}

/// Runs the person's `skald recv` on `machine`; fails unless it says, on one line, that `count`
/// messages wait for a decision. Gives what it showed.
#[track_caller]
pub fn says_waiting(machine: &Machine, count: usize) -> String {
    let shown = received(machine).shown();
    let lines = waiting_lines(&shown);
    let [line] = lines.as_slice() else {
        panic!("not one line of the messages waiting: {shown}");
    };
    assert!(
        holds_word(line, &count.to_string()),
        "not {count} waiting: {line}"
    );
    shown
}

/// Runs the person's `skald recv` on `machine`; fails if it says that messages wait for a
/// decision.
#[track_caller]
pub fn says_none_waiting(machine: &Machine, when: &str) {
    let shown = received(machine).shown();
    let lines = waiting_lines(&shown);
    assert!(lines.is_empty(), "{when}: {lines:?} in {shown}");
}

/// The lines of `shown`, a person's output, that hold `msg_id` and the word `status`.
pub fn status_lines(shown: &str, msg_id: MessageId, status: &str) -> Vec<String> {
    let id = msg_id.to_string();
    (shown.lines())
        .filter(|line| line.contains(&id) && holds_word(line, status))
        .map(str::to_owned)
        .collect()
}

/// The lines of `shown`, an agent's output, that say the word `status` of `msg_id` by its
/// reference and are no message.
pub fn agent_status_lines(shown: &str, msg_id: MessageId, status: &str) -> Vec<String> {
    let lines = lines_referring_to(shown, msg_id);
    (lines.into_iter())
        .filter(|line| holds_word(line, status) && agent_line(line).is_none())
        .collect()
}

/// Runs the person's `skald recv` on `machine` every half second, for up to [`WAIT`], until a run
/// shows `status` of `msg_id`; then once more. Fails unless the runs show it on one line in all,
/// the last one not at all. Gives that line.
#[track_caller]
pub fn shows_once(machine: &Machine, msg_id: MessageId, status: &str) -> String {
    let deadline = Instant::now() + WAIT;
    let mut seen = Vec::new();
    while seen.is_empty() {
        assert!(Instant::now() < deadline, "no {status} of {msg_id} shown");
        let shown = received(machine).shown();
        seen = status_lines(&shown, msg_id, status);
        if seen.is_empty() {
            thread::sleep(Duration::from_millis(500));
        }
    }
    let again = received(machine).shown();
    seen.extend(status_lines(&again, msg_id, status));
    let [line] = seen.as_slice() else {
        panic!("{status} of {msg_id} not on one line: {seen:?}");
    };
    line.clone()
}

/// Runs the person's `skald recv` on `machine`; fails if it shows `status` of `msg_id`.
#[track_caller]
pub fn shows_not(machine: &Machine, msg_id: MessageId, status: &str, when: &str) {
    let shown = received(machine).shown();
    let lines = status_lines(&shown, msg_id, status);
    assert!(lines.is_empty(), "{when}: {lines:?} in {shown}");
}

/// A's agent's next `skald recv` says `status` of `msg_id` on one line, with no UUID, and the one
/// after does not. Gives that line.
#[track_caller]
pub fn a_agent_says_once(roles: &Roles, msg_id: MessageId, status: &str) -> String {
    let first = roles.a_agent_reads(&[]).shown();
    let lines = agent_status_lines(&first, msg_id, status);
    let [line] = lines.as_slice() else {
        panic!("{status} of {msg_id} not on one line: {first}");
    };
    assert_no_uuid(&first);
    let then = roles.a_agent_reads(&[]).shown();
    let again = agent_status_lines(&then, msg_id, status);
    assert!(again.is_empty(), "{status} again: {then}");
    line.clone()
}

/// The receipts C read so far in the channel with `status` that list `msg_id`.
pub fn receipts_with(roles: &mut Roles, msg_id: MessageId, status: ReceiptStatus) -> Vec<Got> {
    (roles.c_receipts().into_iter())
        .filter(|got| got.receipt.status == status && got.lists(msg_id))
        .collect()
}

/// C takes in its mailbox every half second, for up to [`WAIT`], until it has read a receipt with
/// `status` of `msg_id`. Fails unless it read one such, sent by B's device from claude's address to
/// `to`, the message's `from`, with `hops` 0. Gives it.
#[track_caller]
pub fn c_reads(roles: &mut Roles, msg_id: MessageId, status: ReceiptStatus, to: &str) -> Got {
    let deadline = Instant::now() + WAIT;
    let mut found = receipts_with(roles, msg_id, status);
    while found.is_empty() {
        assert!(Instant::now() < deadline, "no {status:?} of {msg_id} read");
        thread::sleep(Duration::from_millis(500));
        found = receipts_with(roles, msg_id, status);
    }
    let [got] = found.as_slice() else {
        panic!("not one {status:?} of {msg_id}: {found:#?}");
    };
    assert_eq!(got.sender, roles.b_device(), "{got:#?}");
    assert_eq!(got.content.from.to_string(), roles.full(CLAUDE), "{got:#?}");
    let addressee = got.content.to.as_ref().map(ToString::to_string);
    assert_eq!(addressee.as_deref(), Some(to), "{got:#?}");
    assert_eq!(got.content.hops, 0, "{got:#?}");
    got.clone()
}

/// The reason of `got`; fails if it has none.
#[track_caller]
pub fn reason_of(got: &Got) -> String {
    let reason = got.receipt.reason.clone().unwrap_or_default();
    assert!(!reason.trim().is_empty(), "no reason: {got:#?}");
    reason
}

/// Roles with claude, B's agent, in `hold`.
#[track_caller]
pub fn holding(ids: [u64; 2], logins: [&str; 2]) -> Roles {
    let roles = Roles::new(ids, logins);
    claude_set_to(&roles, HOLD);
    roles
}

/// The person's `skald held` on B's machine, in B's `~/work/acme`, accepts `text`, A's agent's
/// message `msg_id` to claude: claude's next `skald recv` gets it once, and A's runs show `read` of
/// it once. Gives what `skald held` showed.
#[track_caller]
pub fn accepted_then_read(roles: &Roles, msg_id: MessageId, text: &str) -> String {
    let listed = decide(
        roles.b(),
        &roles.senders.agents.folder,
        &[(text, Decision::Accept)],
    );
    let got = roles.claude_reads(false).shown();
    assert_shows_once(&got, text);
    shows_read_once(roles.a(), msg_id);
    listed
}
