//! The key chain: `docs/protocol/membership.md`, "Key chain". The link of epoch e holds the key of
//! `prev`, an earlier epoch the caller gives, sealed with AES-128-GCM under e's key, a random nonce
//! and the associated data `["skald history link", channel, e, prev]`; it opens with e's key and
//! gives `prev`'s. A reader follows every link it can open back from a key. Keys come from the
//! devices' sealed states; a removed device holds the key of no epoch after its removal.

use skald_crypto::{HistoryKey, SealedState, follow_links};
use skald_proto::{Cbor, ChannelId, HistoryLink, MessageId};

use crate::commits::{empty, process, processes_own_remove, remove, store};
use crate::fields::unknown;
use crate::fixtures::{load, signing_key};
use crate::history::{Sealed, body, forged_link, key, link_aad, link_cbor, next_epoch, open};
use crate::steps::{channel_of, creator, history_key};

/// Device `seed`'s link of `channel` from `epoch` to `prev`, both keys from its sealed state.
fn link_of(
    seed: u8,
    state: &SealedState,
    channel: ChannelId,
    epoch: u64,
    prev: u64,
) -> HistoryLink {
    key(seed, state, channel, epoch)
        .seal_link(channel, epoch, prev, &key(seed, state, channel, prev))
        .expect("a link")
}

/// The epochs `links` reach back from `key` of `epoch`, each with its key's bytes.
fn reached(
    channel: ChannelId,
    epoch: u64,
    key: &HistoryKey,
    links: &[HistoryLink],
) -> Vec<(u64, [u8; 16])> {
    follow_links(channel, epoch, key, links)
        .iter()
        .map(|(&epoch, key)| (epoch, *key.to_bytes()))
        .collect()
}

#[cfg_attr(not(target_arch = "wasm32"), test)]
#[cfg_attr(target_arch = "wasm32", wasm_bindgen_test::wasm_bindgen_test)]
fn a_link_opens_with_its_epochs_key_and_gives_the_key_of_prev() {
    let channel = ChannelId::generate();
    let seeds = [90, 91, 92];
    let mut states = channel_of(channel, seeds[0], &seeds[1..]);
    next_epoch(&seeds, &mut states, channel);
    next_epoch(&seeds, &mut states, channel);

    let link = link_of(90, &states[0], channel, 3, 2);
    assert_eq!(
        (link.channel, link.epoch, link.prev),
        (channel, 3, 2),
        "the link's fields"
    );
    for (at, &seed) in seeds.iter().enumerate() {
        let prev = key(seed, &states[at], channel, 3)
            .open_link(&link)
            .unwrap_or_else(|error| panic!("device {seed} opens the link: {error:?}"));
        assert_eq!(
            *prev.to_bytes(),
            history_key(seed, &states[at], channel, 2),
            "device {seed} gets the key of epoch 2"
        );
    }

    // AES-128-GCM under epoch 3's key with the spec's associated data, apart from the crate.
    assert_eq!(
        open(
            &history_key(91, &states[1], channel, 3),
            &link.nonce,
            &link_aad(channel, 3, 2),
            &link.ciphertext,
        ),
        Some(history_key(91, &states[1], channel, 2).to_vec()),
        "the link is the key of epoch 2 sealed as \"Key chain\" says"
    );

    let again = link_of(90, &states[0], channel, 3, 2);
    assert_ne!(again.nonce, link.nonce, "a fresh random nonce each time");
    assert_ne!(again.ciphertext, link.ciphertext);

    let encoded = link.to_cbor();
    let decoded = HistoryLink::from_cbor(&encoded).expect("the link decodes");
    assert_eq!(decoded, link, "the link round-trips");
    assert_eq!(decoded.to_cbor(), encoded, "byte for byte");

    // A link as the server keeps it, with fields of a later version, opens alike.
    let later = HistoryLink::from_cbor(&link_cbor(
        Sealed {
            channel,
            epoch: 3,
            nonce: &link.nonce,
            ciphertext: &link.ciphertext,
        },
        2,
        unknown(),
    ))
    .expect("a link with unknown fields decodes");
    assert_eq!(
        *key(92, &states[2], channel, 3)
            .open_link(&later)
            .expect("the link with unknown fields opens")
            .to_bytes(),
        history_key(92, &states[2], channel, 2)
    );
}

