//! What the tests share: the space `acme` of the fixture with a second channel, `ops`; frank, a
//! second account outside the space beside eve; invites and acceptances the tests sign with the
//! invite code of `skald-crypto`; and the requests that issue, store and read them.

use std::collections::BTreeMap;
use std::thread;
use std::time::Duration;

use serde_json::{Value, json};
use skald_crypto::{InviteSecret, SigningKey, sign_acceptance, sign_invite};
use skald_proto::{
    AcceptanceBody, Cbor, ChannelId, INVITE_LABEL, Invite, InviteBody, InviteId, Role, SpaceId,
    Unknown,
};

use crate::api::{Person, Session, Team, assert_refused};
use crate::common::Response;
use crate::common::device_flow::{json, text};
use crate::common::devices::{Device, assert_taken, b64, devices_of, unb64};
use crate::fixture::Fixture;
use crate::kit::revocations_of;
use crate::rosters::{channel, with_later_field};
use crate::wire::now;

/// A day, in seconds.
pub const DAY: u64 = 86_400;

/// The fixture's `acme`, with `ops` after `general` in its latest revision, and frank.
pub struct Scene {
    pub f: Fixture,
    pub frank: Person,
    /// The second channel of `acme`: alice and bob.
    pub ops: ChannelId,
}

impl Scene {
    /// The scene; GitHub user IDs from `github` on, six of them.
    pub fn new(github: u64) -> Self {
        let mut f = Fixture::new(github);
        let frank = Person::new(&f.team.flow, github + 5, "frank");
        let ops = f.team.alice.session.channel(f.team.space);
        let entry = channel(ops, &[&f.team.alice, &f.team.bob]);
        f.revise(|body| body.channels.push(entry));
        Scene { f, frank, ops }
    }

    pub fn team(&self) -> &Team {
        &self.f.team
    }

    pub fn space(&self) -> SpaceId {
        self.f.team.space
    }

    /// An invite of `acme` to `general`, reading its history, and `ops`, not reading it, signed by
    /// alice's laptop under an ID issued to alice's session, with `change` applied to its body;
    /// not stored.
    pub fn invite_with(&self, change: impl FnOnce(&mut InviteBody)) -> Made {
        let alice = &self.f.team.alice;
        let channels = vec![(self.f.general, true), (self.ops, false)];
        made(self.space(), alice, &alice.laptop, channels, change)
    }

    /// [`Scene::invite_with`] unchanged, stored from alice's session.
    #[track_caller]
    pub fn invite(&self) -> Made {
        self.invite_until(now() + 7 * DAY)
    }

    /// [`Scene::invite`] expiring at `expires`.
    #[track_caller]
    pub fn invite_until(&self, expires: u64) -> Made {
        let made = self.invite_with(|body| body.expires = expires);
        assert_taken(&self.f.team.alice.session.put_invite(self.space(), &made));
        made
    }

    /// Checks that `person`'s session gets `made` by its ID with `acme`'s revisions, devices and
    /// revocations as alice gets them.
    #[track_caller]
    pub fn gets_invite(&self, person: &Person, made: &Made) {
        let who = &person.handle;
        let response = person.session.invite_response(made.id);
        assert_eq!(response.status, 200, "{who}: {}", text(&response));
        let body = json(&response);
        let invite = body["invite"]
            .as_str()
            .unwrap_or_else(|| panic!("{who}: no invite in {body}"));
        assert_eq!(unb64(invite), made.bytes, "{who}: the invite's bytes");
        let alice = &self.f.team.alice.session;
        let revisions: Vec<Vec<u8>> = body["revisions"]
            .as_array()
            .unwrap_or_else(|| panic!("{who}: no revisions in {body}"))
            .iter()
            .map(|revision| unb64(revision.as_str().expect("a revision's bytes")))
            .collect();
        assert_eq!(revisions, alice.revisions(self.space()), "{who}: revisions");
        assert_eq!(
            devices_of(&response),
            alice.devices(self.space()),
            "{who}: devices"
        );
        assert_eq!(
            revocations_of(&response),
            revocations_of(&alice.devices_response(self.space())),
            "{who}: revocations"
        );
    }

    /// The acceptances alice's session gets as waiting in `acme`.
    #[track_caller]
    pub fn waiting(&self) -> Vec<Waiting> {
        self.f.team.alice.session.waiting(self.space())
    }

