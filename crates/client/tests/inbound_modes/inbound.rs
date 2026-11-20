//! What the tests share: the commands that set and list inbound modes and allowlists; what the
//! sending device shows of `refused`, and the refusing device's person of a refused message; the
//! `refused` receipt C reads.

use std::path::Path;
use std::thread;
use std::time::{Duration, Instant};

use skald_proto::{MessageId, ReceiptStatus};

use crate::agents::CLAUDE;
use crate::cloud::WAIT;
use crate::kit::received;
use crate::lines::{agent_line, assert_no_uuid, lines_referring_to};
use crate::machine::{Finished, Machine};
use crate::receipts::{Roles, assert_shows_once};
use crate::senders::{LAPTOP_AGENT, holds_word};

/// The commands that set an agent's inbound mode, put a sender on its allowlist, and list both.
pub const SET_MODE: &str = "set-inbound-mode";
pub const ALLOW: &str = "allow-sender";
pub const MODES: &str = "inbound-modes";

/// The modes these tests set.
pub const ACCEPT: &str = "accept";
pub const REFUSE: &str = "refuse";

/// The status the CLI shows once the `to` agent's device refused the message.
pub const REFUSED: &str = "refused";

/// Runs `skald <args>` as the person on `machine` in `dir`; fails unless it succeeds.
#[track_caller]
pub fn person_runs_in(machine: &Machine, dir: &Path, args: &[&str]) -> Finished {
    let finished = machine.start_in(dir, args).finish();
    assert!(finished.success, "{args:?}: {finished:?}");
    finished
}

/// Sets the mode of `agent` to `mode` as the person on `machine` in `dir`.
#[track_caller]
pub fn set_mode(machine: &Machine, dir: &Path, agent: &str, mode: &str) {
    person_runs_in(machine, dir, &[SET_MODE, agent, mode]);
}

/// Puts `sender` on `agent`'s allowlist as the person on `machine` in `dir`.
#[track_caller]
pub fn allow(machine: &Machine, dir: &Path, agent: &str, sender: &str) {
    person_runs_in(machine, dir, &[ALLOW, agent, sender]);
}

/// B's person sets claude's mode to `mode` in B's `~/work/acme`.
#[track_caller]
pub fn claude_set_to(roles: &Roles, mode: &str) {
    set_mode(roles.b(), &roles.senders.agents.folder, CLAUDE, mode);
}

/// B's person puts `sender` on claude's allowlist in B's `~/work/acme`.
#[track_caller]
pub fn claude_allows(roles: &Roles, sender: &str) {
    allow(roles.b(), &roles.senders.agents.folder, CLAUDE, sender);
}

/// The lines of `shown`, a person's output, that hold `msg_id` and the word [`REFUSED`].
pub fn refused_lines(shown: &str, msg_id: MessageId) -> Vec<String> {
    let id = msg_id.to_string();
    (shown.lines())
        .filter(|line| line.contains(&id) && holds_word(line, REFUSED))
        .map(str::to_owned)
        .collect()
}

/// The lines of `shown`, the refusing device's person's output, that mark `msg_id` refused for
/// `agent`: they hold its `msg_id`, the word [`REFUSED`] and the agent's name.
pub fn marked_lines(shown: &str, msg_id: MessageId, agent: &str) -> usize {
    (refused_lines(shown, msg_id).iter())
        .filter(|line| line.contains(agent))
        .count()
}

/// The lines of `shown`, an agent's output, that say [`REFUSED`] of `msg_id` by its reference and
/// are no message.
pub fn agent_refused_lines(shown: &str, msg_id: MessageId) -> Vec<String> {
    (lines_referring_to(shown, msg_id).into_iter())
        .filter(|line| agent_line(line).is_none() && holds_word(line, REFUSED))
        .collect()
}

