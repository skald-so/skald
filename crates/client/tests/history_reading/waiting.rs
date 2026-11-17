//! While a revision the device signed waits for the server, the history still checks senders
//! against every revision the device accepted.

use crate::chain::removes_from_channel;
use crate::kit::received;
use crate::reading::{assert_history, says, shown_history};
use crate::talk::{Line, Talk};
use crate::world::{run, store_request};

/// Carol and Bob each say a line. A revision Ada's device signs takes Carol out of the channel,
/// and Ada's run commits the Remove of her device. With the server failing to store revisions,
/// Ada's `skald set-history-period` leaves its revision waiting. Ada's `skald history`, the
/// revision still waiting, shows both lines, oldest first.
#[test]
fn messages_of_a_member_removed_since_show_while_a_revision_of_the_device_waits() {
    let mut talk = Talk::new(
        &[4_166_015_001, 4_166_015_002, 4_166_015_003],
        &["ada-waits-on-policy", "bob-stays-on", "carol-moves-out"],
        false,
    );
    let at = talk.at();
    let mut said = Vec::new();
    // Carol, the second played device, then Bob, the first.
    for (played, text) in [(1, "carol leaves her notes"), (0, "bob takes them over")] {
        let line = Line::new(&talk.address(&talk.played[played].person), 1, text);
        said.push(says(&talk.world.cloud, &mut talk.played[played], at, &line).0);
    }
    let cloud = &talk.world.cloud;
    removes_from_channel(cloud, &talk.ada, at, &talk.played[1].person);
    received(&talk.ada.machine);
    cloud
        .relay
        .refuse(&store_request(at.0), "503 Service Unavailable");
    let space = talk.space.name.clone();
    let set = run(&talk.ada.machine, &["set-history-period", &space, "7d"]);
    assert!(
        !set.success && set.stderr.contains("waits for the server"),
        "the revision does not wait: {set:?}"
    );

    let shown = shown_history(&talk.ada.machine, at.1);

    assert_history(&shown, at.1, &said, &[]);
}
