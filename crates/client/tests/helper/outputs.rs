//! Through the helper, output counts as shown only once the command has written it: a command
//! whose output fails shows nothing as shown, as in one-shot mode.

use crate::gaps::{bob_in, bob_sends};
use crate::helped::{Helped, ONE_SHOT, times};
use crate::kit::RECV;
use crate::talk::{BOB_HOST, Talk, assert_shows_once};

/// What Bob's device says; it is nowhere else.
const BOB_SAYS: &str = "helper test: the invoice run is done";

/// What a run says when it cannot write a message to its output.
const UNWRITTEN: &str = "cannot write a message to the output";

/// Bob's device sends a message to the channel. Ada's `skald recv` with its standard output
/// closed fails in one-shot mode, saying it cannot write the message; so does the next one, which
/// starts a helper and goes through it. The next `skald recv`, through the same helper, shows the
/// message once.
#[test]
fn recv_whose_output_fails_through_the_helper_shows_the_message_again_as_in_one_shot_mode() {
    let mut talk = Talk::new(
        &[4_146_061_001, 4_146_061_002],
        &["ada-closes-her-output", "bob-is-read-later"],
        false,
    );
    let helped = Helped::new(&talk.ada.machine);
    let (from_bob, channel, _) = bob_in(&talk);
    bob_sends(&mut talk, 1, BOB_SAYS);

    let one_shot = helped.run_with_closed_output(&[ONE_SHOT, RECV]);

    assert!(!one_shot.success, "{one_shot:?}");
    assert!(one_shot.stderr.contains(UNWRITTEN), "{one_shot:?}");
    helped.assert_none();

    let through = helped.run_with_closed_output(&[RECV]);

    assert!(!through.success, "{through:?}");
    assert!(through.stderr.contains(UNWRITTEN), "{through:?}");
    let helper = helped.one_helper();
    let shown = helped.received().shown();
    assert_shows_once(&shown, BOB_SAYS, (channel, &from_bob, BOB_HOST));
    assert_eq!(times(&shown, BOB_SAYS), 1, "{shown}");
    assert_eq!(helped.helpers(), [helper], "not the same helper");
}
