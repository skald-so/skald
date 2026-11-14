//! A channel seen from outside its group, as a linked browser or the server sees it:
//! `docs/protocol/membership.md`, "MLS profile", and `docs/protocol/README.md`, "Envelope". Each
//! member gives the GroupInfo of its current epoch, with `ratchet_tree` and `external_pub`, so a
//! Commit's sender gives the new epoch's once the server stored it. From a channel's GroupInfo the
//! view gives its epoch, its signer and its members, once the tree checks and the signature
//! verifies with a member leaf's key; it then follows the channel's Commits, each to a new view the
//! caller keeps or drops. A GroupInfo or a Commit the view refuses leaves it as it was. The hostile
//! GroupInfos are the members' own, changed and signed again apart from the crate
//! (tests/mls/group_info.rs), or made by a client on mls-rs of the tests' own.

use skald_crypto::{ChannelView, MemberDevice, SealedState};
use skald_proto::ChannelId;

use crate::commit_checks::{DAY, foreign_key_package};
use crate::commits::{empty, process, processes_own_remove, remove, store};
use crate::fixtures::{NOW, device_id, load, signing_key};
use crate::foreign;
use crate::foreign_member::{commit, with_foreign_member};
use crate::group_info::{self, EXTERNAL_PUB, GroupInfo, RATCHET_TREE, replace_once, resigned};
use crate::steps::{add, bytes, channel_of, join, send, step, stored, with_key_packages};

/// The member device of seed `seed`, as a view lists it.
fn member(seed: u8) -> MemberDevice {
    MemberDevice {
        device: device_id(seed),
        signing: signing_key(seed).public_key(),
    }
}

/// The devices of `seeds`, each with its state in `states`.
fn seeded(seeds: &[u8], states: Vec<SealedState>) -> Vec<(u8, SealedState)> {
    seeds.iter().copied().zip(states).collect()
}

/// The GroupInfo device `seed` gives of its current epoch of `channel`.
fn group_info_of(seed: u8, state: &mut SealedState, channel: ChannelId) -> Vec<u8> {
    step(seed, state, |device| {
        bytes(device.group_info(channel).expect("a member's GroupInfo"))
    })
}

/// The view of `channel` from `group_info`.
fn observe(channel: ChannelId, group_info: &[u8]) -> ChannelView {
    ChannelView::observe(channel, group_info).expect("the view of the channel's GroupInfo")
}

/// `group_info` is of `channel`'s `epoch`, with `ratchet_tree` and `external_pub`.
fn assert_group_info(group_info: &[u8], channel: ChannelId, epoch: u64, what: &str) {
    let fields = group_info::group_info(group_info);
    assert_eq!(
        (fields.group_id.as_slice(), fields.epoch),
        (channel.as_bytes().as_slice(), epoch),
        "{what}: the GroupInfo's channel and epoch"
    );
    assert!(
        fields.has(RATCHET_TREE) && fields.has(EXTERNAL_PUB),
        "{what}: the GroupInfo has ratchet_tree and external_pub"
    );
}

/// The view is of the epoch and members that each of `devices` holds.
fn assert_holds(view: &ChannelView, devices: &[(u8, SealedState)], channel: ChannelId, what: &str) {
    for (seed, state) in devices {
        let device = load(*seed, state);
        assert_eq!(
            view.epoch(),
            device.epoch(channel).expect("the group"),
            "{what}: device {seed}'s epoch"
        );
        assert_eq!(
            view.members(),
            device.members(channel).expect("the group"),
            "{what}: device {seed}'s members"
        );
    }
}

/// Each of `devices` but `sender` processes `commit` and keeps the new epoch.
fn others_process(
    devices: &mut [(u8, SealedState)],
    sender: u8,
    channel: ChannelId,
    commit: &[u8],
) {
    for (seed, state) in devices.iter_mut().filter(|(seed, _)| *seed != sender) {
        process(*seed, state, channel, commit);
    }
}