    /// The acceptance bytes alice's session gets as waiting in `acme`, in its order.
    #[track_caller]
    pub fn waiting_bytes(&self) -> Vec<Vec<u8>> {
        self.waiting().into_iter().map(|w| w.acceptance).collect()
    }
}

/// An invite a test made: its ID, its secret, its body and its bytes.
pub struct Made {
    pub id: InviteId,
    pub secret: InviteSecret,
    pub body: InviteBody,
    pub bytes: Vec<u8>,
}

/// An invite of `space` to `channels`, its ID issued to `issuer`'s session, signed by `signer`, for
/// a member, expiring in 7 days, with `change` applied to its body; not stored.
pub fn made(
    space: SpaceId,
    issuer: &Person,
    signer: &Device,
    channels: Vec<(ChannelId, bool)>,
    change: impl FnOnce(&mut InviteBody),
) -> Made {
    let secret = InviteSecret::generate();
    let mut body = InviteBody {
        invite: issuer.session.issue(space),
        space,
        key: secret.invite_key().public_key(),
        role: Role::Member,
        channels,
        signer: signer.id,
        expires: now() + 7 * DAY,
        unknown: Unknown::default(),
    };
    change(&mut body);
    let bytes = signed_invite(&body, &signer.signing.key);
    Made {
        id: body.invite,
        secret,
        body,
        bytes,
    }
}

/// `body` signed by `key`, encoded.
pub fn signed_invite(body: &InviteBody, key: &SigningKey) -> Vec<u8> {
    sign_invite(body, key)
        .expect("an invite body the spec allows")
        .to_cbor()
}

/// `body` with a field of a later version, key 11, signed by `key` over the bytes with it, encoded.
pub fn signed_invite_with_later_field(body: &InviteBody, key: &SigningKey) -> Vec<u8> {
    let bytes = with_later_field(&body.to_cbor());
    let signature = key.sign(INVITE_LABEL, &[&bytes]);
    Invite::from_parts(bytes, signature)
        .expect("an invite body with a later field decodes")
        .to_cbor()
}

/// The acceptance of `made` for `person`'s account, master key and handle.
pub fn acceptance_body(made: &Made, person: &Person) -> AcceptanceBody {
    AcceptanceBody {
        invite: made.id,
        account: person.account,
        master: person.master.public(),
        handle: person.handle.clone(),
        unknown: Unknown::default(),
    }
}

/// `body` signed by `key`, encoded.
pub fn signed_acceptance(body: &AcceptanceBody, key: &SigningKey) -> Vec<u8> {
    sign_acceptance(body, key)
        .expect("an acceptance body the spec allows")
        .to_cbor()
}

/// The acceptance of `made` for `person`, signed by its invite key, encoded.
pub fn acceptance(made: &Made, person: &Person) -> Vec<u8> {
    signed_acceptance(&acceptance_body(made, person), &made.secret.invite_key())
}

/// `signed`, a signed object's encoding, with the last byte of its signature changed.
pub fn broken(signed: &[u8]) -> Vec<u8> {
    let mut bytes = signed.to_vec();
    *bytes.last_mut().expect("a signature") ^= 0x01;
    bytes
}

/// Bytes that decode as no CBOR item.
pub const MALFORMED: &[u8] = &[0xff, 0x00, 0x13];

/// Waits until the clock is past `expires`, a `timestamp`.
pub fn wait_past(expires: u64) {
    while now() <= expires {
        thread::sleep(Duration::from_millis(200));
    }
}

/// A waiting acceptance as an admin's session gets it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Waiting {
    pub acceptance: Vec<u8>,
    /// Each device of its account by its ID, with its certificate.
    pub devices: BTreeMap<String, Option<Vec<u8>>>,
    /// Its account's revocations, sorted.
    pub revocations: Vec<Vec<u8>>,
}

impl Waiting {
    fn of(entry: &Value) -> Self {
        let bytes = |value: &Value| unb64(value.as_str().expect("bytes"));
        let devices = entry["devices"]
            .as_array()
            .unwrap_or_else(|| panic!("no devices in {entry}"))
            .iter()
            .map(|device| {
                let id = device["device"].as_str().expect("a device ID").to_owned();
                (id, device["cert"].as_str().map(unb64))
            })
            .collect();
        let mut revocations: Vec<Vec<u8>> = entry["revocations"]
            .as_array()
            .unwrap_or_else(|| panic!("no revocations in {entry}"))
            .iter()
            .map(bytes)
            .collect();
        revocations.sort();
        Waiting {
            acceptance: bytes(&entry["acceptance"]),
            devices,
            revocations,
        }
    }
}