#[cfg_attr(not(target_arch = "wasm32"), test)]
#[cfg_attr(target_arch = "wasm32", wasm_bindgen_test::wasm_bindgen_test)]
fn after_reloads_and_a_catch_up_a_member_links_a_later_epoch_whose_prev_skips_one() {
    let channel = ChannelId::generate();
    let mut states = channel_of(channel, 93, &[94]);
    // Device 94 is offline while device 93 commits three times: epochs 2, 3 and 4.
    let commits: Vec<Vec<u8>> = (0..3)
        .map(|_| {
            let commit = empty(93, &mut states[0], channel, b"");
            store(93, &mut states[0], channel);
            commit
        })
        .collect();
    for commit in &commits {
        process(94, &mut states[1], channel, commit);
    }

    // Device 94 judged epoch 3 not clean, so the link of epoch 4 holds the key of epoch 2.
    let link = link_of(94, &states[1], channel, 4, 2);
    assert_eq!((link.epoch, link.prev), (4, 2));
    assert_eq!(
        *key(93, &states[0], channel, 4)
            .open_link(&link)
            .expect("device 93 opens device 94's link")
            .to_bytes(),
        history_key(93, &states[0], channel, 2),
        "the key of epoch 2"
    );
}

#[cfg_attr(not(target_arch = "wasm32"), test)]
#[cfg_attr(target_arch = "wasm32", wasm_bindgen_test::wasm_bindgen_test)]
fn links_over_several_epochs_with_a_prev_that_skips_one_reach_the_earliest_key() {
    let channel = ChannelId::generate();
    let other = ChannelId::generate();
    let seeds = [95, 96];
    let mut states = channel_of(channel, seeds[0], &seeds[1..]);
    for _ in 0..4 {
        next_epoch(&seeds, &mut states, channel);
    }
    let keys: Vec<[u8; 16]> = (0..=5)
        .map(|epoch| history_key(95, &states[0], channel, epoch))
        .collect();

    let mut changed = link_of(95, &states[0], channel, 5, 3);
    changed.ciphertext[0] ^= 1;
    // Links in no order: 5 to 4, 4 to 2 (skipping 3), 2 to 1, and links to epoch 3 that no key
    // reached opens: of epoch 3, under another key, of another channel under epoch 5's key, and
    // changed.
    let links = vec![
        link_of(95, &states[0], channel, 2, 1),
        link_of(95, &states[0], channel, 3, 1),
        forged_link(&[0x13; 16], channel, 5, 3, &keys[3]),
        forged_link(&keys[5], other, 5, 3, &keys[3]),
        changed,
        link_of(95, &states[0], channel, 5, 4),
        link_of(95, &states[0], channel, 4, 2),
    ];
    assert_eq!(
        reached(channel, 5, &key(96, &states[1], channel, 5), &links),
        vec![(1, keys[1]), (2, keys[2]), (4, keys[4])],
        "from epoch 5's key: epochs 4, 2 and 1, not 3"
    );
    assert_eq!(
        reached(channel, 2, &key(96, &states[1], channel, 2), &links),
        vec![(1, keys[1])],
        "from epoch 2's key: epoch 1"
    );
    assert_eq!(
        reached(channel, 5, &key(96, &states[1], channel, 5), &[]),
        vec![],
        "no links, no keys"
    );
}

