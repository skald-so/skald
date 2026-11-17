//! No epoch judged unclean is wrapped to a browser.

use crate::chain::assert_chain;
use crate::joins::{intruder, intruder_joins};
use crate::kit::received;
use crate::talk::Talk;
use crate::wraps::{assert_wrapped, held_epochs, sent_to, valid_browser};

/// The test sets up a browser of Ada's account. A device the server lets in on Bob's account, its
/// certificate chained to another master key, joins the channel's group by itself: epoch 2, which
/// Ada's `skald recv` judges unclean before it commits the device's Remove: epoch 3. The browser
/// gets the wrap of epoch 3, signed by Ada's device; the server holds no wrap of epoch 2, and
/// neither that run nor the next sent one.
#[test]
fn epoch_judged_unclean_is_wrapped_to_no_browser() {
    let mut talk = Talk::new(
        &[7_300_007_001, 7_300_007_002],
        &["ada-wraps-clean-only", "bob-account-lent"],
        false,
    );
    let browser = valid_browser(&talk.world.cloud, &talk.ada, "ada-careful-browser");
    let intruder = intruder(
        talk.cloud(),
        &talk.played[0].person,
        "bob-server-made-device",
    );
    intruder_joins(talk.cloud(), &intruder, talk.at());
    let uploads = talk.front.uploads().len();

    received(&talk.ada.machine);
    received(&talk.ada.machine);

    assert_wrapped(&mut talk, &browser, 3);
    let members = [talk.ada.device, talk.played[0].device()];
    assert_chain(&talk, (3, &members), &[(3, 1)]);
    let held = held_epochs(&talk, browser.id);
    assert!(!held.contains(&2), "epoch 2 wrapped: {held:?}");
    let sent = sent_to(&talk.front, talk.channel, uploads, browser.id);
    assert!(
        sent.iter().all(|one| one.epoch != 2),
        "a wrap of epoch 2 sent: {sent:?}"
    );
}
