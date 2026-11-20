//! What the tests share: browsers the tests write to the server's database, the wraps of a channel
//! the server gives and those Ada's machine uploads, opening them with a browser's keys, and
//! reading the channel's entries with the key a wrap gives.

use std::collections::{BTreeMap, BTreeSet};

use skald_crypto::{HistoryKey, HpkeKey, follow_links, open_wrap};
use skald_proto::{
    Cbor as _, ChannelId, DeviceId, DeviceKind, HistoryEntry, HistoryItem, HistoryWrap, MessageId,
    SpaceId, WrapRecipient,
};

use crate::chain::{key_of, links, listed, same, uploads_in};
use crate::cloud::Cloud;
use crate::entries::talk_entries;
use crate::fixtures::{Issuer, Keyed, add_device_of_kind, cert_of};
use crate::front::Front;
use crate::talk::Talk;
use crate::world::Person;

/// A `browser` device of `owner`'s account named `name`, written to the server's database, its
/// certificate issued as `issuer` says: by `owner`'s device, a valid one.
pub fn linked_browser(cloud: &Cloud, owner: &Person, name: &str, issuer: &Issuer) -> Keyed {
    add_device_of_kind(cloud, (owner.account, DeviceKind::Browser), name, issuer)
}

/// As [`linked_browser`], certified by `owner`'s device.
pub fn valid_browser(cloud: &Cloud, owner: &Person, name: &str) -> Keyed {
    linked_browser(cloud, owner, name, &Issuer::Device(owner))
}

/// The wraps of `at`'s channel the server gives the session `token`, each as it decodes.
#[track_caller]
pub fn wraps(cloud: &Cloud, token: &str, at: (SpaceId, ChannelId)) -> Vec<HistoryWrap> {
    (listed(cloud, token, at, "wraps").iter())
        .map(|bytes| {
            HistoryWrap::from_cbor(bytes)
                .unwrap_or_else(|err| panic!("a wrap that does not decode: {err}"))
        })
        .collect()
}

/// As [`wraps`], of the talk's channel, for the session of Ada's machine.
#[track_caller]
pub fn talk_wraps(talk: &Talk) -> Vec<HistoryWrap> {
    wraps(&talk.world.cloud, &talk.ada.token, talk.at())
}

/// The epochs of the wraps among `wraps` to the browser `browser`, each as often as it comes.
pub fn epochs_to(wraps: &[HistoryWrap], browser: DeviceId) -> Vec<u64> {
    wraps
        .iter()
        .filter(|wrap| wrap.body().recipient == WrapRecipient::Browser(browser))
        .map(|wrap| wrap.body().epoch)
        .collect()
}

/// The epochs of the wraps of the talk's channel the server holds to `browser`, each once.
#[track_caller]
pub fn held_epochs(talk: &Talk, browser: DeviceId) -> BTreeSet<u64> {
    epochs_to(&talk_wraps(talk), browser).into_iter().collect()
}

/// A wrap of `channel` a machine sent in a `history` frame: its epoch and recipient, and whether
/// the front forwarded it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SentWrap {
    pub epoch: u64,
    pub recipient: WrapRecipient,
    pub forwarded: bool,
}

/// The wraps of `channel` the front saw the machines send after its first `from` uploads, in
/// order.
pub fn wraps_sent(front: &Front, channel: ChannelId, from: usize) -> Vec<SentWrap> {
    (uploads_in(front, channel, from).into_iter())
        .filter_map(|upload| match &upload.item {
            HistoryItem::Wrap(bytes) => {
                let wrap = HistoryWrap::from_cbor(bytes)
                    .unwrap_or_else(|err| panic!("a wrap sent that does not decode: {err}"));
                Some(SentWrap {
                    epoch: wrap.body().epoch,
                    recipient: wrap.body().recipient,
                    forwarded: upload.forwarded,
                })
            }
            _ => None,
        })
        .collect()
}

/// Of [`wraps_sent`], those to `browser`.
pub fn sent_to(front: &Front, channel: ChannelId, from: usize, browser: DeviceId) -> Vec<SentWrap> {
    wraps_sent(front, channel, from)
        .into_iter()
        .filter(|sent| sent.recipient == WrapRecipient::Browser(browser))
        .collect()
}

