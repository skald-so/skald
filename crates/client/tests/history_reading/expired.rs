//! A device offline past the mailbox retention shows in the history the messages that expired
//! before reaching it.

use std::time::Duration;

use skald_server::Settings;

use crate::kit::received;
use crate::reading::{Duo, assert_history, rotates};
use crate::times::{MAILBOX, WEEK, moved_back};

/// Gives the server a mailbox retention of 7 days.
fn retained(settings: &mut Settings) {
    settings.mailbox_retention = Some(Duration::from_secs(WEEK));
}

/// With a retention of 7 days: Bob says a line, which Ada's `skald recv` shows; then her machine
/// stays offline. Bob says another, commits an empty Commit, epoch 2, and says a third. With every
/// storing time moved back 8 days, Ada's next `skald recv` shows neither expired message; her
/// `skald history` shows all three, oldest first.
#[test]
fn messages_that_expired_before_reaching_the_device_show_in_its_history() {
    let mut duo = Duo::with(
        &[4_166_006_001, 4_166_006_002],
        &["ada-back-from-leave", "bob-writes-meanwhile"],
        retained,
    );
    let (first, ..) = duo.bob_says(&duo.bob_s(1, "the archive move starts"));
    let ada = &duo.talk.ada.machine;
    let shown = received(ada).shown();
    assert!(shown.contains(&first.text), "{shown}");
    let (second, ..) = duo.bob_says(&duo.bob_s(2, "half the archive moved"));
    let cloud = &duo.talk.world.cloud;
    let epoch = rotates(cloud, &mut duo.talk.played[0], duo.at);
    assert_eq!(epoch, 2, "the epoch of Bob's empty Commit");
    let (third, ..) = duo.bob_says(&duo.bob_s(3, "the archive move is done"));
    let cloud = &duo.talk.world.cloud;
    cloud.execute(&moved_back(&MAILBOX, "8 days"));
    let back = received(&duo.talk.ada.machine).shown();
    for said in [&second, &third] {
        assert!(!back.contains(&said.text), "not expired: {back}");
    }

    let shown = duo.ada_s_history();

    assert_history(&shown, duo.at.1, &[first, second, third], &[]);
}
