//! The issuing device as a test plays it with a linked device's key, from the code the new device
//! shows: it answers the new device's run, opens its request and sends it a grant it makes.

use std::thread;
use std::time::{Duration, Instant};

use serde_json::{Value, json};
use skald_crypto::{LinkCode, LinkEntry, SigningKey, sign_link};
use skald_proto::{
    CERT_LABEL, Cbor as _, CertBody, DeviceId, LinkBody, LinkGrantBody, LinkMessage,
    LinkRequestBody, Unknown,
};

use crate::anchors::{latest, personal_space};
use crate::cloud::{Cloud, WAIT, unb64};
use crate::kit::b64;
use crate::linking::NewDevice;
use crate::world::{Person, now, post_as};

/// The account's latest hello, its path, and the path of a run's later messages.
const HELLO_PATH: &str = "/account/links/hello";
const MESSAGES_PATH: &str = "/account/links/messages";
/// Where the issuing device takes the hello of a run without a session by its pairing.
const PAIRING_PATH: &str = "/account/links/pairing";

/// A run with the new device as the issuing device plays it, up to the grant: the code, and the
/// run's keys and what the new device asked for.
pub struct Run {
    pub session: [u8; 16],
    pub isk: skald_crypto::Isk,
    pub request: LinkRequestBody,
}

/// Plays the issuing device on `first`'s session for the run `new` opened: takes the latest
/// hello, sends a reply keyed by `new`'s code, and opens its request.
pub fn answer(cloud: &Cloud, first: &Person, new: &mut NewDevice) -> Run {
    new.open_run(cloud, first);
    let (session, share) = hello_in(cloud.get_as(HELLO_PATH, &first.token), HELLO_PATH);
    let code = LinkCode::from_entry(&new.code()).expect("a code");
    let (reply, isk) = code
        .respond(first.account, &session, &share)
        .expect("the hello's share");
    replied(cloud, first, session, (reply, isk))
}

/// Plays the issuing device on `first`'s session for the run of a new device without a session
/// that shows `shown`, a code of 12 characters: pairs the run by its pairing, sends a reply keyed by
/// the code, and opens its request.
pub fn answer_paired(cloud: &Cloud, first: &Person, shown: &str) -> Run {
    let Ok(LinkEntry::Paired { pairing, code }) = LinkEntry::from_entry(shown) else {
        panic!("not a code of 12 characters: {shown}");
    };
    let request = json!({ "pairing": pairing.as_str() });
    let answer = post_as(cloud, &first.token, PAIRING_PATH, Some(&request));
    let (session, share) = hello_in(answer, PAIRING_PATH);
    let replied_with = code
        .respond_paired(&pairing, &session, &share)
        .expect("the hello's share");
    replied(cloud, first, session, replied_with)
}

/// The `session` and share of the hello in `answer`, the status and body `path` answered with.
#[track_caller]
fn hello_in((status, body): (u16, Value), path: &str) -> ([u8; 16], [u8; 32]) {
    assert_eq!(status, 200, "{path}: {body}");
    let hello = unb64(body["hello"].as_str().expect("a hello"));
    let LinkMessage::Hello { session, share } =
        LinkMessage::from_cbor(&hello).expect("a link message")
    else {
        panic!("not a hello: {hello:?}");
    };
    (session, share)
}

/// Sends the run `session` the `reply` of the side keyed by `isk` as `first`'s session, and opens
/// the new device's request.
fn replied(
    cloud: &Cloud,
    first: &Person,
    session: [u8; 16],
    (reply, isk): ([u8; 32], skald_crypto::Isk),
) -> Run {
    send(
        cloud,
        first,
        &LinkMessage::Reply {
            session,
            share: reply,
        },
    );
    let sealed = request_of(cloud, first, &session);
    let opened = isk.request_key().open(&sealed).expect("the request opens");
    let request = LinkRequestBody::from_cbor(&opened).expect("a link request body");
    Run {
        session,
        isk,
        request,
    }
}

/// Sends `message` to the run as `first`'s session.
fn send(cloud: &Cloud, first: &Person, message: &LinkMessage) {
    let request = json!({ "message": b64(&message.to_cbor()) });
    let (status, body) = post_as(cloud, &first.token, MESSAGES_PATH, Some(&request));
    assert!(
        (200..300).contains(&status),
        "POST {MESSAGES_PATH}: {status} {body}"
    );
}

/// The sealed request of the run `session`, once the new device sent it.
fn request_of(cloud: &Cloud, first: &Person, session: &[u8; 16]) -> Vec<u8> {
    let path = format!("{MESSAGES_PATH}/{}", b64(session));
    let deadline = Instant::now() + WAIT;
    loop {
        let (status, body) = cloud.get_as(&path, &first.token);
        assert_eq!(status, 200, "GET {path}: {body}");
        let sealed = (body["messages"].as_array().into_iter().flatten())
            .filter_map(Value::as_str)
            .map(unb64)
            .find_map(|message| match LinkMessage::from_cbor(&message) {
                Ok(LinkMessage::Request { sealed, .. }) => Some(sealed),
                _ => None,
            });
        if let Some(sealed) = sealed {
            return sealed;
        }
        assert!(Instant::now() < deadline, "no request in the run");
        thread::sleep(Duration::from_millis(100));
    }
}

/// What the played issuing device grants: the certificate body, who signs it, the master key, and
/// the link object's body and signer. An honest grant has them as the run asks.
pub struct Granted {
    pub cert: CertBody,
    pub cert_signer: SigningKey,
    pub master: [u8; 32],
    pub link: LinkBody,
    pub link_signer: SigningKey,
}

impl Granted {
    /// The honest grant of `first`'s device for `run`, to the device its request's signing key
    /// names.
    pub fn honest(cloud: &Cloud, first: &Person, run: &Run) -> Self {
        let request = &run.request;
        let device = DeviceId::from_signing_key(&request.signing);
        Granted {
            cert: CertBody {
                account: first.account,
                device,
                kind: request.kind,
                signing: request.signing,
                hpke: request.hpke,
                issuer: Some(first.device),
                issued: now(),
                ssh: None,
                github: None,
                name: request.name.clone(),
                unknown: Unknown::default(),
            },
            cert_signer: copy(&first.key),
            master: first.master,
            link: LinkBody {
                device,
                issuer: first.device,
                anchors: latest(cloud, first).into_iter().collect(),
                personal: Some(personal_space(cloud, first)),
                unknown: Unknown::default(),
            },
            link_signer: copy(&first.key),
        }
    }

    /// Sends it, sealed with `run`'s grant key, as `first`'s session.
    pub fn send(self, cloud: &Cloud, first: &Person, run: &Run) {
        let cert = self.cert.to_cbor();
        let grant = LinkGrantBody {
            master: self.master,
            signature: self.cert_signer.sign(CERT_LABEL, &[&cert]),
            cert,
            link: sign_link(&self.link, &self.link_signer).expect("a link object"),
            unknown: Unknown::default(),
        };
        let sealed = run.isk.grant_key().seal(&grant.to_cbor());
        let session = run.session;
        send(cloud, first, &LinkMessage::Grant { session, sealed });
    }
}

/// The same key as `key`.
fn copy(key: &SigningKey) -> SigningKey {
    SigningKey::from_bytes(&key.to_bytes())
}
