//! A session's `inbox`: the messages kept for its agent since the session initialized, before its
//! first call too, as the lines the agent's `skald recv` shows, then one short line; none the
//! device wrote before; and the warnings the agent's `skald recv` shows.

use crate::agents::{Agents, CLAUDE};
use crate::gaps::{MISSING, second_dropped};
use crate::kit::received;
use crate::lines::{
    agent_line, agent_line_of, assert_handed_line, assert_no_uuid, lines_referring_to,
};
use crate::talk::{BOB_HOST, Line};
use crate::{CLIENT, assert_nothing_waiting, session_in};

/// What Bob's device says; none is anywhere else.
const SINCE: [&str; 2] = [
    "inbox test: the staging deploy is green",
    "inbox test: the hotfix branch is merged",
];
const BEFORE: &str = "inbox test: a note from before the session";
const AFTER: &str = "inbox test: a note from after the session started";
const SAYS: [&str; 3] = [
    "inbox test: the first of three",
    "inbox test: the second, which never arrives",
    "inbox test: the third of three",
];
const FORGED: &str = "inbox test: a message whose messageId is not its msg_id";

/// Ada's device has run as claude in `~/work/acme`, and a session of the MCP client `claude-code`
/// starts there and initializes. Bob's device sends a message to the channel, which the person's
/// `skald recv` takes in before the session's first call, then another. The session's first
/// `inbox` shows each once, by its reference, on the line claude's next `skald recv` shows it on;
/// its second `inbox` gives one short line.
#[test]
fn inbox_shows_what_came_since_the_session_started_as_the_agents_recv_shows_it() {
    let mut agents = Agents::new(
        [4_627_001_001, 4_627_001_002],
        ["ada-reads-in-her-session", "bob-writes-to-the-session"],
    );
    agents.run_as(CLAUDE);
    let mut session = session_in(&agents.talk.ada.machine, &agents.folder, &[], CLIENT);
    let bob = agents.bob();
    let first = agents.bob_sends(&Line::new(&bob, 1, SINCE[0]));
    received(&agents.talk.ada.machine);
    let second = agents.bob_sends(&Line::new(&bob, 2, SINCE[1]));

    let shown = session.inbox();

    let claude = agents.run_as(CLAUDE).shown();
    for (text, id) in SINCE.into_iter().zip([first, second]) {
        let line = assert_handed_line(&shown, text, id);
        assert_eq!(line, agent_line_of(&claude, text), "not claude's line");
    }

    let again = session.inbox();

    assert_nothing_waiting(&again);
}

/// Bob's device sends a message to the channel, which the person's `skald recv` takes in. Then a
/// session of `claude-code` starts in `~/work/acme`, and Bob's device sends another. The session's
/// `inbox` shows the second, not the first, which the device wrote before the agent registered.
#[test]
fn message_the_device_wrote_before_the_session_registered_is_not_for_it() {
    let mut agents = Agents::new(
        [4_627_002_001, 4_627_002_002],
        ["ada-starts-her-session-late", "bob-wrote-before-it"],
    );
    let bob = agents.bob();
    agents.bob_sends(&Line::new(&bob, 1, BEFORE));
    let person = received(&agents.talk.ada.machine).shown();
    assert!(person.contains(BEFORE), "{person}");
    let mut session = session_in(&agents.talk.ada.machine, &agents.folder, &[], CLIENT);
    let after = agents.bob_sends(&Line::new(&bob, 2, AFTER));

    let shown = session.inbox();

    assert_handed_line(&shown, AFTER, after);
    assert!(!shown.contains(BEFORE), "{shown}");
}

/// A session of `claude-code` has started in `~/work/acme`. Bob's device sends three messages to
/// the channel; the second never reaches the server. The session's `inbox`, the first to get them,
/// shows the first and the third, and warns on one line, without IDs, that a message from Bob's
/// device is missing before the third, by the third's reference, as the agent's `skald recv` does.
#[test]
fn inbox_warns_of_missing_messages_as_the_agents_recv_does() {
    let mut agents = Agents::new(
        [4_627_003_001, 4_627_003_002],
        ["ada-session-sees-a-gap", "bob-loses-one-on-the-way"],
    );
    let mut session = session_in(&agents.talk.ada.machine, &agents.folder, &[], CLIENT);
    let (first, third) = second_dropped(&mut agents.talk, SAYS);

    let shown = session.inbox();

    assert_handed_line(&shown, SAYS[0], first);
    assert_handed_line(&shown, SAYS[2], third);
    let notes: Vec<String> = (lines_referring_to(&shown, third).into_iter())
        .filter(|line| agent_line(line).is_none())
        .collect();
    let [note] = notes.as_slice() else {
        panic!("not one note of the third: {shown}");
    };
    assert!(
        note.contains(BOB_HOST) && note.to_lowercase().contains(MISSING),
        "{note}"
    );
    assert_no_uuid(&shown);
}

/// A session of `claude-code` starts in `~/work/acme`. A proxy puts into Ada's catch-up Bob's
/// message in the channel whose `messageId` is not its `msg_id`, which the device drops. The
/// session's `inbox` gives the one line of warning of it that the agent's `skald recv` shows, by
/// its reference and naming Bob's device, and not the text, nor the line of nothing waiting.
#[test]
fn inbox_that_has_only_a_warning_gives_it() {
    let mut agents = Agents::new(
        [4_627_004_001, 4_627_004_002],
        ["ada-session-told-of-a-drop", "bob-forges-for-the-session"],
    );
    let mut session = session_in(&agents.talk.ada.machine, &agents.folder, &[], CLIENT);
    let msg_id = agents.bob_forges(FORGED);

    let shown = session.inbox();

    let lines: Vec<&str> = (shown.lines().map(str::trim))
        .filter(|line| !line.is_empty())
        .collect();
    let [note] = lines.as_slice() else {
        panic!("not one line: {shown:?}");
    };
    assert_eq!(
        lines_referring_to(&shown, msg_id),
        [note.to_string()],
        "{shown:?}"
    );
    assert!(
        note.contains(BOB_HOST) && agent_line(note).is_none(),
        "{note}"
    );
    assert!(!shown.contains(FORGED), "{shown:?}");
    assert_no_uuid(&shown);
}