#[cfg_attr(not(target_arch = "wasm32"), test)]
#[cfg_attr(target_arch = "wasm32", wasm_bindgen_test::wasm_bindgen_test)]
fn a_removed_device_from_a_full_copy_of_its_state_opens_no_link_or_entry_of_a_later_epoch() {
    let channel = ChannelId::generate();
    let seeds = [90, 91];
    let mut states = channel_of(channel, 90, &[91, 92]);
    // A full copy of device 92's sealed state, taken before the Remove; its keys are the tests'.
    let copy = states[2].clone();
    let commit = remove(90, &mut states[0], channel, &[92], b"");
    store(90, &mut states[0], channel);
    process(91, &mut states[1], channel, &commit);
    let mut processed = copy.clone();
    processes_own_remove(92, &mut processed, channel, &commit);
    next_epoch(&seeds, &mut states[..2], channel);

    // Epochs 2 and 3 come after the Remove.
    let links = vec![
        link_of(90, &states[0], channel, 2, 1),
        link_of(90, &states[0], channel, 3, 2),
    ];
    let entries: Vec<_> = [2, 3]
        .into_iter()
        .map(|epoch| {
            key(90, &states[0], channel, epoch)
                .seal_entry(
                    epoch,
                    &body(90, channel, MessageId::generate()),
                    &signing_key(90),
                )
                .expect("an entry")
        })
        .collect();
    let member = key(91, &states[1], channel, 3);
    assert!(member.open_link(&links[1]).is_ok(), "a member opens them");
    assert!(
        member.open_entry(&entries[1]).is_ok(),
        "a member opens them"
    );

    for (what, state) in [
        ("the copy", &copy),
        ("the copy after it processed the Remove", &processed),
    ] {
        let device = load(92, state);
        assert!(
            device.history_key(channel, 1).is_some(),
            "{what} keeps epoch 1's key"
        );
        for epoch in 0..=3 {
            let Some(held) = device.history_key(channel, epoch) else {
                continue;
            };
            assert!(
                epoch < 2,
                "{what} holds no key of epoch {epoch}, after the Remove"
            );
            for link in &links {
                assert!(
                    held.open_link(link).is_err(),
                    "{what} opens no link of epoch {} with epoch {epoch}'s key",
                    link.epoch
                );
            }
            for entry in &entries {
                assert!(
                    held.open_entry(entry).is_err(),
                    "{what} opens no entry of epoch {} with epoch {epoch}'s key",
                    entry.epoch
                );
            }
            assert!(
                follow_links(channel, epoch, held, &links)
                    .keys()
                    .all(|&reached| reached < 2),
                "{what} reaches no epoch after the Remove"
            );
        }
    }
}

#[cfg_attr(not(target_arch = "wasm32"), test)]
#[cfg_attr(target_arch = "wasm32", wasm_bindgen_test::wasm_bindgen_test)]
fn a_link_under_another_key_or_with_another_channel_epoch_or_prev_is_refused() {
    let channel = ChannelId::generate();
    let other = ChannelId::generate();
    let seeds = [93, 94];
    let mut states = channel_of(channel, seeds[0], &seeds[1..]);
    for _ in 0..3 {
        next_epoch(&seeds, &mut states, channel);
    }
    let other_state = creator(95, other);
    let link = link_of(93, &states[0], channel, 3, 2);
    let reader = key(94, &states[1], channel, 3);
    assert!(reader.open_link(&link).is_ok(), "the link as made opens");

    for (what, key) in [
        ("epoch 2's key", key(94, &states[1], channel, 2)),
        ("epoch 4's key", key(94, &states[1], channel, 4)),
        ("another channel's key", key(95, &other_state, other, 0)),
        ("a key of no epoch", HistoryKey::from_bytes(&[0x13; 16])),
    ] {
        assert!(key.open_link(&link).is_err(), "refused with {what}");
    }
    let forged = forged_link(
        &[0x13; 16],
        channel,
        3,
        2,
        &history_key(94, &states[1], channel, 2),
    );
    assert!(
        reader.open_link(&forged).is_err(),
        "refused: a link under another key"
    );

    let mut with_channel = link.clone();
    with_channel.channel = other;
    let mut with_epoch = link.clone();
    with_epoch.epoch = 4;
    let mut with_prev = link.clone();
    with_prev.prev = 1;
    for (what, changed, key) in [
        ("another channel", &with_channel, &reader),
        (
            "another epoch",
            &with_epoch,
            &key(94, &states[1], channel, 4),
        ),
        (
            "another epoch, read with its epoch's key",
            &with_epoch,
            &reader,
        ),
        ("another prev", &with_prev, &reader),
    ] {
        assert!(
            key.open_link(changed).is_err(),
            "refused: a link with {what}"
        );
    }
}

