//! What the tests share: queue key objects to upload, their uploads on a device's connection, and
//! the objects the server gives.

use skald_crypto::{SigningKey, sign_queue_key};
use skald_proto::{Cbor, ChannelId, QueueKeyBody, SpaceId, Unknown};

use crate::common::device_flow::{json, text};
use crate::common::devices::{Device, random, unb64};
use crate::common::{Response, TestDb};
use crate::history::{Answer, Item, attempt, upload_to};
use crate::kit::{Body, Client, World};
use crate::scene::{Peer, removing};
use crate::wire::{Socket, assert_nowhere};

/// The problems of a refused upload.
pub const NOT_A_MEMBER: &str = "not-a-member";
pub const EPOCH_MISMATCH: &str = "epoch-mismatch";
pub const MALFORMED: &str = "malformed";

/// The object of `channel`'s epoch `epoch` whose key is `key`, signed by the device `signer`, which
/// it names as its `signer`.
pub fn object_with(channel: ChannelId, epoch: u64, key: [u8; 32], signer: &Device) -> Item {
    let body = QueueKeyBody {
        channel,
        epoch,
        key,
        signer: signer.id,
        unknown: Unknown::default(),
    };
    let signing = SigningKey::from_bytes(&signer.signing.secret);
    let signed = sign_queue_key(&body, &signing).expect("a queue key object");
    Item::QueueKey(signed.to_cbor())
}

/// As [`object_with`], with a key of its own, as the server sees one: it cannot tell the epoch's.
pub fn object(channel: ChannelId, epoch: u64, signer: &Device) -> Item {
    object_with(channel, epoch, random(), signer)
}

/// The object of `channel`'s epoch `epoch` with the `pk` of `peer`'s history key of it, signed by
/// the device `signer`.
#[track_caller]
pub fn genuine(channel: ChannelId, peer: &Peer, epoch: u64, signer: &Device) -> Item {
    let key = (peer.mls.history_key(channel, epoch))
        .unwrap_or_else(|| panic!("{} holds no key of epoch {epoch}", peer.id))
        .queue_key()
        .public_key();
    object_with(channel, epoch, key, signer)
}

/// `peer` uploads `item` in `at`'s channel; gives the answer.
#[track_caller]
pub fn upload(at: (SpaceId, ChannelId), peer: &mut Peer, item: &Item) -> Answer {
    upload_to(peer, at.0, at.1, item)
}

/// Checks that the server keeps `item` from `peer` in `at`'s channel: `history-stored`.
#[track_caller]
pub fn keeps(at: (SpaceId, ChannelId), peer: &mut Peer, item: &Item, what: &str) {
    assert_eq!(
        upload(at, peer, item),
        Answer::Stored,
        "{what}: history-stored"
    );
}

/// Checks that the server refuses `item` from `peer` in `at`'s channel, with `problem` if any, and
/// that `db` holds nothing of it.
#[track_caller]
pub fn refuses(
    (db, at): (&TestDb, (SpaceId, ChannelId)),
    peer: &mut Peer,
    item: &Item,
    problem: Option<&str>,
    what: &str,
) {
    let answer = upload(at, peer, item);
    match (&answer, problem) {
        (Answer::Refused(given), Some(problem)) => assert_eq!(given, problem, "{what}"),
        (Answer::Refused(_), None) => {}
        (Answer::Stored, _) => panic!("{what}: history-stored"),
    }
    assert_nowhere(db, item.bytes());
}

/// Checks that the server stores nothing of `item` that `socket`, a connection it may close,
/// uploads in `at`'s channel: no `history-stored`, and no row of `db` holds it.
#[track_caller]
pub fn not_kept(
    (db, at): (&TestDb, (SpaceId, ChannelId)),
    socket: &mut Socket,
    item: &Item,
    what: &str,
) {
    let answer = attempt(socket, at.0, at.1, item);
    assert!(
        !matches!(answer, Some(Answer::Stored)),
        "{what}: history-stored"
    );
    assert_nowhere(db, item.bytes());
}

/// The path of `general`'s queue key object of `epoch`.
pub fn path(w: &World, epoch: u64) -> String {
    format!(
        "/spaces/{}/channels/{}/queue-keys/{epoch}",
        w.space, w.channel
    )
}

/// `GET` of `general`'s object of `epoch` by `client`, proved if `proved`.
pub fn read(w: &World, client: &Client, epoch: u64, proved: bool) -> Response {
    client.call("get", &path(w, epoch), Body::None, proved)
}

/// The bytes of `general`'s object of `epoch` the server gives `client`, proved; `None` when it
/// answers that it keeps none. Fails on any other answer.
#[track_caller]
pub fn given(w: &World, client: &Client, epoch: u64) -> Option<Vec<u8>> {
    let response = read(w, client, epoch, true);
    match response.status {
        200 => {
            let body = json(&response);
            let bytes = (body["queue_key"].as_str())
                .unwrap_or_else(|| panic!("{}: no queue_key in {body}", client.who));
            Some(unb64(bytes))
        }
        404 => None,
        _ => panic!("{} reads epoch {epoch}: {}", client.who, text(&response)),
    }
}

/// Checks that the server gives `client` `item` as `general`'s object of `epoch`, byte for byte.
#[track_caller]
pub fn assert_given(w: &World, client: &Client, epoch: u64, item: &Item, what: &str) {
    let found = given(w, client, epoch);
    assert!(
        found.as_deref() == Some(item.bytes()),
        "{what}: {} is not given the object kept for epoch {epoch}",
        client.who
    );
}

/// Checks that the server answers `client` that it keeps no object of `general`'s `epoch`.
#[track_caller]
pub fn assert_none_given(w: &World, client: &Client, epoch: u64, what: &str) {
    assert_eq!(
        given(w, client, epoch),
        None,
        "{what}: an object of epoch {epoch} given to {}",
        client.who
    );
}

/// alice's laptop commits carol's Remove, which bob's laptop processes: epoch 2, of alice and bob.
#[track_caller]
pub fn remove_carol(w: &mut World) {
    let carol = w.carol.id;
    let removal = w.alice.commits(&w.stage.f, removing(&[carol]));
    w.bob.process(&w.stage.f, &removal);
    assert_eq!(w.epoch(), 2, "the epoch of carol's Remove");
}