/// `view` follows `commit`, of device `sender` of `devices`, which the server stored and the other
/// devices processed: the new view is of the next epoch, signed by `sender`, with the epoch and
/// members each device holds, and the same as the view of the GroupInfo `sender` now gives;
/// `view` stays as it was, for the caller to drop or keep. Gives the new view.
fn follows(
    view: &ChannelView,
    commit: &[u8],
    sender: u8,
    devices: &mut [(u8, SealedState)],
    channel: ChannelId,
    what: &str,
) -> ChannelView {
    let epoch = view.epoch();
    let next = view
        .follow(commit)
        .unwrap_or_else(|error| panic!("{what}: the view follows the Commit: {error:?}"));
    assert_eq!(
        view.epoch(),
        epoch,
        "{what}: the view it followed from stays"
    );
    assert_eq!(next.epoch(), epoch + 1, "{what}: the new view's epoch");
    assert_eq!(next.signer(), member(sender), "{what}: the Commit's sender");
    assert_holds(&next, devices, channel, what);
    let (_, state) = (devices.iter_mut())
        .find(|(seed, _)| *seed == sender)
        .expect("the sender is one of the devices");
    let group_info = group_info_of(sender, state, channel);
    assert_group_info(&group_info, channel, epoch + 1, what);
    let observed = observe(channel, &group_info);
    assert_eq!(
        (observed.epoch(), observed.signer(), observed.members()),
        (next.epoch(), next.signer(), next.members()),
        "{what}: the view of the sender's new GroupInfo"
    );
    next
}

/// `view` refuses `commit` and stays as it was.
fn assert_refused(view: &ChannelView, commit: &[u8], what: &str) {
    let (epoch, signer, members) = (view.epoch(), view.signer(), view.members());
    assert!(view.follow(commit).is_err(), "refused: {what}");
    assert_eq!(
        (view.epoch(), view.signer(), view.members()),
        (epoch, signer, members),
        "{what}: the view as it was"
    );
}

#[cfg_attr(not(target_arch = "wasm32"), test)]
#[cfg_attr(target_arch = "wasm32", wasm_bindgen_test::wasm_bindgen_test)]
fn each_members_group_info_gives_the_epoch_and_members_the_members_hold_and_its_signer() {
    let channel = ChannelId::generate();
    let seeds = [61, 62, 63];
    let mut devices = seeded(&seeds, channel_of(channel, 61, &[62, 63]));
    for at in 0..devices.len() {
        let (seed, state) = &mut devices[at];
        let seed = *seed;
        let group_info = group_info_of(seed, state, channel);
        let what = format!("device {seed}'s GroupInfo");
        assert_group_info(&group_info, channel, 1, &what);
        let leaf = group_info::group_info(&group_info).signer;
        assert_eq!(
            usize::try_from(leaf).unwrap(),
            at,
            "{what}: the signer's leaf"
        );
        let view = observe(channel, &group_info);
        assert_eq!(view.epoch(), 1, "{what}: the epoch");
        assert_eq!(view.signer(), member(seed), "{what}: the signer");
        assert_eq!(view.members(), seeds.map(member).to_vec(), "{what}");
        assert_holds(&view, &devices, channel, &what);
    }
}

#[cfg_attr(not(target_arch = "wasm32"), test)]
#[cfg_attr(target_arch = "wasm32", wasm_bindgen_test::wasm_bindgen_test)]
fn the_view_follows_adds_removes_and_empty_commits_to_the_epochs_and_members_the_members_hold() {
    let channel = ChannelId::generate();
    let mut devices = seeded(&[64, 65, 66], channel_of(channel, 64, &[65, 66]));
    let view = observe(channel, &group_info_of(64, &mut devices[0].1, channel));

    // Device 65 adds device 67.
    let (mut joiner, packages) = with_key_packages(67, 1);
    let commit = add(65, &mut devices[1].1, channel, &[&packages[0]]);
    let welcome = stored(65, &mut devices[1].1, channel);
    others_process(&mut devices, 65, channel, &commit);
    join(67, &mut joiner, &welcome);
    devices.push((67, joiner));
    let view = follows(&view, &commit, 65, &mut devices, channel, "an Add");
    assert_eq!(view.members(), [64, 65, 66, 67].map(member).to_vec());

    // Device 66 removes device 64.
    let commit = remove(66, &mut devices[2].1, channel, &[64], b"");
    store(66, &mut devices[2].1, channel);
    let (_, mut removed) = devices.remove(0);
    processes_own_remove(64, &mut removed, channel, &commit);
    others_process(&mut devices, 66, channel, &commit);
    let view = follows(&view, &commit, 66, &mut devices, channel, "a Remove");
    assert_eq!(view.members(), [65, 66, 67].map(member).to_vec());

    // Device 67 commits an empty Commit.
    let commit = empty(67, &mut devices[2].1, channel, b"");
    store(67, &mut devices[2].1, channel);
    others_process(&mut devices, 67, channel, &commit);
    let view = follows(&view, &commit, 67, &mut devices, channel, "an empty Commit");
    assert_eq!(view.epoch(), 4);
    assert_eq!(view.members(), [65, 66, 67].map(member).to_vec());
}

