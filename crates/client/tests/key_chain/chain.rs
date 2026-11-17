//! What the tests share: the history items of a channel the server gives and those the machines
//! upload; the links among them, opening them with the played devices' history keys; and the
//! Commits, joins and links of the played devices.

use std::collections::BTreeSet;

use skald_crypto::{HistoryKey, follow_links};
use skald_proto::{Cbor as _, ChannelId, DeviceId, Frame, HistoryItem, HistoryLink, Role, SpaceId};

use crate::cloud::{Cloud, unb64};
use crate::front::{Front, Upload};
use crate::joins::group_info_bytes;
use crate::kit::{Change, group, latest, revise};
use crate::played::Played;
use crate::talk::{CAROL_HOST, Talk};
use crate::world::{Person, now};

/// An epoch of a link and its `prev`.
pub type Pair = (u64, u64);

/// The history items of `kind`, `links`, `wraps` or `entries`, of `at`'s channel the server gives
/// the session `token`, as their bytes.
#[track_caller]
pub fn listed(
    cloud: &Cloud,
    token: &str,
    (space, channel): (SpaceId, ChannelId),
    kind: &str,
) -> Vec<Vec<u8>> {
    let path = format!("/spaces/{space}/channels/{channel}/history/{kind}");
    let (status, body) = cloud.get_as(&path, token);
    assert_eq!(status, 200, "GET {path}: {body}");
    body[kind]
        .as_array()
        .unwrap_or_else(|| panic!("GET {path}: {body}"))
        .iter()
        .map(|item| unb64(item.as_str().expect("an item in base64url")))
        .collect()
}

/// The links of `at`'s channel the server gives the session `token`, each as it decodes.
#[track_caller]
pub fn links(cloud: &Cloud, token: &str, at: (SpaceId, ChannelId)) -> Vec<HistoryLink> {
    (listed(cloud, token, at, "links").iter())
        .map(|bytes| {
            HistoryLink::from_cbor(bytes)
                .unwrap_or_else(|err| panic!("a link that does not decode: {err}"))
        })
        .collect()
}

/// The epoch and `prev` of each of `links`.
pub fn pairs(links: &[HistoryLink]) -> BTreeSet<Pair> {
    links.iter().map(|link| (link.epoch, link.prev)).collect()
}

/// Fails unless the server gives the session `token` exactly the links of `at`'s channel whose
/// epochs and `prev` are `expected`, each once, all of the channel; gives them.
#[track_caller]
pub fn assert_links(
    cloud: &Cloud,
    token: &str,
    at: (SpaceId, ChannelId),
    expected: &[Pair],
) -> Vec<HistoryLink> {
    let held = links(cloud, token, at);
    let wanted: BTreeSet<Pair> = expected.iter().copied().collect();
    assert_eq!(
        pairs(&held),
        wanted,
        "the links of {} the server holds",
        at.1
    );
    assert_eq!(held.len(), wanted.len(), "a link twice: {held:?}");
    assert!(
        held.iter().all(|link| link.channel == at.1),
        "a link of another channel: {held:?}"
    );
    held
}

/// As [`assert_links`], of the talk's channel, for the session of Ada's machine.
#[track_caller]
pub fn assert_talk_links(talk: &Talk, expected: &[Pair]) -> Vec<HistoryLink> {
    assert_links(&talk.world.cloud, &talk.ada.token, talk.at(), expected)
}

/// Fails unless the server's GroupInfo of the talk's channel is of `epoch` with each of `devices`
/// once, and the server holds the links of `expected` alone, as [`assert_links`] says; gives them.
#[track_caller]
pub fn assert_chain(
    talk: &Talk,
    (epoch, devices): (u64, &[DeviceId]),
    expected: &[Pair],
) -> Vec<HistoryLink> {
    let (space, channel) = talk.at();
    group(&talk.world.cloud, &talk.ada.token, space, channel).assert_is(epoch, devices);
    assert_talk_links(talk, expected)
}

/// Carol, GitHub user `id` with `login`, signed up; her device played.
pub fn carol(talk: &Talk, id: u64, login: &str) -> Played {
    let cloud = &talk.world.cloud;
    Played::new(cloud, talk.world.sign_up(id, login, CAROL_HOST))
}

/// Bob's device, the talk's first played one, commits an empty Commit in the talk's channel, as
/// [`commits_empty`] says; fails unless it starts `epoch`.
#[track_caller]
pub fn bob_commits_empty(talk: &mut Talk, epoch: u64) {
    let at = talk.at();
    let started = commits_empty(&talk.world.cloud, &mut talk.played[0], at);
    assert_eq!(started, epoch, "the epoch of Bob's empty Commit");
}

