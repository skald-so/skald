//! The hooks print nothing while nothing waits for their agent, so `Stop` lets the turn end; a
//! failing run prints nothing on its standard output and never exits with code 2.

use crate::codex::{Codex, Event, hook_command};
use crate::hooks::run_hook;
use crate::lines::assert_handed_line;
use crate::solo::WEB;

/// The exit code with which Codex blocks `PostToolUse` or keeps a turn going.
const BLOCKS: i32 = 2;

/// web's session starts and nothing waits for web: its `SessionStart`, `UserPromptSubmit`,
/// `PostToolUse` and `Stop` hooks, `Stop` also in a turn a hook kept going, print nothing and exit
/// with success, each twice.
#[test]
fn hooks_print_nothing_while_nothing_waits() {
    let codex = Codex::new(4_927_001_001, "ada-codex-hooks-idle");
    let web = codex.thread_of(WEB);

    for round in ["first", "second"] {
        for event in Event::ALL {
            codex.silent(&web, event, round);
        }
    }
}

/// What api says to web; it is nowhere else.
const WAITS: &str = "codex hook test: api asks web to look at the failing hook";

/// web's session starts, and api sends web a message. Its hooks run on an input cut in its middle
/// and on one that is no JSON: each prints nothing on its standard output and exits with a code
/// other than 2. web's `inbox` then shows the message.
#[test]
fn hook_on_a_broken_input_prints_nothing_and_never_blocks() {
    let mut codex = Codex::new(4_927_002_001, "ada-codex-hooks-broken");
    let web = codex.thread_of(WEB);
    codex.starts(&web);
    let msg_id = codex.web.own(WAITS);
    let input = web.input(Event::PostToolUse);
    let cut = &input[..input.len() / 2];

    for (broken, what) in [(cut, "a cut input"), ("not json\n", "no JSON")] {
        let hooked = run_hook(&mut hook_command(&codex.helped, &web, &[]), broken);
        assert!(hooked.stdout.is_empty(), "{what}: printed {hooked:?}");
        assert!(hooked.code.is_some(), "{what}: no exit code: {hooked:?}");
        assert_ne!(hooked.code, Some(BLOCKS), "{what}: {hooked:?}");
    }

    assert_handed_line(&codex.web.session.inbox(), WAITS, msg_id);
}
