//! Once a member device has learned of an account's deletion, it uploads no link, wrap or entry of
//! an epoch with the account's devices, and no wrap to the account's recovery key or browsers.

use skald_proto::{
    Cbor as _, HistoryEntry, HistoryItem, HistoryLink, HistoryWrap, Role, WrapRecipient,
};

use crate::chain::uploads_in;
use crate::fixtures::send;
use crate::kit::received;
use crate::scene::{Scene, commits_sent, delete_account};
use crate::wraps::valid_browser;

/// What Bob says after Ada's deletion, in the channel and in the default channel.
const AFTER: [&str; 2] = [
    "the audit starts next week",
    "lunch is on the fourth floor today",
];

/// The host name of Ada's browser, its certificate's `name`.
const BROWSER: &str = "ada-browser";

/// The epoch of a history item, and whom it is sealed to if it is a wrap; `None` for a queue key
/// object.
fn epoch_and_recipient(item: &HistoryItem) -> Option<(u64, Option<WrapRecipient>)> {
    match item {
        HistoryItem::Link(bytes) => {
            let link = HistoryLink::from_cbor(bytes).expect("a link that decodes");
            Some((link.epoch, None))
        }
        HistoryItem::Wrap(bytes) => {
            let wrap = HistoryWrap::from_cbor(bytes).expect("a wrap that decodes");
            Some((wrap.body().epoch, Some(wrap.body().recipient)))
        }
        HistoryItem::Entry { entry, .. } => {
            let entry = HistoryEntry::from_cbor(entry).expect("an entry that decodes");
            Some((entry.epoch, None))
        }
        HistoryItem::QueueKey(_) => None,
    }
}

/// Ada, a member of `acme` with a linked browser, deletes her account. Bob's next runs, `skald
/// send` in the channel and then in the default channel, remove Ada's devices from each channel by
/// one Commit and say a line in each. Every link, wrap and entry of each channel Bob's machine
/// uploads from Ada's deletion on is of an epoch after that of the channel's Commit, which Ada's
/// devices are in; none of its wraps goes to Ada's recovery key or her browser; and it uploads an
/// entry in each channel.
#[test]
fn after_learning_of_a_deletion_no_history_item_of_an_epoch_with_its_devices_is_uploaded() {
    let scene = Scene::new(
        [4_305_103_001, 4_305_103_002, 4_305_103_003],
        [
            "bob-uploads-after",
            "ada-with-a-browser",
            "carol-beside-them",
        ],
        Role::Member,
    );
    let cloud = scene.cloud();
    let browser = valid_browser(cloud, scene.ada(), BROWSER).id;
    received(&scene.bob.machine);
    let ada = scene.ada().account;
    delete_account(cloud, scene.ada());
    let (from, uploaded) = (scene.front.sent().len(), scene.front.uploads().len());

    for (channel, text) in [scene.channel, scene.space.default].into_iter().zip(AFTER) {
        let sent = send(&scene.bob.machine, channel, text);
        assert!(sent.success, "{sent:?}");
    }

    let barred = [
        WrapRecipient::Recovery(ada),
        WrapRecipient::Browser(browser),
    ];
    for channel in scene.channels() {
        let removals = commits_sent(&scene.front, channel, from);
        assert_eq!(removals.len(), 1, "not one Commit in {channel}");
        let removal = removals[0];
        let items: Vec<(u64, Option<WrapRecipient>, bool)> =
            (uploads_in(&scene.front, channel, uploaded).iter())
                .filter_map(|upload| {
                    let entry = matches!(upload.item, HistoryItem::Entry { .. });
                    epoch_and_recipient(&upload.item).map(|(epoch, to)| (epoch, to, entry))
                })
                .collect();
        for (epoch, to, _) in &items {
            assert!(
                *epoch > removal,
                "an item of epoch {epoch} in {channel}, which Ada's devices are in: {items:?}"
            );
            assert!(
                !to.is_some_and(|to| barred.contains(&to)),
                "a wrap to Ada in {channel}: {items:?}"
            );
        }
        assert!(
            items.iter().any(|(_, _, entry)| *entry),
            "no entry uploaded in {channel}: {items:?}"
        );
    }
}
