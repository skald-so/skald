//! A failing run of the hook handler prints nothing on its standard output and never exits with
//! code 2, with which Claude Code would keep the turn going; a message it could not give still
//! waits.

use std::fs;

use crate::hooks::{Event, Hooked, Web, hook_command, run_hook};
use crate::lines::assert_handed_line;
use crate::machine::STATE_DIR;
use crate::solo::WEB;

/// The exit code with which a `Stop` hook keeps the turn going.
const KEEPS_GOING: i32 = 2;

/// Fails unless `hooked` printed nothing on its standard output and exited with a code other than
/// [`KEEPS_GOING`].
#[track_caller]
fn assert_ends_the_turn(hooked: &Hooked, what: &str) {
    assert!(hooked.stdout.is_empty(), "{what}: printed {hooked:?}");
    assert_ne!(hooked.code, Some(KEEPS_GOING), "{what}: {hooked:?}");
    assert!(hooked.code.is_some(), "{what}: no exit code: {hooked:?}");
}

/// What api says to web; it is nowhere else.
const WAITS: &str = "hook test: api asks web to check the failing hook";

/// api sends web a message. web's hook handler run on an input that is cut in its middle, and on
/// one that is no JSON, prints nothing on its standard output and does not exit with code 2; web's
/// `inbox` then shows the message.
#[test]
fn hook_on_a_broken_input_ends_the_turn_and_leaves_the_message_waiting() {
    let mut web = Web::new(4_913_008_001, "ada-hooks-a-broken-input");
    let msg_id = web.own(WAITS);
    let input = Event::Stop.input(&web.dir());
    let cut = &input[..input.len() / 2];

    for (broken, what) in [(cut, "a cut input"), ("not json\n", "no JSON")] {
        let hooked = run_hook(
            &mut hook_command(web.machine(), &web.dir(), Some(WEB)),
            broken,
        );
        assert_ends_the_turn(&hooked, what);
    }

    assert_handed_line(&web.session.inbox(), WAITS, msg_id);
}

/// web's hook handler run on `Stop` and `PostToolUse` with a state directory that is a file prints
/// nothing on its standard output and does not exit with code 2.
#[test]
fn hook_that_cannot_open_the_local_state_ends_the_turn() {
    let web = Web::new(4_913_009_001, "ada-hooks-a-broken-state");
    let file = web
        .machine()
        .root()
        .join("tmp")
        .join("not-a-state-directory");
    fs::write(&file, "a file where the state directory should be").expect("write a file");

    for event in [Event::Stop, Event::PostToolUse] {
        let mut command = hook_command(web.machine(), &web.dir(), Some(WEB));
        command.env(STATE_DIR, &file);
        let hooked = run_hook(&mut command, &event.input(&web.dir()));
        assert_ends_the_turn(&hooked, event.name());
    }
}
