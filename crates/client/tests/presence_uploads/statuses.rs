//! An agent's state and note, set from the CLI and from the MCP tool, in every channel of its
//! space; a note over 128 characters refused, naming the limit.

use skald_proto::AgentState;

use crate::agents::{Agents, CLAUDE};
use crate::client::{CLIENT, session_in};
use crate::helped::Helped;
use crate::presence::{AVAILABLE, BUSY, assert_between};
use crate::values::{
    NOTE_FLAG, NOTE_LIMIT, NOTE_MAX, SET_STATUS, acme_channels, ada_s_body, await_available,
    await_in_each, claude_code_session, listed, note_of, run_with, set_status_tool,
    shows_with_note,
};
use crate::world::{AGENT, now};

/// What claude works on, as its CLI says.
const FROM_THE_CLI: &str = "presence test: reviewing PR 12";

/// Through a helper, claude runs an MCP session of `claude-code` in `~/work/acme`, shown
/// `available`. Its `skald set-status busy` with a note makes Ada's value in each channel of
/// `acme` list it `busy` with that note, since then. The session's `set_status` tool then sets it
/// `available` with a note of 128 characters, which each value lists, since then.
#[test]
fn state_and_note_set_from_the_cli_and_from_the_tool_reach_every_channel_of_the_space() {
    let mut agents = Agents::new(
        [4_731_011_001, 4_731_011_002],
        ["ada-tells-what-claude-does", "bob-reads-the-notes"],
    );
    let helped = Helped::new(&agents.talk.ada.machine);
    let mut session = claude_code_session(&helped, &agents, CLAUDE);
    let (claude, acme) = await_available(&mut agents, CLAUDE);
    let set = now();

    let ran = helped.run_in(
        &agents.folder,
        &[AGENT, CLAUDE, SET_STATUS, BUSY, NOTE_FLAG, FROM_THE_CLI],
    );

    assert!(ran.success, "{ran:?}");
    let busy = await_in_each(&mut agents.talk, &acme, "with claude busy", |body| {
        shows_with_note(body, &claude, AgentState::Busy, FROM_THE_CLI)
    });
    for body in &busy {
        assert_between(listed(body, &claude).since, (set, now()), "busy");
    }
    let longest = note_of(NOTE_MAX);
    let set = now();

    let called = set_status_tool(&mut session, AVAILABLE, Some(&longest));

    assert!(!called.error, "{called:?}: {}", session.errors());
    let back = await_in_each(&mut agents.talk, &acme, "with the tool's note", |body| {
        shows_with_note(body, &claude, AgentState::Available, &longest)
    });
    for body in &back {
        assert_between(listed(body, &claude).since, (set, now()), "available again");
    }
}

/// Claude's `skald set-status busy` with a note of 129 characters fails, saying `128`; so does the
/// `set_status` tool of a session of `claude-code`, as an error. No value of Ada's lists the note.
#[test]
fn note_over_128_characters_is_refused_naming_the_limit() {
    let agents = Agents::new(
        [4_731_012_001, 4_731_012_002],
        ["ada-writes-too-long-a-note", "bob-never-sees-it"],
    );
    let machine = &agents.talk.ada.machine;
    let too_long = note_of(NOTE_MAX + 1);

    let ran = run_with(
        machine,
        &agents.folder,
        &[],
        &[AGENT, CLAUDE, SET_STATUS, BUSY, NOTE_FLAG, &too_long],
    );

    assert!(!ran.success, "a note of 129 characters set: {ran:?}");
    assert!(ran.shown().contains(NOTE_LIMIT), "{ran:?}");
    let mut session = session_in(machine, &agents.folder, &[AGENT, CLAUDE], CLIENT);

    let called = set_status_tool(&mut session, BUSY, Some(&too_long));

    assert!(called.error, "a note of 129 characters set: {called:?}");
    assert!(called.text.contains(NOTE_LIMIT), "{called:?}");
    for at in acme_channels(&agents) {
        let body = ada_s_body(&agents.talk, at);
        let notes: Vec<_> = (body.iter().flat_map(|body| &body.agents))
            .filter_map(|status| status.note.as_deref())
            .collect();
        assert!(!notes.contains(&too_long.as_str()), "{notes:?}");
    }
}
