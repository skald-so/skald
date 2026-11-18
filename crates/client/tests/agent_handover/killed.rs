//! A CLI killed during a handover: before its output is written, or after it but before the mark,
//! the next run as the agent hands the message over again; after the mark, nothing again
//! (`held.rs`).

use std::thread;

use crate::held::{FILL, HEAD, Held, long_text, write_lock};
use crate::kit::received;
use crate::talk::{BOB_HOST, Line, assert_shows_once};
use crate::{Agents, CLAUDE, assert_handed_once, assert_not_handed};

/// Ada's device has run as claude in `~/work/acme`. Bob's device sends a message with a long text
/// to the channel, which the person's `skald recv` shows. claude's run, killed while it waits in
/// the middle of writing the text, never wrote it whole: the next one writes it whole, and, killed
/// while it waits to mark it, the next one shows it once, with the reference of its `msg_id`;
/// after that run, nothing again.
#[test]
fn cli_killed_before_its_output_or_before_the_mark_hands_the_message_over_again() {
    let mut agents = Agents::new(
        [4_147_075_001, 4_147_075_002],
        ["ada-agent-crashes", "bob-sends-the-export"],
    );
    agents.run_as(CLAUDE);
    let bob = agents.bob();
    let text = long_text();
    let id = agents.bob_sends(&Line::new(&bob, 1, &text));
    let person = received(&agents.talk.ada.machine).shown();
    let channel = agents.talk.channel;
    assert_shows_once(&person, &text, (channel, &bob, BOB_HOST));
    let (machine, folder) = (&agents.talk.ada.machine, &agents.folder);

    let mut before_output = Held::start(machine, folder, CLAUDE);
    before_output.wait_for(HEAD);
    thread::sleep(FILL);
    let killed = before_output.kill();

    assert!(!killed.contains(&text), "the text written whole");

    let mut before_mark = Held::start(machine, folder, CLAUDE);
    before_mark.wait_for(HEAD);
    thread::sleep(FILL);
    let lock = write_lock(machine);
    before_mark.let_go();
    before_mark.wait_for(&text);
    before_mark.kill();
    lock.execute_batch("ROLLBACK")
        .expect("let go of the local state's write lock");

    let next = agents.run_as(CLAUDE).shown();

    assert_handed_once(&next, &text, id);

    let after_mark = agents.run_as(CLAUDE).shown();

    assert_not_handed(&after_mark, HEAD);
}
