//! Give a fixture a signed invite through the API.

use serde_json::json;
use skald_crypto::{InviteSecret, sign_invite};
use skald_proto::{Cbor as _, ChannelId, Invite, InviteBody, InviteId, Role, SpaceId, Unknown};

use crate::cloud::Cloud;
use crate::kit::b64;
use crate::world::{Person, now, post_as, put_as};

const WEEK: u64 = 7 * 86_400;

/// An invite ID the server issues for `space` to the session `token`.
#[track_caller]
pub fn issue(cloud: &Cloud, token: &str, space: SpaceId) -> InviteId {
    let path = format!("/spaces/{space}/invites");
    let (status, answer) = post_as(cloud, token, &path, None);
    assert_eq!(
        status, 201,
        "the server did not issue an invite ID for {space}"
    );
    answer["invite"]
        .as_str()
        .and_then(|id| id.parse().ok())
        .expect("an issued invite ID")
}

/// Sign and store a member invite expiring in a week, after applying `change` to its body.
#[track_caller]
pub fn sign_and_store(
    cloud: &Cloud,
    signer: &Person,
    (space, invite): (SpaceId, InviteId),
    secret: &InviteSecret,
    channels: Vec<(ChannelId, bool)>,
    change: impl FnOnce(&mut InviteBody),
) -> Invite {
    let mut body = InviteBody {
        invite,
        space,
        key: secret.invite_key().public_key(),
        role: Role::Member,
        channels,
        signer: signer.device,
        expires: now() + WEEK,
        unknown: Unknown::default(),
    };
    change(&mut body);
    let invite = sign_invite(&body, &signer.key).expect("an invite");
    let path = format!("/spaces/{}/invites/{}", body.space, body.invite);
    let request = json!({ "invite": b64(&invite.to_cbor()) });
    let (status, _) = put_as(cloud, &signer.token, &path, &request);
    assert_eq!(status, 201, "the server did not store the invite");
    invite
}