#[cfg_attr(not(target_arch = "wasm32"), test)]
#[cfg_attr(target_arch = "wasm32", wasm_bindgen_test::wasm_bindgen_test)]
fn a_link_with_a_changed_nonce_or_ciphertext_is_refused_without_a_panic() {
    let channel = ChannelId::generate();
    let seeds = [96, 97];
    let mut states = channel_of(channel, seeds[0], &seeds[1..]);
    next_epoch(&seeds, &mut states, channel);
    let link = link_of(96, &states[0], channel, 2, 1);
    let reader = key(97, &states[1], channel, 2);

    let mut changes: Vec<(&str, HistoryLink)> = Vec::new();
    let mut nonce = link.clone();
    nonce.nonce[11] ^= 1;
    changes.push(("a changed nonce", nonce));
    for at in [0, link.ciphertext.len() - 1] {
        let mut ciphertext = link.clone();
        ciphertext.ciphertext[at] ^= 0x80;
        changes.push(("a changed ciphertext", ciphertext));
    }
    for length in [0, 1, 15, 16, link.ciphertext.len() - 1] {
        let mut truncated = link.clone();
        truncated.ciphertext.truncate(length);
        changes.push(("a truncated ciphertext", truncated));
    }
    let mut longer = link.clone();
    longer.ciphertext.push(0);
    changes.push(("a longer ciphertext", longer));
    for (what, changed) in &changes {
        assert!(reader.open_link(changed).is_err(), "refused: {what}");
    }
}

#[cfg_attr(not(target_arch = "wasm32"), test)]
#[cfg_attr(target_arch = "wasm32", wasm_bindgen_test::wasm_bindgen_test)]
fn a_link_with_an_epoch_below_two_a_prev_of_zero_or_a_prev_not_below_its_epoch_is_neither_made_nor_opened()
 {
    let channel = ChannelId::generate();
    let seeds = [98, 99];
    let mut states = channel_of(channel, seeds[0], &seeds[1..]);
    for _ in 0..2 {
        next_epoch(&seeds, &mut states, channel);
    }
    let keys: Vec<[u8; 16]> = (0..=3)
        .map(|epoch| history_key(98, &states[0], channel, epoch))
        .collect();

    for (epoch, prev, what) in [
        (1, 0, "epoch 1"),
        (1, 1, "epoch 1"),
        (0, 0, "epoch 0"),
        (2, 0, "prev 0"),
        (3, 0, "prev 0"),
        (2, 2, "prev equal to e"),
        (2, 3, "prev above e"),
        (3, 3, "prev equal to e"),
    ] {
        let sealer = HistoryKey::from_bytes(&keys[epoch as usize]);
        let held = HistoryKey::from_bytes(&keys[prev as usize]);
        assert!(
            sealer.seal_link(channel, epoch, prev, &held).is_err(),
            "no link of {what}: e {epoch}, prev {prev}"
        );
        // Sealed apart from the crate with the associated data of its own fields, it opens with
        // the key it is under, and is refused still.
        let forged = forged_link(
            &keys[epoch as usize],
            channel,
            epoch,
            prev,
            &keys[prev as usize],
        );
        assert!(
            HistoryKey::from_bytes(&keys[epoch as usize])
                .open_link(&forged)
                .is_err(),
            "refused: a link of {what}: e {epoch}, prev {prev}"
        );
        assert!(
            follow_links(channel, epoch, &sealer, &[forged]).is_empty(),
            "followed: a link of {what}: e {epoch}, prev {prev}"
        );
    }
}

#[cfg_attr(not(target_arch = "wasm32"), test)]
#[cfg_attr(target_arch = "wasm32", wasm_bindgen_test::wasm_bindgen_test)]
fn a_link_that_holds_no_sixteen_byte_key_is_refused_without_a_panic() {
    let channel = ChannelId::generate();
    let key = [0x4b; 16];
    for held in [&[1; 1][..], &[1; 15], &[1; 17], &[1; 32]] {
        let forged = forged_link(&key, channel, 3, 2, held);
        assert!(
            HistoryKey::from_bytes(&key).open_link(&forged).is_err(),
            "refused: a link that holds {} bytes",
            held.len()
        );
    }
}
