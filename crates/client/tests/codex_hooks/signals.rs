//! Every message reaches a Codex session only as the signal, one fixed line whatever its sender,
//! level or length; each new message gives it once, by `PostToolUse`, `UserPromptSubmit` or `Stop`,
//! and the agent takes the messages with `inbox`.

use std::path::PathBuf;

use skald_proto::MessageId;

use crate::codex::{Codex, Event, assert_bare};
use crate::devices::second_machine;
use crate::fixtures::{SEND, TO};
use crate::hooks::REVIEW;
use crate::kit::received;
use crate::lines::{agent_lines, assert_handed_line, refers_to};
use crate::senders::LAPTOP;
use crate::solo::WEB;
use crate::talk::{ADA_HOST, one_message_sent};
use crate::world::{ADD_RULE, AGENT, CLIENT_A, Person, dir_of, path, run};

/// What Bob's agent review and Ada's agent api say to web; none is anywhere else.
const FROM_BOB: &str = "codex signal test: review asks web to rerun the migration";
const FROM_API: &str = "codex signal test: api asks web to tag the release";
const FIRST_OF_TWO: &str = "codex signal test: api asks web for the first of two notes";
const SECOND_OF_TWO: &str = "codex signal test: api asks web for the second of two notes";

const BARE_SIGNAL: &str =
    "skald: messages wait for you: call the inbox tool, or skald recv, to read them";
const RECV_REFERENCE_ID: &str = "01a20aae-c14f-7a7a-8de6-e0d841a9bec7";

#[test]
fn a_bare_signal_can_contain_a_command_word_that_matches_a_reference() {
    let msg_id = RECV_REFERENCE_ID.parse().expect("a message ID");
    assert!(refers_to("recv", msg_id));
    assert_bare(BARE_SIGNAL, &[], &[msg_id], &[]);
}

#[test]
#[should_panic(expected = "the reference of")]
fn a_bare_signal_cannot_contain_a_formatted_reference() {
    let msg_id = RECV_REFERENCE_ID.parse().expect("a message ID");
    assert_bare(&format!("{BARE_SIGNAL} (#recv)"), &[], &[msg_id], &[]);
}

#[test]
#[should_panic(expected = "IDs")]
fn a_bare_signal_cannot_contain_a_raw_message_id() {
    let msg_id = RECV_REFERENCE_ID.parse().expect("a message ID");
    assert_bare(
        &format!("{BARE_SIGNAL} ({RECV_REFERENCE_ID})"),
        &[],
        &[msg_id],
        &[],
    );
}

/// The text of a message of 15,000 characters that api sends web.
fn long_text() -> String {
    let head = "codex signal test: api sends web a long spec. ";
    let mut text = head.to_owned();
    while text.chars().count() < 15_000 {
        text.push_str("Every line of it is the agent's to read with inbox. ");
    }
    text.chars().take(15_000).collect()
}

/// Bob joins `clienta`, and web's session starts its turn. Bob's agent review sends web a message, of the
/// level `peer`; then api sends web one, of the level `own`; then one of 15,000 characters; then two
/// at once. After each, web's `PostToolUse` gives the signal: the same line each time, holding none
/// of their texts, senders, devices or references. web's `inbox` then shows each message once, by
/// its reference and from the start of its text.
#[test]
fn every_message_reaches_the_session_only_as_the_same_signal() {
    let mut codex = Codex::new(4_927_006_001, "ada-codex-one-signal");
    let bob = codex.web.with_bob(4_927_006_002, "bob-codex-one-signal");
    let web = codex.thread_of(WEB);
    codex.starts(&web);
    codex.busy(&web);
    let long = long_text();
    let api = codex.web.api();
    let review = bob.review();
    let handle = bob.person.handle.to_string();
    let names = [
        api.as_str(),
        review.as_str(),
        handle.as_str(),
        bob.person.host,
        ADA_HOST,
    ];

    let mut sent: Vec<(&str, MessageId)> = Vec::new();
    let mut signals = Vec::new();
    sent.push((FROM_BOB, codex.web.peer(&bob, FROM_BOB)));
    signals.push(codex.gives(&web, Event::PostToolUse));
    sent.push((FROM_API, codex.web.own(FROM_API)));
    signals.push(codex.gives(&web, Event::PostToolUse));
    sent.push((long.as_str(), codex.web.own(&long)));
    signals.push(codex.gives(&web, Event::PostToolUse));
    sent.push((FIRST_OF_TWO, codex.web.own(FIRST_OF_TWO)));
    sent.push((SECOND_OF_TWO, codex.web.own(SECOND_OF_TWO)));
    signals.push(codex.gives(&web, Event::PostToolUse));

    let texts: Vec<&str> = sent.iter().map(|(text, _)| *text).collect();
    let msg_ids: Vec<MessageId> = sent.iter().map(|(_, msg_id)| *msg_id).collect();
    for signal in &signals {
        assert_bare(signal, &texts, &msg_ids, &names);
        assert_eq!(signal, &signals[0], "not the same line: {signals:#?}");
    }
    let shown = codex.web.session.inbox();
    let lines = agent_lines(&shown);
    assert_eq!(lines.len(), sent.len(), "not one line per message: {shown}");
    for (text, msg_id) in &sent {
        let head: String = text.chars().take(40).collect();
        let holding: Vec<_> = (lines.iter())
            .filter(|line| refers_to(&line.reference, *msg_id) && line.text.starts_with(&head))
            .collect();
        assert_eq!(holding.len(), 1, "{head:?} not shown once: {shown}");
    }
}

