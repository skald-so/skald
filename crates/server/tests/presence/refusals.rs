//! Uploads the server refuses, keeping nothing of them: from a device outside the channel's current
//! epoch, naming another device or another epoch, with a `ct` over 65552 bytes, bytes that do not
//! decode, and a session that does not prove its device.

use skald_proto::ChannelId;

use crate::common::cbor::{array_of, byte_item, head, uint};
use crate::common::hex;
use crate::kit::{
    LONGEST_CT, TOO_LARGE, Value, assert_given, assert_kept, assert_not_held, assert_refused_with,
    assert_unproved, callers, epoch, formed, held, random_bytes,
};
use crate::scene::{EPOCH_MISMATCH, empty, removing};
use crate::wire::NOT_A_MEMBER;

/// dave's laptop, a channel member outside the group, and eve's, outside the space, are refused;
/// so is carol's once alice's Commit removes it, with `not-a-member`. bob's value is kept, and the
/// members get it alone.
#[test]
fn an_upload_from_a_device_outside_the_current_epoch_is_refused() {
    let (mut s, general) = formed(3_621_200_001);
    let [alice, bob, carol, dave, eve] = callers(&s);
    let of_bob = general.value(s.bob.id);
    assert_kept(&bob.put(&general, &of_bob), "bob's");

    let of_dave = general.value(s.dave.id);
    let response = dave.put(&general, &of_dave);
    assert_refused_with(&response, Some(NOT_A_MEMBER), "dave outside the group");
    let of_eve = general.value(s.eve.id);
    assert_refused_with(&eve.put(&general, &of_eve), None, "eve outside the space");

    let commit = s.alice.commits(&s.fixture, removing(&[s.carol.id]));
    s.bob.process(&s.fixture, &commit);
    let after = general.at(epoch(&s));
    let of_carol = after.value(s.carol.id);
    let response = carol.put(&after, &of_carol);
    assert_refused_with(&response, Some(NOT_A_MEMBER), "carol removed");
    let of_carol_before = general.value(s.carol.id);
    let response = carol.put(&general, &of_carol_before);
    assert_refused_with(&response, None, "carol removed, in the epoch before");

    for (value, what) in [
        (&of_dave, "dave's"),
        (&of_eve, "eve's"),
        (&of_carol, "carol's"),
        (&of_carol_before, "carol's of the epoch before"),
    ] {
        assert_not_held(s.fixture.db(), value, what);
    }
    assert_given(&[&alice, &bob], &general, &[&of_bob], "bob's alone");
}

/// alice's laptop uploads a value naming bob's laptop, refused with `not-a-member`, and values of
/// the epoch after the current one and, once bob's empty Commit has moved the channel on, of the
/// epoch before, refused with `epoch-mismatch`. None is kept: bob gets alice's value of the
/// current epoch alone.
#[test]
fn an_upload_naming_another_device_or_epoch_is_refused() {
    let (mut s, first) = formed(3_621_200_011);
    let [alice, bob, ..] = callers(&s);

    let as_bob = first.value(s.bob.id);
    let response = alice.put(&first, &as_bob);
    assert_refused_with(&response, Some(NOT_A_MEMBER), "naming bob's laptop");
    let ahead = first.at(first.epoch + 1).value(s.alice.id);
    let response = alice.put(&first, &ahead);
    assert_refused_with(&response, Some(EPOCH_MISMATCH), "the next epoch");

    let commit = s.bob.commits(&s.fixture, empty);
    for peer in [&mut s.alice, &mut s.carol] {
        peer.process(&s.fixture, &commit);
    }
    let current = first.at(epoch(&s));
    assert_eq!(
        current.epoch,
        first.epoch + 1,
        "the epoch after bob's Commit"
    );
    let behind = first.value(s.alice.id);
    let response = alice.put(&current, &behind);
    assert_refused_with(&response, Some(EPOCH_MISMATCH), "the epoch before");
    let of_alice = current.value(s.alice.id);
    assert_kept(
        &alice.put(&current, &of_alice),
        "alice's of the current epoch",
    );

    for (value, what) in [
        (&as_bob, "naming bob"),
        (&ahead, "ahead"),
        (&behind, "behind"),
    ] {
        assert_not_held(s.fixture.db(), value, what);
    }
    assert_given(
        &[&bob],
        &current,
        &[&of_alice],
        "alice's of the current epoch alone",
    );
}