#[cfg_attr(not(target_arch = "wasm32"), test)]
#[cfg_attr(target_arch = "wasm32", wasm_bindgen_test::wasm_bindgen_test)]
fn a_group_info_whose_tree_or_signature_does_not_check_is_refused() {
    let channel = ChannelId::generate();
    let (signer, removed, other, outsider) = (71, 72, 73, 79);
    let mut states = channel_of(channel, signer, &[removed, other]);
    let first = group_info_of(signer, &mut states[0], channel);
    let commit = remove(signer, &mut states[0], channel, &[removed], b"");
    store(signer, &mut states[0], channel);
    process(other, &mut states[2], channel, &commit);
    let latest = group_info_of(signer, &mut states[0], channel);

    // Signed again unchanged, by its signer, it is the same GroupInfo: the tests' encoding holds.
    assert_eq!(resigned(&latest, signer, |_| {}), latest);
    let view = observe(channel, &latest);
    assert_eq!(view.members(), [signer, other].map(member).to_vec());
    // Any member signs a GroupInfo at its own leaf.
    let view = observe(
        channel,
        &resigned(&latest, other, |fields| fields.signer = 2),
    );
    assert_eq!(view.signer(), member(other));

    // The other member's leaf lists the outsider instead.
    let altered = |fields: &mut GroupInfo| {
        let tree = fields.extension(RATCHET_TREE);
        replace_once(
            tree,
            device_id(other).as_bytes(),
            device_id(outsider).as_bytes(),
        );
        let (from, to) = (signing_key(other), signing_key(outsider));
        replace_once(tree, &from.public_key(), &to.public_key());
    };
    let mut unsigned = group_info::group_info(&latest);
    altered(&mut unsigned);
    // The tree of epoch 1, which still lists the removed device; each leaf's signature verifies.
    let first_tree = group_info::group_info(&first)
        .extension(RATCHET_TREE)
        .clone();
    for (group_info, what) in [
        (unsigned.encode(), "a GroupInfo with an altered tree"),
        (
            resigned(&latest, signer, altered),
            "a GroupInfo with an altered tree, signed again by its signer",
        ),
        (
            resigned(&latest, signer, |fields| {
                *fields.extension(RATCHET_TREE) = first_tree.clone();
            }),
            "a GroupInfo with the tree of an earlier epoch, signed again by its signer",
        ),
        (
            resigned(&latest, signer, |fields| {
                fields.extensions.retain(|(kind, _)| *kind != RATCHET_TREE);
            }),
            "a GroupInfo without its tree, signed again by its signer",
        ),
        (
            resigned(&latest, outsider, |_| {}),
            "a GroupInfo signed by a key outside the group",
        ),
        (
            resigned(&latest, other, |_| {}),
            "a GroupInfo signed by another member's key than its signer leaf's",
        ),
        (
            resigned(&latest, removed, |fields| fields.signer = 1),
            "a GroupInfo signed by the removed device at its blank leaf",
        ),
        (
            resigned(&latest, outsider, |fields| fields.signer = 9),
            "a GroupInfo signed for a leaf that does not exist",
        ),
    ] {
        assert!(
            ChannelView::observe(channel, &group_info).is_err(),
            "refused: {what}"
        );
    }
}