/// What web gets from Ada's second machine and from api; neither is anywhere else.
const FROM_LAPTOP: &str = "codex signal test: the laptop's agent asks web to sync the notes";
const AFTER_LAPTOP: &str = "codex signal test: api asks web after the laptop did";

/// Ada's second machine `laptop`, linked by hand, joins `clienta`'s channels; its `~/work/clientA`
/// is bound to `clienta`. Gives it and that folder.
fn laptop_of(codex: &Codex) -> (Person, PathBuf) {
    let talk = &codex.web.solo.talk;
    let laptop = second_machine(&talk.world, &talk.ada, LAPTOP);
    received(&laptop.machine);
    received(codex.machine());
    let folder = dir_of(&laptop.machine, "work/clientA");
    let bound = run(&laptop.machine, &[ADD_RULE, path(&folder), CLIENT_A]);
    assert!(bound.success, "not bound: {bound:?}");
    (laptop, folder)
}

/// Ada's second machine joins `clienta`, and web's session starts its turn. The second machine's agent
/// review sends web a message, of the level `own` from another device: web's `PostToolUse` gives
/// the signal, holding none of its text, sender, device or reference. api then sends web one: its
/// signal is the same line. web's `inbox` shows both.
#[test]
fn a_message_from_the_accounts_second_machine_reaches_the_session_only_as_the_signal() {
    let mut codex = Codex::new(4_927_007_001, "ada-codex-second-machine");
    let (laptop, folder) = laptop_of(&codex);
    let web = codex.thread_of(WEB);
    codex.starts(&web);
    codex.busy(&web);
    let solo = &codex.web.solo;

    let mark = solo.mark();
    let to = solo.short(WEB);
    let args = [AGENT, REVIEW, SEND, TO, &to, FROM_LAPTOP];
    let sent = laptop.machine.start_in(&folder, &args).finish();
    assert!(sent.success, "{sent:?}");
    let from_laptop = one_message_sent(&solo.talk.front, solo.default_channel(), mark);
    let signal = codex.gives(&web, Event::PostToolUse);
    let after = codex.web.own(AFTER_LAPTOP);
    let next = codex.gives(&web, Event::PostToolUse);

    let review = format!("{}/{REVIEW}", solo.talk.ada.handle);
    assert_bare(&signal, &[FROM_LAPTOP], &[from_laptop], &[&review, LAPTOP]);
    assert_eq!(next, signal, "not the same line");
    let shown = codex.web.session.inbox();
    assert_handed_line(&shown, FROM_LAPTOP, from_laptop);
    assert_handed_line(&shown, AFTER_LAPTOP, after);
}

/// What api says to web, one message per hook that gives its signal; none is anywhere else.
const BY_POST_TOOL_USE: &str = "codex signal test: api asks web, signalled after a tool";
const BY_PROMPT: &str = "codex signal test: api asks web, signalled with the next prompt";
const BY_STOP: &str = "codex signal test: api asks web, signalled as the turn ends";

/// Fails unless each hook of [`Event::TURN`] prints nothing for `web`, which leaves it busy.
#[track_caller]
fn turn_is_silent(codex: &Codex, web: &crate::codex::Thread, when: &str) {
    for event in Event::TURN {
        codex.silent(web, event, when);
    }
}

/// web's session starts its turn. api sends web a message: `PostToolUse` gives the signal, and then every
/// hook of the turn prints nothing. api sends another: `UserPromptSubmit` gives the signal, and then
/// every hook prints nothing. api sends a third: `Stop` continues the turn with the signal, the
/// continued turn's `Stop` lets it end, and then every hook prints nothing. The three signals are
/// one line; web's `inbox` then shows the three messages.
#[test]
fn each_new_message_gives_the_signal_once_and_stop_continues_the_turn_once() {
    let mut codex = Codex::new(4_927_008_001, "ada-codex-signal-once");
    let web = codex.thread_of(WEB);
    codex.starts(&web);
    codex.busy(&web);

    let first = codex.web.own(BY_POST_TOOL_USE);
    let after_tool = codex.gives(&web, Event::PostToolUse);
    turn_is_silent(&codex, &web, "after PostToolUse gave the signal");
    let second = codex.web.own(BY_PROMPT);
    let with_prompt = codex.gives(&web, Event::UserPromptSubmit);
    turn_is_silent(&codex, &web, "after UserPromptSubmit gave the signal");
    let third = codex.web.own(BY_STOP);
    let at_stop = codex.gives(&web, Event::Stop);
    codex.silent(&web, Event::StopActive, "the continued turn's Stop");
    turn_is_silent(&codex, &web, "after Stop gave the signal");

    let sent = [
        (BY_POST_TOOL_USE, first),
        (BY_PROMPT, second),
        (BY_STOP, third),
    ];
    let texts = sent.map(|(text, _)| text);
    let msg_ids = sent.map(|(_, msg_id)| msg_id);
    assert_bare(&after_tool, &texts, &msg_ids, &[]);
    assert_eq!(with_prompt, after_tool, "UserPromptSubmit's line");
    assert_eq!(at_stop, after_tool, "Stop's line");
    let shown = codex.web.session.inbox();
    for (text, msg_id) in sent {
        assert_handed_line(&shown, text, msg_id);
    }
}
