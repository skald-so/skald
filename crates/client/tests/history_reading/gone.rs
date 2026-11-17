//! Messages of a member removed from the channel since, and of a device revoked since, still show:
//! the sender's certificate is checked revoked or not, against any revision the device accepted.

use crate::chain::removes_from_channel;
use crate::kit::{group, received};
use crate::reading::{assert_history, says, sends, shown_history};
use crate::revocations::revoked_by;
use crate::talk::{Line, Talk};

/// Carol and Bob each say a line, and Ada sends one. A revision Ada's device signs takes Carol out
/// of the channel, and Ada's run commits the Remove of her device; Bob's device revokes itself, and
/// Ada's run commits its Remove: the group holds Ada's device alone. Ada's `skald history` still
/// shows the three lines, oldest first.
#[test]
fn messages_of_a_member_removed_since_and_of_a_device_revoked_since_still_show() {
    let mut talk = Talk::new(
        &[4_166_007_001, 4_166_007_002, 4_166_007_003],
        &[
            "ada-keeps-records",
            "bob-revokes-laptop",
            "carol-leaves-team",
        ],
        false,
    );
    let (at, space) = (talk.at(), talk.space.name.clone());
    let from_bob = talk.address(&talk.played[0].person);
    let from_carol = talk.address(&talk.played[1].person);
    let cloud = &talk.world.cloud;
    let line = Line::new(&from_carol, 1, "carol hands over her notes");
    let (carol_s, ..) = says(cloud, &mut talk.played[1], at, &line);
    let line = Line::new(&from_bob, 1, "bob signs off from this laptop");
    let (bob_s, ..) = says(cloud, &mut talk.played[0], at, &line);
    let ada_s = sends(&talk.ada, (&space, at.1), "ada thanks them both");
    removes_from_channel(cloud, &talk.ada, at, &talk.played[1].person);
    received(&talk.ada.machine);
    let bob = &talk.played[0];
    revoked_by(cloud, &bob.person, bob.device());
    received(&talk.ada.machine);
    group(cloud, &talk.ada.token, at.0, at.1).assert_is(3, &[talk.ada.device]);

    let shown = shown_history(&talk.ada.machine, at.1);

    assert_history(&shown, at.1, &[carol_s, bob_s, ada_s], &[]);
}