/// Bob's device, the talk's first played one, takes in its mailbox; fails unless its key of the
/// epoch of `pair` opens the link of `pair` among `links` to its key of the link's `prev`.
#[track_caller]
pub fn bob_opens(talk: &mut Talk, links: &[HistoryLink], pair: Pair) {
    talk.played[0].take_in(&talk.world.cloud);
    let bob = &talk.played[0];
    assert_opens(links, talk.channel, pair, (bob, bob));
}

/// The history key of `channel` in `epoch` that `played` derived; fails unless it holds one.
#[track_caller]
pub fn key_of(played: &Played, channel: ChannelId, epoch: u64) -> &HistoryKey {
    played
        .mls
        .history_key(channel, epoch)
        .unwrap_or_else(|| panic!("{} holds no key of epoch {epoch}", played.device()))
}

/// Whether two history keys are the same.
pub fn same(one: &HistoryKey, other: &HistoryKey) -> bool {
    *one.to_bytes() == *other.to_bytes()
}

/// Fails unless the link of `(epoch, prev)` among `links` opens with `opener`'s key of `epoch` and
/// gives `holder`'s key of `prev`.
#[track_caller]
pub fn assert_opens(
    links: &[HistoryLink],
    channel: ChannelId,
    (epoch, prev): Pair,
    (opener, holder): (&Played, &Played),
) {
    let link = links
        .iter()
        .find(|link| (link.epoch, link.prev) == (epoch, prev))
        .unwrap_or_else(|| panic!("no link of ({epoch}, {prev}): {links:?}"));
    let opened = key_of(opener, channel, epoch)
        .open_link(link)
        .unwrap_or_else(|err| panic!("the link of ({epoch}, {prev}) does not open: {err:?}"));
    assert!(
        same(&opened, key_of(holder, channel, prev)),
        "the link of ({epoch}, {prev}) does not give the key of epoch {prev}"
    );
}

/// Fails unless `links`, followed from `played`'s key of `epoch`, reach each epoch of `back` with
/// `played`'s key of it.
#[track_caller]
pub fn assert_reach(
    links: &[HistoryLink],
    channel: ChannelId,
    (played, epoch): (&Played, u64),
    back: &[u64],
) {
    let reached = follow_links(channel, epoch, key_of(played, channel, epoch), links);
    let epochs: BTreeSet<u64> = reached.keys().copied().collect();
    assert_eq!(
        epochs,
        back.iter().copied().collect(),
        "the epochs the links reach from epoch {epoch}"
    );
    for (of, key) in &reached {
        assert!(
            same(key, key_of(played, channel, *of)),
            "the links give a wrong key of epoch {of}"
        );
    }
}

/// A link of `channel` Ada's machine sent in a `history` frame: its epoch and `prev`, and whether
/// the front forwarded it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Sent {
    pub pair: Pair,
    pub forwarded: bool,
}

/// The `history` frames of `channel` the front saw the machines send after its first `from`
/// uploads, in order.
pub fn uploads_in(front: &Front, channel: ChannelId, from: usize) -> Vec<Upload> {
    let mut uploads = front.uploads().split_off(from);
    uploads.retain(|upload| upload.channel == channel);
    uploads
}

/// The links of `channel` the front saw the machines send after its first `from` uploads, in
/// order.
pub fn links_sent(front: &Front, channel: ChannelId, from: usize) -> Vec<Sent> {
    (uploads_in(front, channel, from).into_iter())
        .filter_map(|upload| match &upload.item {
            HistoryItem::Link(bytes) => {
                let link = HistoryLink::from_cbor(bytes)
                    .unwrap_or_else(|err| panic!("a link sent that does not decode: {err}"));
                Some(Sent {
                    pair: (link.epoch, link.prev),
                    forwarded: upload.forwarded,
                })
            }
            _ => None,
        })
        .collect()
}

/// How many links of `pair` in `channel` the front forwarded to the server after its first `from`
/// uploads.
pub fn forwarded(front: &Front, channel: ChannelId, from: usize, pair: Pair) -> usize {
    links_sent(front, channel, from)
        .iter()
        .filter(|sent| sent.forwarded && sent.pair == pair)
        .count()
}

/// Fails if the front saw a link of `channel` of `epoch`, or with `epoch` as `prev`, sent after
/// its first `from` uploads.
#[track_caller]
pub fn assert_none_sent_of(front: &Front, channel: ChannelId, from: usize, epoch: u64) {
    let sent = links_sent(front, channel, from);
    assert!(
        sent.iter()
            .all(|one| one.pair.0 != epoch && one.pair.1 != epoch),
        "a link of epoch {epoch} sent: {sent:?}"
    );
}

