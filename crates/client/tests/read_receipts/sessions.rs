//! `read` follows a handover by the helper's intake to a session that waits through it.

use crate::agents::CLAUDE;
use crate::helped::Helped;
use crate::kit::RECV;
use crate::receipts::{Roles, assert_shows_once, shows_read_once};
use crate::waits::{TRACE_SETTING, TRACED, WAIT_FLAG, assert_keeps_waiting};
use crate::world::AGENT;

/// What A's agent sends; it is nowhere else.
const TEXT: &str = "receipt session test: claude, compact the event log";

/// Through a helper, claude's `skald recv --wait` waits in B's `~/work/acme`. A's agent sends a
/// message `--to` claude, which the helper's intake hands to the session; A's runs then show
/// `read` of it once, with no other run on B's machine.
#[test]
fn read_shows_once_a_session_waiting_through_the_helper_has_the_message() {
    let roles = Roles::new(
        [4_155_010_001, 4_155_010_002],
        ["ada-agent-waits-helped", "bob-beside-the-helper"],
    );
    // Dropped first: it kills the helper before the machine's directory goes.
    let helped = Helped::new(roles.b());
    let args = [AGENT, CLAUDE, RECV, WAIT_FLAG];
    let traced = [(TRACE_SETTING, "1")];
    let mut session = helped.start_in(&roles.senders.agents.folder, &traced, &args);
    assert_keeps_waiting(&mut session);

    let msg_id = roles.a_sends_to_claude(TEXT);

    let got = session.finish();
    assert!(got.success, "{got:?}");
    assert_shows_once(&got.stdout, TEXT);
    assert!(got.stderr.contains(TRACED), "not the intake's: {got:?}");
    shows_read_once(roles.a(), msg_id);
}
