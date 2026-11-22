//! What the tests share: the queue key objects the server holds, as a device reads them, objects
//! the tests sign, and the ones the front saw the machines upload.

use skald_crypto::{SigningKey, check_queue_key, sign_queue_key};
use skald_proto::{Cbor as _, ChannelId, DeviceId, QueueKey, QueueKeyBody, SpaceId, Unknown};

use crate::chain::key_of;
use crate::cloud::{Cloud, unb64};
use crate::front::{Front, QueueKeyUpload};
use crate::played::Played;
use crate::queuing::proved;
use crate::world::{Person, warnings, words};

/// The path of the queue key object of `at`'s channel in `epoch`.
pub fn object_path((space, channel): (SpaceId, ChannelId), epoch: u64) -> String {
    format!("/spaces/{space}/channels/{channel}/queue-keys/{epoch}")
}

/// The request for the queue key object of `at`'s channel in `epoch`, as its line starts.
pub fn object_request(at: (SpaceId, ChannelId), epoch: u64) -> String {
    format!("GET {} ", object_path(at, epoch))
}

/// The queue key object of `at`'s channel in `epoch` the server holds, as it gives it to
/// `person`'s device, with the device's proof; `None` when it answers `404`. Fails on any other
/// answer.
#[track_caller]
pub fn held(
    cloud: &Cloud,
    person: &Person,
    at: (SpaceId, ChannelId),
    epoch: u64,
) -> Option<QueueKey> {
    let path = object_path(at, epoch);
    let (status, body) = proved(cloud, person, ("GET", &path), None);
    match status {
        200 => {
            let bytes = (body["queue_key"].as_str())
                .map(unb64)
                .unwrap_or_else(|| panic!("no queue_key in {body}"));
            Some(QueueKey::from_cbor(&bytes).expect("a queue key object that decodes"))
        }
        404 => None,
        _ => panic!("the object of epoch {epoch}: {status} {body}"),
    }
}

/// As [`held`]; fails unless the server holds one.
#[track_caller]
pub fn held_one(cloud: &Cloud, person: &Person, at: (SpaceId, ChannelId), epoch: u64) -> QueueKey {
    held(cloud, person, at, epoch)
        .unwrap_or_else(|| panic!("the server holds no queue key object of epoch {epoch}"))
}

/// The `pk` of `played`'s history key of `channel` in `epoch`.
#[track_caller]
pub fn pk_of(played: &Played, channel: ChannelId, epoch: u64) -> [u8; 32] {
    key_of(played, channel, epoch).queue_key().public_key()
}

/// The queue key object of `channel`'s `epoch` with `key`, signed by the device `signer` with
/// `signing`, in CBOR.
pub fn object_of(
    (channel, epoch): (ChannelId, u64),
    key: [u8; 32],
    (signer, signing): (DeviceId, &SigningKey),
) -> Vec<u8> {
    let body = QueueKeyBody {
        channel,
        epoch,
        key,
        signer,
        unknown: Unknown::default(),
    };
    (sign_queue_key(&body, signing).expect("a queue key object")).to_cbor()
}

/// A 32-byte key no device derives.
pub fn wrong_key() -> [u8; 32] {
    let mut key = [0; 32];
    getrandom::fill(&mut key).expect("the operating system's random generator");
    key
}

/// Fails unless `object` is of `channel`'s `epoch` with `key`, names `signer` and its signature
/// verifies with `signing`, `signer`'s signing key as its certificate holds it.
#[track_caller]
pub fn assert_object(
    object: &QueueKey,
    (channel, epoch): (ChannelId, u64),
    key: [u8; 32],
    (signer, signing): (DeviceId, &[u8; 32]),
) {
    let body = object.body();
    assert_eq!(body.channel, channel, "the object's channel");
    assert_eq!(body.epoch, epoch, "the object's epoch");
    assert!(
        body.key == key,
        "the object of epoch {epoch} holds another key than its pk"
    );
    assert_eq!(body.signer, signer, "the object's signer");
    assert!(
        check_queue_key(object, signing).is_ok(),
        "the object of epoch {epoch} is not signed by {signer}"
    );
}

/// The queue key objects of `channel`'s `epoch` the front saw uploaded from the `from`th on.
pub fn uploads_of(
    front: &Front,
    (channel, epoch): (ChannelId, u64),
    from: usize,
) -> Vec<QueueKeyUpload> {
    (front.queue_key_uploads().into_iter().skip(from))
        .filter(|upload| upload.channel == channel)
        .filter(|upload| {
            QueueKey::from_cbor(&upload.object).is_ok_and(|object| object.body().epoch == epoch)
        })
        .collect()
}

/// The warnings of `finished` of a queue key: lines holding the words `queue` and `key`.
pub fn queue_key_warnings(finished: &crate::machine::Finished) -> Vec<String> {
    (warnings(finished).into_iter())
        .filter(|line| {
            let found = words(line);
            found.contains(&"queue") && found.contains(&"key")
        })
        .collect()
}
