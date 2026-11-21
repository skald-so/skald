//! What the tests of presence uploads and of the presence directory share: the path of a channel's
//! presence, a device's proof on a request for it, the states as the CLI names them, and how far a
//! time a value states may be from the tests' clock.

use base64::Engine as _;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use skald_proto::{Cbor as _, ChannelId, REQUEST_LABEL, RequestProof, SpaceId};

use crate::cloud::{Cloud, unb64};
use crate::world::{Person, post_as};

/// The states as the CLI shows and takes them.
pub const AVAILABLE: &str = "available";
pub const BUSY: &str = "busy";

/// How far a time a value states may be from the tests' clock, in seconds: the machine's clock is
/// the tests', and a value states whole seconds.
pub const SLACK: u64 = 3;

/// The path of `at`'s channel's presence.
pub fn presence_path((space, channel): (SpaceId, ChannelId)) -> String {
    format!("/spaces/{space}/channels/{channel}/presence")
}

/// The `Skald-Device-Proof` of a request of `device`'s machine on a new challenge of its session,
/// as `/mailbox/send` takes it; fails unless the server gives a challenge.
#[track_caller]
pub fn proof(cloud: &Cloud, device: &Person) -> String {
    let (status, body) = post_as(cloud, &device.token, "/session/challenge", None);
    assert_eq!(status, 200, "no challenge for {}: {body}", device.device);
    let challenge = (body["challenge"].as_str())
        .map(unb64)
        .unwrap_or_else(|| panic!("a challenge in base64url: {body}"));
    let signature = (device.key).sign(REQUEST_LABEL, &[&challenge, cloud.public.as_bytes()]);
    let proof = RequestProof {
        challenge,
        device: device.device,
        signature,
    };
    URL_SAFE_NO_PAD.encode(proof.to_cbor())
}

/// Fails unless `time`, a time a value states, is within `[from - SLACK, to + SLACK]`.
#[track_caller]
pub fn assert_between(time: u64, (from, to): (u64, u64), what: &str) {
    assert!(
        from.saturating_sub(SLACK) <= time && time <= to + SLACK,
        "{what} at {time}, not between {from} and {to}"
    );
}
