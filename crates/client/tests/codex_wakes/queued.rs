//! An idle Codex session gets one `codex queue` for its thread, on its config home and with its
//! process's `codex`, within a second of a message for its agent; it holds only the signal, whoever
//! sent the message, at whatever level, however long, and with the `text` setting `inline`.

use std::path::PathBuf;
use std::thread;
use std::time::Duration;

use skald_proto::MessageId;

use crate::codex::Codex;
use crate::delivery::{INLINE, set_text};
use crate::standin::{StandIn, assert_queued, assert_quiet, assert_soon, now, web_in_app};
use crate::{
    devices::second_machine,
    fixtures::{SEND, TO},
    hooks::REVIEW,
    kit::received,
    lines::{agent_lines, refers_to},
    senders::{LAPTOP, PEER},
    solo::WEB,
    talk::{ADA_HOST, one_message_sent},
    world::{ADD_RULE, AGENT, CLIENT_A, Person, dir_of, path, run},
};

/// What api says to web before the timed messages; it is nowhere else.
const FIRST: &str = "codex wake test: api asks web to look at the first issue";
/// What Bob's agent review says to web, at the level `peer` with the text setting `inline`.
const FROM_BOB: &str = "codex wake test: review asks web to rotate the staging key";
/// What the agent review of Ada's second machine says to web, at the level `own`.
const FROM_LAPTOP: &str = "codex wake test: the laptop asks web to merge the docs branch";

/// How long web's session stays idle before each timed message: past the helper's reload of the
/// registrations after the last turn's hooks, so only the message itself can wake the session.
const IDLE: Duration = Duration::from_secs(3);

/// A message of 15,000 characters that api sends web.
fn long_spec() -> String {
    let mut text = String::from("codex wake test: api sends web a spec of 15,000 characters. ");
    while text.chars().count() < 15_000 {
        text.push_str("No word of this spec goes into codex queue; inbox gives it all. ");
    }
    text.chars().take(15_000).collect()
}

/// Ada's second machine, linked by hand, with `~/work/clientA` bound to `clienta`; gives it and
/// that folder.
fn second_device(codex: &Codex) -> (Person, PathBuf) {
    let talk = &codex.web.solo.talk;
    let laptop = second_machine(&talk.world, &talk.ada, LAPTOP);
    for machine in [&laptop.machine, codex.machine()] {
        received(machine);
    }
    let folder = dir_of(&laptop.machine, "work/clientA");
    let bound = run(&laptop.machine, &[ADD_RULE, path(&folder), CLIENT_A]);
    assert!(bound.success, "the laptop's folder not bound: {bound:?}");
    (laptop, folder)
}

/// Fails unless web's `inbox` gives each of `sent` once, by its reference and from the start of
/// its text.
#[track_caller]
pub fn assert_inbox_gives(codex: &mut Codex, sent: &[(&str, MessageId)]) {
    let shown = codex.web.session.inbox();
    let lines = agent_lines(&shown);
    for (text, msg_id) in sent {
        let of: Vec<_> = (lines.iter())
            .filter(|line| refers_to(&line.reference, *msg_id))
            .collect();
        assert_eq!(of.len(), 1, "{msg_id} not given once: {shown}");
        let head: String = text.chars().take(40).collect();
        assert!(of[0].text.starts_with(&head), "not {head:?}: {shown}");
    }
}

/// Bob joins `clienta`, Ada's second machine is linked, and the person sets the text of `peer` in
/// the default channel to `inline`. web's session starts in a Codex app of its own, with a config
/// home of its own: it is idle. api sends web a message: the app's `codex` runs `codex queue` once;
/// a turn of the session runs. Then, one after another, each once the session stayed idle 3
/// seconds, Bob's agent review sends web a message, the second machine's agent review one, and api
/// one of 15,000 characters: after each, the app's `codex` runs `codex queue` once more, within a
/// second of the send's return, for web's thread with its config home, holding only the signal,
/// and then a turn runs. No other run of `codex` follows, and the `codex` on the search path never
/// runs. web's `inbox` gives the four messages.
pub fn an_idle_session_gets_one_codex_queue_with_only_the_signal_within_a_second() {
    let mut codex = Codex::new(4_928_001_001, "ada-codex-wake-idle");
    let bob = codex.web.with_bob(4_928_001_002, "bob-codex-wake-idle");
    let (laptop, folder) = second_device(&codex);
    let solo = &codex.web.solo;
    set_text(solo, solo.default_channel(), PEER, INLINE);
    let on_path = StandIn::on_path(&codex);
    let (app, web, mut process) = web_in_app(&codex);

    let first = codex.web.own(FIRST);
    let ran = app.await_runs(1);
    assert_queued(&ran[0], (&web, &app), &[FIRST], &[first], &[]);
    process.turn(&codex, &web);

    let solo = &codex.web.solo;
    let review = bob.review();
    let laptop_review = format!("{}/{REVIEW}", solo.talk.ada.handle);
    let handle = bob.person.handle.to_string();
    let api = codex.web.api();
    let names = [
        api.as_str(),
        review.as_str(),
        laptop_review.as_str(),
        handle.as_str(),
        bob.person.host,
        ADA_HOST,
        LAPTOP,
    ];
    let long = long_spec();
    let mut sent = vec![(FIRST, first)];
    for round in 0..3 {
        thread::sleep(IDLE);
        let (text, msg_id) = match round {
            0 => (FROM_BOB, codex.web.peer(&bob, FROM_BOB)),
            1 => {
                let mark = solo.mark();
                let to = solo.short(WEB);
                let args = [AGENT, REVIEW, SEND, TO, &to, FROM_LAPTOP];
                let finished = laptop.machine.start_in(&folder, &args).finish();
                assert!(finished.success, "the laptop's send: {finished:?}");
                let msg_id = one_message_sent(&solo.talk.front, solo.default_channel(), mark);
                (FROM_LAPTOP, msg_id)
            }
            _ => (long.as_str(), codex.web.own(&long)),
        };
        let came = now();
        let ran = app.await_runs(round + 2);
        let last = ran.last().expect("a run");
        assert_soon(last, came);
        assert_queued(last, (&web, &app), &[text], &[msg_id], &names);
        process.turn(&codex, &web);
        sent.push((text, msg_id));
    }

    assert_quiet(&[
        (&app, sent.len(), "one codex queue per message"),
        (&on_path, 0, "the codex on the search path"),
    ]);
    assert_inbox_gives(&mut codex, &sent);
}
