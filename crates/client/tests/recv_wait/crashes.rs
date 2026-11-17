//! A helper or a session that dies during a handover: the message is handed over again, never lost,
//! and never again once the session has written it out. A session killed while it waits keeps its
//! helper from exiting idle no longer.
//!
//! As in the tests of handing messages over (`agent_handover/held.rs`), the message's text is far
//! longer than a pipe holds, so a session whose output the test stops reading waits in the middle
//! of writing it.

use std::thread;
use std::time::Instant;

use skald_proto::MessageId;

use crate::agents::CLAUDE;
use crate::held::{FILL, HEAD, Held, long_text};
use crate::helped::{EXIT_LIMIT, IDLE_SETTING};
use crate::kit::RECV;
use crate::talk::Line;
use crate::waits::{
    AT_ONCE, WAIT_FLAG, Waits, assert_got, assert_keeps_waiting, await_helper_beside,
};
use crate::world::AGENT;

/// What Bob's device says after the long message; it is nowhere else.
const AFTER: &str = "crash test: the export was imported";

/// The helper's idle time, in seconds, in the test of a session killed while it waits: under
/// [`crate::waits::IDLE_WATCH`].
const SHORT_IDLE: &str = "1";

/// A handover in the middle: claude's session, through the helper, waits in the middle of writing
/// a message with a long text.
struct Midway {
    session: Held,
    waits: Waits,
    helper: u32,
    text: String,
    msg_id: MessageId,
}

impl Midway {
    /// Ada and Bob, GitHub users `ids` with `logins`. The device has run as claude in
    /// `~/work/acme`, through a helper; claude waits through it, and Bob's device sends a message
    /// with a long text, which the session starts to write.
    fn start(ids: [u64; 2], logins: [&str; 2]) -> Self {
        let mut waits = Waits::new(ids, logins);
        waits.helped_as(CLAUDE);
        let helper = waits.helped.one_helper();
        let args = [AGENT, CLAUDE, RECV, WAIT_FLAG];
        let mut command = waits.helped.command_in(&waits.agents.folder, &[], &args);
        let mut session = Held::spawn(&mut command);
        let bob = waits.agents.bob();
        let text = long_text();
        let msg_id = waits.agents.bob_sends(&Line::new(&bob, 1, &text));
        session.wait_for(HEAD);
        thread::sleep(FILL);
        Midway {
            session,
            waits,
            helper,
            text,
            msg_id,
        }
    }

    /// Kills the session; fails if it wrote the text whole.
    #[track_caller]
    fn kill_session(&mut self) {
        let cut = self.session.kill();
        assert!(!cut.contains(&self.text), "the text written whole");
    }
}

/// claude's session waits in the middle of writing a message with a long text when the helper is
/// killed, then the session: it never wrote the text whole. claude's next `skald recv --wait`
/// starts a new helper and shows the text once, by its reference. That helper is killed once the
/// session has exited. Bob's device sends another message, and claude's next `skald recv --wait`
/// shows only that one.
#[test]
fn helper_killed_before_the_mark_hands_the_message_over_again_and_one_killed_after_it_does_not() {
    let mut midway = Midway::start(
        [4_148_021_001, 4_148_021_002],
        ["ada-loses-helper-mid-handover", "bob-sends-a-long-export"],
    );

    midway.waits.helped.kill(midway.helper);

    midway.kill_session();
    let waits = &mut midway.waits;
    let again = waits.session(CLAUDE).finish();
    assert_got(&again, &midway.text, midway.msg_id);
    let restarted = waits.helped.one_helper();
    assert_ne!(restarted, midway.helper, "the killed helper still runs");

    waits.helped.kill(restarted);

    let bob = waits.agents.bob();
    let after = waits.agents.bob_sends(&Line::new(&bob, 2, AFTER));
    let next = waits.session(CLAUDE).finish();
    assert_got(&next, AFTER, after);
    assert!(!next.shown().contains(HEAD), "handed over again: {next:?}");
}

/// claude's session is killed while it waits in the middle of writing a message with a long text:
/// it never wrote the text whole. claude's next `skald recv --wait`, through the same helper, shows
/// the text once, by its reference, and exits within [`AT_ONCE`].
#[test]
fn session_killed_before_writing_its_message_leaves_it_to_the_next_wait() {
    let mut midway = Midway::start(
        [4_148_031_001, 4_148_031_002],
        ["ada-closes-her-session", "bob-sends-the-export-twice"],
    );

    midway.kill_session();

    let helped = &midway.waits.helped;
    assert_eq!(helped.helpers(), [midway.helper], "not the same helper");
    let started = Instant::now();
    let next = midway.waits.session(CLAUDE).finish();
    let took = started.elapsed();
    assert_got(&next, &midway.text, midway.msg_id);
    assert!(took < AT_ONCE, "it took {took:?}: {next:?}");
    assert_eq!(helped.helpers(), [midway.helper], "not the same helper");
}

/// claude's `skald recv --wait` starts a helper with an idle time of [`SHORT_IDLE`] and waits
/// through it while nothing comes, longer than that idle time. Once the session is killed, the
/// helper exits within [`EXIT_LIMIT`].
#[test]
fn session_killed_while_it_waits_leaves_its_helper_to_exit_idle() {
    let waits = Waits::new(
        [4_148_032_001, 4_148_032_002],
        ["ada-walks-away-mid-wait", "bob-stays-silent"],
    );
    let mut session = waits.session_with(&[(IDLE_SETTING, SHORT_IDLE)], CLAUDE);
    let helper = await_helper_beside(&waits.helped, &[&session]);
    assert_keeps_waiting(&mut session);

    session.kill();

    waits.helped.await_exit(helper, EXIT_LIMIT);
}