#[cfg_attr(not(target_arch = "wasm32"), test)]
#[cfg_attr(target_arch = "wasm32", wasm_bindgen_test::wasm_bindgen_test)]
fn a_group_info_of_another_channel_is_refused() {
    let (channel, elsewhere) = (ChannelId::generate(), ChannelId::generate());
    let mut states = channel_of(elsewhere, 74, &[75]);
    let group_info = group_info_of(74, &mut states[0], elsewhere);
    assert!(ChannelView::observe(channel, &group_info).is_err());
    assert_eq!(observe(elsewhere, &group_info).epoch(), 1);
}

#[cfg_attr(not(target_arch = "wasm32"), test)]
#[cfg_attr(target_arch = "wasm32", wasm_bindgen_test::wasm_bindgen_test)]
fn a_group_info_with_a_leaf_that_breaks_the_profile_is_refused() {
    let channel = ChannelId::generate();
    let (signer, joiner, third) = (81, 82, 83);
    let (_, packages) = with_key_packages(joiner, 1);
    // A GroupInfo of `channel`'s group on mls-rs, its signer's credential `identity`, with the
    // joiner and a third device with credential `third_identity`.
    let made = |identity: &[u8], third_identity: &[u8]| {
        let package = foreign::key_package(third, third_identity);
        let (group, _) = foreign::group(signer, identity, channel, &[&packages[0], &package]);
        group
            .group_info_message_allowing_ext_commit(true)
            .and_then(|message| message.to_bytes())
            .expect("a GroupInfo of mls-rs")
    };
    let (own, theirs) = (device_id(signer), device_id(third));
    let view = observe(channel, &made(own.as_bytes(), theirs.as_bytes()));
    assert_eq!(view.signer(), member(signer));
    assert_eq!(view.members(), [signer, joiner, third].map(member).to_vec());

    let other_id = device_id(84);
    let long_id = [own.as_bytes().as_slice(), b"+"].concat();
    for (signers, thirds, what) in [
        (
            other_id.as_bytes().as_slice(),
            theirs.as_bytes().as_slice(),
            "the signer's identity is another device's ID",
        ),
        (
            &own.as_bytes()[1..],
            theirs.as_bytes(),
            "the signer's identity is 15 bytes",
        ),
        (
            &long_id,
            theirs.as_bytes(),
            "the signer's identity is 17 bytes",
        ),
        (
            own.as_bytes(),
            other_id.as_bytes(),
            "another member's identity is another device's ID",
        ),
    ] {
        assert!(
            ChannelView::observe(channel, &made(signers, thirds)).is_err(),
            "refused: {what}"
        );
    }
}

#[cfg_attr(not(target_arch = "wasm32"), test)]
#[cfg_attr(target_arch = "wasm32", wasm_bindgen_test::wasm_bindgen_test)]
fn a_commit_of_another_epoch_is_reported_and_leaves_the_view_as_it_was() {
    let channel = ChannelId::generate();
    let mut devices = seeded(&[91, 92, 93], channel_of(channel, 91, &[92, 93]));
    let view = observe(channel, &group_info_of(91, &mut devices[0].1, channel));
    let first = empty(92, &mut devices[1].1, channel, b"");
    store(92, &mut devices[1].1, channel);
    others_process(&mut devices, 92, channel, &first);
    let second = empty(93, &mut devices[2].1, channel, b"");
    store(93, &mut devices[2].1, channel);
    others_process(&mut devices, 93, channel, &second);

    assert_refused(&view, &second, "a Commit of the next epoch");
    let next = view.follow(&first).expect("the Commit of the view's epoch");
    assert_refused(&next, &first, "the Commit the view followed, again");
    let last = next
        .follow(&second)
        .expect("the Commit of the new view's epoch");
    assert_eq!((last.epoch(), last.signer()), (3, member(93)));
    assert_holds(&last, &devices, channel, "after two Commits");
}