/// `played` takes in its mailbox, then commits an empty Commit in `at`'s channel; once the server
/// stores it, the device merges it and uploads the new epoch's GroupInfo. Gives the new epoch.
#[track_caller]
pub fn commits_empty(cloud: &Cloud, played: &mut Played, at: (SpaceId, ChannelId)) -> u64 {
    played.take_in(cloud);
    let commit = (played.mls)
        .commit_empty(at.1, &[], now())
        .expect("an empty Commit");
    played.commits(cloud, at, &commit);
    played.uploads_group_info(cloud, at);
    played.mls.epoch(at.1).expect("the new epoch")
}

/// `played`, admitted to `at`'s channel and outside its group, joins it by itself with an external
/// Commit from the GroupInfo the server holds, and uploads the new epoch's GroupInfo; fails unless
/// the server stores the Commit. Gives the new epoch.
#[track_caller]
pub fn joins_by_itself(cloud: &Cloud, played: &mut Played, at: (SpaceId, ChannelId)) -> u64 {
    let group_info = group_info_bytes(cloud, &played.person.token, at);
    let commit = (played.mls)
        .join_by_itself(at.1, &group_info, now(), |_| true)
        .expect("an external Commit");
    played.stores(cloud, at, &commit, &[]);
    played.uploads_group_info(cloud, at);
    played.mls.epoch(at.1).expect("the join's epoch")
}

/// `played` seals the link of `at`'s channel of `(epoch, prev)` with its keys and uploads it on its
/// connection; fails unless the server stores it.
#[track_caller]
pub fn uploads_link(cloud: &Cloud, played: &Played, at: (SpaceId, ChannelId), (epoch, prev): Pair) {
    let link = key_of(played, at.1, epoch)
        .seal_link(at.1, epoch, prev, key_of(played, at.1, prev))
        .expect("a link");
    let answer = played
        .connect(cloud)
        .upload(1, at, HistoryItem::Link(link.to_cbor()));
    assert_eq!(
        answer,
        Frame::HistoryStored { upload: 1 },
        "the link not stored"
    );
}

/// The revision after the latest of `at`'s space, signed by `signer`'s device and stored as a
/// fixture, that adds `person` to the space, its default channel and `at`'s channel.
pub fn adds_to_channel(cloud: &Cloud, signer: &Person, at: (SpaceId, ChannelId), person: &Person) {
    let previous = latest(cloud, &signer.token, at.0);
    let change = Change {
        members: vec![(person, Role::Member)],
        join: vec![(at.1, person)],
        ..Change::default()
    };
    revise(cloud, &previous, signer, &change);
}

/// The revision after the latest of `at`'s space, signed by `signer`'s device and stored as a
/// fixture, that adds `person`, a member of the space, to `at`'s channel alone.
pub fn joins_channel(cloud: &Cloud, signer: &Person, at: (SpaceId, ChannelId), person: &Person) {
    let previous = latest(cloud, &signer.token, at.0);
    let change = Change {
        join: vec![(at.1, person)],
        ..Change::default()
    };
    revise(cloud, &previous, signer, &change);
}

/// The revision after the latest of `at`'s space, signed by `signer`'s device and stored as a
/// fixture, that adds `person` to the space and its default channel alone.
pub fn joins_space(cloud: &Cloud, signer: &Person, space: SpaceId, person: &Person) {
    let previous = latest(cloud, &signer.token, space);
    let change = Change {
        members: vec![(person, Role::Member)],
        ..Change::default()
    };
    revise(cloud, &previous, signer, &change);
}

/// The revision after the latest of `at`'s space, signed by `signer`'s device and stored as a
/// fixture, that removes `person` from `at`'s channel.
pub fn removes_from_channel(
    cloud: &Cloud,
    signer: &Person,
    at: (SpaceId, ChannelId),
    person: &Person,
) {
    let previous = latest(cloud, &signer.token, at.0);
    let change = Change {
        leave: vec![(at.1, person)],
        ..Change::default()
    };
    revise(cloud, &previous, signer, &change);
}

/// The server's database deletes the links of `channel` of `epochs`, as retention does.
pub fn delete_links(cloud: &Cloud, channel: ChannelId, epochs: &[u64]) {
    let epochs: Vec<String> = epochs.iter().map(u64::to_string).collect();
    cloud.execute(&format!(
        "DELETE FROM history_links WHERE channel_id = '{channel}'::uuid AND epoch IN ({})",
        epochs.join(", ")
    ));
}