/// A value whose `ct` is one byte over 65552 is refused with `too-large`; one of 65552 bytes is
/// kept.
#[test]
fn a_ct_over_65552_bytes_is_refused_as_too_large() {
    let (s, general) = formed(3_621_200_021);
    let [alice, bob, ..] = callers(&s);
    let over = general.value(s.alice.id).with_ct(LONGEST_CT + 1);
    assert_refused_with(&alice.put(&general, &over), Some(TOO_LARGE), "a ct over");
    assert_not_held(s.fixture.db(), &over, "a ct over");

    let longest = general.value(s.alice.id).with_ct(LONGEST_CT);
    assert_kept(&alice.put(&general, &longest), "the longest ct");
    assert_given(&[&bob], &general, &[&longest], "the longest ct");
}

/// The encoded map of `fields`, each a key and an encoded item, in the given order.
fn map_of(fields: &[(u64, Vec<u8>)]) -> Vec<u8> {
    let mut bytes = head(5, fields.len() as u64);
    for (key, item) in fields {
        bytes.extend(uint(*key));
        bytes.extend_from_slice(item);
    }
    bytes
}

/// alice's laptop uploads bytes that are no `presence` value, each refused: bytes that are no CBOR,
/// a value cut short, an array of its fields, a map without `ct`, one with a nonce of 11 bytes, one
/// with the channel as text, and a value of another channel than the request's. None is kept: bob
/// gets alice's value alone.
#[test]
fn bytes_that_do_not_decode_are_refused() {
    let (s, general) = formed(3_621_200_031);
    let [alice, bob, ..] = callers(&s);
    let good = general.value(s.alice.id);
    let bytes = good.bytes();
    let device = byte_item(s.alice.id.as_bytes());
    let channel = byte_item(general.id.as_bytes());
    let epoch = uint(general.epoch);
    let nonce = byte_item(&good.nonce);
    let ct = byte_item(&good.ct);
    let elsewhere = Value::new(s.alice.id, ChannelId::generate(), general.epoch);
    let mut short = good.clone();
    short.nonce = random_bytes(11);
    let channel_text = [&[0x78, 32][..], hex(general.id.as_bytes()).as_bytes()].concat();

    let fields = [
        device.clone(),
        channel.clone(),
        epoch.clone(),
        nonce.clone(),
        ct.clone(),
    ];
    let no_ct = [
        (1, device.clone()),
        (2, channel),
        (3, epoch.clone()),
        (4, nonce.clone()),
    ];
    let as_text = [
        (1, device),
        (2, channel_text),
        (3, epoch),
        (4, nonce),
        (5, ct),
    ];
    let cases: Vec<(Vec<u8>, &str)> = vec![
        (random_bytes(64), "bytes that are no CBOR"),
        (bytes[..bytes.len() - 7].to_vec(), "a value cut short"),
        (array_of(&fields), "an array"),
        (map_of(&no_ct), "no ct"),
        (short.bytes(), "a nonce of 11 bytes"),
        (map_of(&as_text), "the channel as text"),
        (elsewhere.bytes(), "a value of another channel"),
    ];
    for (body, what) in &cases {
        let response = alice.put_raw(&general, body, true);
        assert_refused_with(&response, None, what);
        assert!(!held(s.fixture.db(), body), "{what}: kept");
    }

    assert_kept(&alice.put(&general, &good), "alice's value");
    assert_given(&[&bob], &general, &[&good], "alice's value alone");
}

/// bob's session uploads his laptop's value without the laptop's proof: `401` with
/// `auth-required`, nothing kept. With the proof, it is kept.
#[test]
fn a_session_that_does_not_prove_its_device_uploads_nothing() {
    let (s, general) = formed(3_621_200_041);
    let [alice, bob, ..] = callers(&s);
    let unproved = general.value(s.bob.id);
    let response = bob.put_raw(&general, &unproved.bytes(), false);
    assert_unproved(&response, "without the proof");
    assert_not_held(s.fixture.db(), &unproved, "without the proof");

    let proved = general.value(s.bob.id);
    assert_kept(&bob.put(&general, &proved), "with the proof");
    assert_given(&[&alice], &general, &[&proved], "with the proof");
}