#[cfg_attr(not(target_arch = "wasm32"), test)]
#[cfg_attr(target_arch = "wasm32", wasm_bindgen_test::wasm_bindgen_test)]
fn a_commit_from_a_non_member_is_reported_and_leaves_the_view_as_it_was() {
    let channel = ChannelId::generate();
    let mut states = channel_of(channel, 94, &[95, 96]);
    let view = observe(channel, &group_info_of(94, &mut states[0], channel));
    // Devices outside the group form their own with the channel's ID: its Commit of epoch 1 names
    // leaf 0, device 94's.
    let mut fork = channel_of(channel, 97, &[98]);
    let forked = empty(97, &mut fork[0], channel, b"");
    // Device 94's Commit of epoch 1 of another channel, from leaf 0 there too.
    let elsewhere = ChannelId::generate();
    let mut other = channel_of(elsewhere, 94, &[95]);
    let other_commit = empty(94, &mut other[0], elsewhere, b"");

    assert_refused(&view, &forked, "a Commit of devices outside the group");
    assert_refused(&view, &other_commit, "a Commit of another channel's group");
    let commit = empty(95, &mut states[1], channel, b"");
    let next = view.follow(&commit).expect("a member's Commit");
    assert_eq!((next.epoch(), next.signer()), (2, member(95)));
}

#[cfg_attr(not(target_arch = "wasm32"), test)]
#[cfg_attr(target_arch = "wasm32", wasm_bindgen_test::wasm_bindgen_test)]
fn a_malformed_or_truncated_group_info_or_commit_is_refused_without_a_panic() {
    let channel = ChannelId::generate();
    let mut states = channel_of(channel, 85, &[86]);
    let group_info = group_info_of(85, &mut states[0], channel);
    let view = observe(channel, &group_info);
    let message = send(85, &mut states[0], channel, b"neither");
    let commit = empty(86, &mut states[1], channel, b"");
    for (bytes, what) in [
        (Vec::new(), "no bytes"),
        (vec![0xff; 64], "bytes that are no MLS message"),
        (message, "an application message"),
    ] {
        assert!(
            ChannelView::observe(channel, &bytes).is_err(),
            "refused as a GroupInfo: {what}"
        );
        assert_refused(&view, &bytes, what);
    }
    assert!(
        ChannelView::observe(channel, &commit).is_err(),
        "refused as a GroupInfo: a Commit"
    );
    assert_refused(&view, &group_info, "a GroupInfo");
    for len in 0..group_info.len() {
        assert!(
            ChannelView::observe(channel, &group_info[..len]).is_err(),
            "refused: the GroupInfo cut to {len} bytes"
        );
    }
    for len in 0..commit.len() {
        assert_refused(&view, &commit[..len], "a truncated Commit");
    }
    assert_eq!(view.follow(&commit).expect("the whole Commit").epoch(), 2);
}

#[cfg_attr(not(target_arch = "wasm32"), test)]
#[cfg_attr(target_arch = "wasm32", wasm_bindgen_test::wasm_bindgen_test)]
fn a_group_info_or_commit_with_a_leaf_whose_lifetime_spans_more_than_sixty_days_is_refused() {
    let channel = ChannelId::generate();
    let (mut states, mut group) = with_foreign_member(channel, &[44, 45], 46);
    let view = observe(channel, &group_info_of(44, &mut states[0], channel));
    let long = || foreign_key_package(47, NOW, 60 * DAY + 1);
    let added = commit(&mut group, |builder| {
        builder
            .add_member(long())
            .expect("mls-rs adds the KeyPackage")
    });
    assert_refused(
        &view,
        &added,
        "an Add of a KeyPackage spanning 60 days and a second",
    );
    let added = commit(&mut group, |builder| {
        (builder.add_member(foreign_key_package(47, NOW, 60 * DAY)))
            .expect("mls-rs adds the KeyPackage")
    });
    let next = view
        .follow(&added)
        .expect("an Add of a KeyPackage spanning 60 days");
    assert_eq!(next.members(), [44, 45, 46, 47].map(member).to_vec());

    let package = long().to_bytes().expect("the KeyPackage's bytes");
    let (group, _) = foreign::group(48, device_id(48).as_bytes(), channel, &[&package]);
    let group_info = group
        .group_info_message_allowing_ext_commit(true)
        .and_then(|message| message.to_bytes())
        .expect("a GroupInfo of mls-rs");
    assert!(
        ChannelView::observe(channel, &group_info).is_err(),
        "refused: a GroupInfo with a leaf spanning 60 days and a second"
    );
}