/// Runs the person's `skald recv` on `machine` every half second, for up to [`WAIT`], until the
/// runs together show `refused` of `msg_id`; fails if a run fails or none shows it. Fails unless
/// they show it on one line, and the next run shows it no more. Gives what the runs showed together,
/// the next one included.
#[track_caller]
pub fn shows_refused_once(machine: &Machine, msg_id: MessageId) -> String {
    let deadline = Instant::now() + WAIT;
    let mut shown = received(machine).shown();
    while refused_lines(&shown, msg_id).is_empty() {
        assert!(
            Instant::now() < deadline,
            "no {REFUSED} of {msg_id} shown: {shown}"
        );
        thread::sleep(Duration::from_millis(500));
        shown += &received(machine).shown();
    }
    let next = received(machine).shown();
    shown += &next;
    let lines = refused_lines(&shown, msg_id);
    assert_eq!(
        lines.len(),
        1,
        "{REFUSED} of {msg_id} not on one line: {shown}"
    );
    shown
}

/// Fails if `machine`'s next person's `skald recv` shows `refused` of `msg_id`.
#[track_caller]
pub fn shows_no_refused(machine: &Machine, msg_id: MessageId, when: &str) {
    let shown = received(machine).shown();
    let lines = refused_lines(&shown, msg_id);
    assert!(lines.is_empty(), "{when}: {lines:?} in {shown}");
}

/// C reads B's receipt of `msg_id`; fails unless it is one `refused` receipt of claude's, sent by
/// B's device with `report` false, from claude's address to A's agent's, with `hops` 0 and a
/// reason. Gives the reason.
#[track_caller]
pub fn refused_receipt(roles: &mut Roles, msg_id: MessageId) -> String {
    let receipts = roles.c_gets_receipt_of(msg_id);
    let of_it: Vec<_> = receipts.iter().filter(|got| got.lists(msg_id)).collect();
    let [got] = of_it.as_slice() else {
        panic!("not one receipt of {msg_id}: {receipts:#?}");
    };
    assert_eq!(got.receipt.status, ReceiptStatus::Refused, "{got:#?}");
    assert_eq!(got.sender, roles.b_device(), "{got:#?}");
    assert_eq!(got.content.from.to_string(), roles.full(CLAUDE), "{got:#?}");
    assert_eq!(
        got.content.to.as_ref().map(ToString::to_string),
        Some(roles.full(LAPTOP_AGENT)),
        "{got:#?}"
    );
    assert_eq!(got.content.hops, 0, "{got:#?}");
    let front = &roles.senders.agents.talk.front;
    let reports: Vec<bool> = (front.sent().into_iter())
        .filter(|sent| sent.envelope.envelope().header.msg_id == got.msg_id)
        .map(|sent| sent.report)
        .collect();
    assert!(
        !reports.is_empty() && reports.iter().all(|report| !report),
        "not sent with report false: {reports:?}"
    );
    match &got.receipt.reason {
        Some(reason) if !reason.trim().is_empty() => reason.clone(),
        _ => panic!("no reason: {got:#?}"),
    }
}

/// B's person's next `skald recv` shows `text`, the message `msg_id`, once, and marks it refused
/// for claude on one line.
#[track_caller]
pub fn b_shows_it_marked(roles: &Roles, msg_id: MessageId, text: &str) {
    let person = received(roles.b()).shown();
    assert_shows_once(&person, text);
    assert_eq!(
        marked_lines(&person, msg_id, CLAUDE),
        1,
        "not marked {REFUSED} for {CLAUDE} once: {person}"
    );
}

/// Once B has refused `msg_id`: C reads the `refused` receipt with its reason; A's person's runs
/// show `refused` of it once, with the reason; A's agent's next `skald recv` says it once, with the
/// reason and no UUID, and the one after does not.
#[track_caller]
pub fn a_sees_refused(roles: &mut Roles, msg_id: MessageId) {
    let reason = refused_receipt(roles, msg_id);

    let shown = shows_refused_once(roles.a(), msg_id);
    let lines = refused_lines(&shown, msg_id);
    assert!(
        lines.iter().all(|line| line.contains(&reason)),
        "no reason {reason:?}: {lines:?}"
    );

    let agent = roles.a_agent_reads(&[]).shown();
    let lines = agent_refused_lines(&agent, msg_id);
    let [said] = lines.as_slice() else {
        panic!("{REFUSED} of {msg_id} not on one line: {agent}");
    };
    assert!(said.contains(&reason), "no reason {reason:?}: {said}");
    assert_no_uuid(&agent);
    let next = roles.a_agent_reads(&[]).shown();
    let again = agent_refused_lines(&next, msg_id);
    assert!(again.is_empty(), "{REFUSED} again: {next}");
}
