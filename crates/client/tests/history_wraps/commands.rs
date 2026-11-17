//! A Commit an admin command makes, with no catch-up after it, uploads the new epoch's wraps.

use crate::chain::{carol, joins_space};
use crate::edits::{Edit, edited};
use crate::kit::received;
use crate::talk::Talk;
use crate::wraps::{assert_wrapped, sent_to, valid_browser};

/// The test sets up a browser of Ada's account, and Ada's next run wraps epoch 1 to it. Carol joins
/// the space by a revision Ada's device signs. Ada's `skald add-to-channel` adds Carol to the talk's
/// channel and commits the Add of Carol's device: epoch 2. That run uploads the wrap of epoch 2 to
/// the browser, which the server holds, signed by Ada's device.
#[test]
fn commit_of_an_admin_command_uploads_the_new_epoch_s_wraps() {
    let mut talk = Talk::new(
        &[7_300_013_001, 7_300_013_002],
        &["ada-adds-by-command", "bob-sees-command-add"],
        false,
    );
    let browser = valid_browser(&talk.world.cloud, &talk.ada, "ada-browser-of-admin");
    received(&talk.ada.machine);
    assert_wrapped(&mut talk, &browser, 1);
    let carol = carol(&talk, 7_300_013_003, "carol-added-by-command");
    joins_space(talk.cloud(), &talk.ada, talk.space.id, &carol.person);
    let uploads = talk.front.uploads().len();

    edited(
        &talk.ada.machine,
        &talk.space.name,
        Edit::AddToChannel(talk.channel, &carol.person, true),
    );

    let sent = sent_to(&talk.front, talk.channel, uploads, browser.id);
    assert!(
        sent.iter().any(|one| one.epoch == 2 && one.forwarded),
        "the command's run uploaded no wrap of epoch 2: {sent:?}"
    );
    assert_wrapped(&mut talk, &browser, 2);
}
