//! The sender of a Commit wraps the new epoch's key to each valid browser of the channel's member
//! accounts, and a browser reads with it that epoch's entries and, through the links, earlier ones.

use std::collections::BTreeMap;

use crate::chain::{adds_to_channel, assert_chain, carol};
use crate::entries::ada_s_message;
use crate::kit::received;
use crate::talk::Talk;
use crate::wraps::{assert_reads, sent_to, valid_browser};

/// Ada's `skald send` sends a message at epoch 1. The test then sets up a browser of Ada's account
/// and one of Bob's, each certified by its account's device. A revision Ada's device signs adds
/// Carol to the channel, and Ada's next run commits the Add of Carol's device: epoch 2, and that
/// run uploads a wrap of epoch 2 to each browser. Ada sends another message at epoch 2. Each
/// browser opens its one wrap of epoch 2 with its HPKE key; it is signed by Ada's device and gives
/// the key of epoch 2 Bob's device derives. With it and the links the server holds, each browser
/// reads both of Ada's entries, each under its epoch.
#[test]
fn commit_s_sender_wraps_the_new_epoch_s_key_to_each_browser_of_the_members() {
    let mut talk = Talk::new(
        &[7_300_001_001, 7_300_001_002],
        &["ada-wraps-to-browsers", "bob-has-a-browser"],
        false,
    );
    let carol = carol(&talk, 7_300_001_003, "carol-joins-wrapped");
    let from = talk.front.sent().len();
    let sent = talk.send("a line before the browsers");
    assert!(sent.success, "{sent:?}");
    let first = ada_s_message(&talk, from);
    let cloud = &talk.world.cloud;
    let browsers = [
        valid_browser(cloud, &talk.ada, "ada-web-browser"),
        valid_browser(cloud, &talk.played[0].person, "bob-web-browser"),
    ];
    adds_to_channel(cloud, &talk.ada, talk.at(), &carol.person);
    let uploads = talk.front.uploads().len();

    received(&talk.ada.machine);

    let devices = [talk.ada.device, talk.played[0].device(), carol.device()];
    assert_chain(&talk, (2, &devices), &[(2, 1)]);
    for browser in &browsers {
        let sent = sent_to(&talk.front, talk.channel, uploads, browser.id);
        assert!(
            sent.iter().any(|one| one.epoch == 2 && one.forwarded),
            "the Commit's run uploaded no wrap of epoch 2 to {}: {sent:?}",
            browser.id
        );
    }
    let from = talk.front.sent().len();
    let sent = talk.send("a line after the Commit");
    assert!(sent.success, "{sent:?}");
    let second = ada_s_message(&talk, from);
    let read = BTreeMap::from([(1, vec![first]), (2, vec![second])]);
    for browser in &browsers {
        assert_reads(&mut talk, (browser, 2), &read);
    }
}