impl Session {
    /// `POST /spaces/<space>/invites`.
    pub fn issue_response(&self, space: SpaceId) -> Response {
        self.call("post", &format!("/spaces/{space}/invites"), None)
    }

    /// A new invite ID for `space`, which the server must give this session.
    #[track_caller]
    pub fn issue(&self, space: SpaceId) -> InviteId {
        let response = self.issue_response(space);
        assert_eq!(response.status, 201, "an invite ID: {}", text(&response));
        let body = json(&response);
        let id = body["invite"]
            .as_str()
            .unwrap_or_else(|| panic!("no invite in {body}"));
        id.parse().expect("an invite ID")
    }

    /// `PUT /spaces/<space>/invites/<id>` of `bytes`.
    pub fn store_invite(&self, space: SpaceId, id: InviteId, bytes: &[u8]) -> Response {
        let body = json!({ "invite": b64(bytes) });
        self.call("put", &format!("/spaces/{space}/invites/{id}"), Some(&body))
    }

    /// `PUT /spaces/<space>/invites/<ID>` of `made`.
    pub fn put_invite(&self, space: SpaceId, made: &Made) -> Response {
        self.store_invite(space, made.id, &made.bytes)
    }

    /// `GET /invites/<id>`.
    pub fn invite_response(&self, id: InviteId) -> Response {
        self.call("get", &format!("/invites/{id}"), None)
    }

    /// `PUT /invites/<id>/acceptance` of `bytes`.
    pub fn accept(&self, id: InviteId, bytes: &[u8]) -> Response {
        let body = json!({ "acceptance": b64(bytes) });
        self.call("put", &format!("/invites/{id}/acceptance"), Some(&body))
    }

    /// `GET /spaces/<space>/acceptances`.
    pub fn waiting_response(&self, space: SpaceId) -> Response {
        self.call("get", &format!("/spaces/{space}/acceptances"), None)
    }

    /// The waiting acceptances of `space` this session gets, in the server's order.
    #[track_caller]
    pub fn waiting(&self, space: SpaceId) -> Vec<Waiting> {
        let response = self.waiting_response(space);
        assert_eq!(response.status, 200, "{}", text(&response));
        let body = json(&response);
        body["acceptances"]
            .as_array()
            .unwrap_or_else(|| panic!("no acceptances in {body}"))
            .iter()
            .map(Waiting::of)
            .collect()
    }
}

/// Checks that `person`'s session gets no invite `id` and no bytes of `made`, if given; `what`
/// names the case.
#[track_caller]
pub fn gets_no_invite(person: &Person, id: InviteId, made: Option<&Made>, what: &str) {
    let response = person.session.invite_response(id);
    assert_refused(&response, &format!("{what}: {}", person.handle));
    if let Some(made) = made {
        assert!(
            !text(&response).contains(&b64(&made.bytes)),
            "{what}: {}",
            text(&response)
        );
    }
}

/// Checks that `session` gets no waiting acceptance of `space`: a refusal or an empty list, and
/// none of `acceptances`' bytes; `what` names the case.
#[track_caller]
pub fn gets_no_waiting(session: &Session, space: SpaceId, acceptances: &[&[u8]], what: &str) {
    let response = session.waiting_response(space);
    if response.status == 200 {
        let body = json(&response);
        assert_eq!(body["acceptances"], json!([]), "{what}: {body}");
    } else {
        assert_refused(&response, what);
    }
    for acceptance in acceptances {
        assert!(
            !text(&response).contains(&b64(acceptance)),
            "{what}: {}",
            text(&response)
        );
    }
}

/// Checks that `response` is taken: `201` when `first`, `200` for the same bytes again; `what`
/// names the case.
#[track_caller]
pub fn assert_stored(response: &Response, first: bool, what: &str) {
    let status = if first { 201 } else { 200 };
    assert_eq!(response.status, status, "{what}: {}", text(response));
}