/// Fails if the server holds a wrap of the talk's channel to `browser`, or the front saw one sent
/// after its first `from` uploads.
#[track_caller]
pub fn assert_no_wrap_to(talk: &Talk, from: usize, browser: DeviceId) {
    let held = held_epochs(talk, browser);
    assert!(
        held.is_empty(),
        "the server holds wraps to {browser} of epochs {held:?}"
    );
    let sent = sent_to(&talk.front, talk.channel, from, browser);
    assert!(sent.is_empty(), "wraps to {browser} sent: {sent:?}");
}

/// The key of the one wrap of the talk's channel and `epoch` that the server holds to `browser`:
/// fails unless it is of the channel, to `browser`, opens with `browser`'s HPKE key, is signed by
/// Ada's device, as its `signer` says, and gives the key of `epoch` Bob's device derives. Bob's
/// device takes in its mailbox first.
#[track_caller]
pub fn assert_wrapped(talk: &mut Talk, browser: &Keyed, epoch: u64) -> HistoryKey {
    let recipient = WrapRecipient::Browser(browser.id);
    assert_wrapped_as(talk, (recipient, &browser.hpke), epoch)
}

/// As [`assert_wrapped`], of the wrap to `recipient`, opening with `hpke`, its HPKE key pair.
#[track_caller]
pub fn assert_wrapped_as(
    talk: &mut Talk,
    (recipient, hpke): (WrapRecipient, &HpkeKey),
    epoch: u64,
) -> HistoryKey {
    let held: Vec<HistoryWrap> = talk_wraps(talk)
        .into_iter()
        .filter(|wrap| wrap.body().recipient == recipient)
        .filter(|wrap| wrap.body().epoch == epoch)
        .collect();
    assert_eq!(
        held.len(),
        1,
        "not one wrap to {recipient:?} of epoch {epoch}: {held:?}"
    );
    let wrap = &held[0];
    assert_eq!(wrap.body().channel, talk.channel, "the channel of {wrap:?}");
    let opened = open_wrap(wrap, hpke).unwrap_or_else(|err| {
        panic!("the wrap to {recipient:?} of epoch {epoch} does not open: {err:?}")
    });
    assert_eq!(
        opened.signer(),
        talk.ada.device,
        "the signer of the wrap of epoch {epoch}"
    );
    let cert = cert_of(&talk.world.cloud, &talk.ada);
    let key = opened.verify(&cert).unwrap_or_else(|err| {
        panic!("the wrap of epoch {epoch} is not signed by Ada's device: {err:?}")
    });
    talk.played[0].take_in(&talk.world.cloud);
    assert!(
        same(&key, key_of(&talk.played[0], talk.channel, epoch)),
        "the wrap to {recipient:?} gives a wrong key of epoch {epoch}"
    );
    key
}

/// The `msg_id` of each entry of the talk's channel the server holds that opens with `key`, the
/// key of `epoch`, or with a key the links the server holds reach back from it, and whose
/// signature verifies with the certificate of Ada's device; by epoch.
pub fn read_with(talk: &Talk, (epoch, key): (u64, &HistoryKey)) -> BTreeMap<u64, Vec<MessageId>> {
    let (cloud, at) = (&talk.world.cloud, talk.at());
    let held_links = links(cloud, &talk.ada.token, at);
    let mut keys = follow_links(at.1, epoch, key, &held_links);
    keys.insert(epoch, HistoryKey::from_bytes(&key.to_bytes()));
    let cert = cert_of(cloud, &talk.ada);
    let mut read: BTreeMap<u64, Vec<MessageId>> = BTreeMap::new();
    for bytes in talk_entries(talk) {
        let Ok(entry) = HistoryEntry::from_cbor(&bytes) else {
            continue;
        };
        let Some(key) = keys.get(&entry.epoch) else {
            continue;
        };
        let Ok(message) = key.open_entry(&entry) else {
            continue;
        };
        if let Ok(body) = message.verify(&cert) {
            read.entry(entry.epoch).or_default().push(body.msg_id);
        }
    }
    read
}

/// Fails unless `browser` opens its wrap of `epoch` as [`assert_wrapped`] says, and reads with the
/// key it gives, as [`read_with`] says, the entries of `expected`, by epoch.
#[track_caller]
pub fn assert_reads(
    talk: &mut Talk,
    (browser, epoch): (&Keyed, u64),
    expected: &BTreeMap<u64, Vec<MessageId>>,
) {
    let key = assert_wrapped(talk, browser, epoch);
    assert_eq!(
        &read_with(talk, (epoch, &key)),
        expected,
        "the entries {} reads with its wrap of epoch {epoch}",
        browser.id
    );
}
