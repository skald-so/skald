//! The new device takes its certificate and the master key only from a run that passes its checks
//! (`membership.md`, "Link code", the new device's step 3). The test plays the issuing device with
//! Ada's first device's key, from the code the new device shows: a grant that is not for the new
//! device's account, ID, kind, keys and name, whose signature or link object fails, or whose master
//! key is not the one the issuer chains to, fails the run, and the new device stores no
//! certificate. The same played device's honest grant links it.

use std::thread;
use std::time::{Duration, Instant};

use serde_json::{Value, json};
use skald_crypto::{LinkCode, SigningKey, sign_link};
use skald_proto::{
    CERT_LABEL, Cbor as _, CertBody, DeviceId, DeviceKind, LinkBody, LinkGrantBody, LinkMessage,
    LinkRequestBody, Unknown,
};

use crate::anchors::{latest, personal_space};
use crate::cloud::{Cloud, WAIT, unb64};
use crate::kit::b64;
use crate::linking::{NewDevice, STORE_CERT, THREE_TIMES, says};
use crate::world::{Person, World, now, post_as};

/// The host name of Ada's new machine.
const LAPTOP: &str = "ada-laptop";

/// The account's latest hello, its path, and the path of a run's later messages.
const HELLO_PATH: &str = "/account/links/hello";
const MESSAGES_PATH: &str = "/account/links/messages";

/// A run with the new device as the issuing device plays it, up to the grant: the code, and the
/// run's keys and what the new device asked for.
pub struct Run {
    session: [u8; 16],
    isk: skald_crypto::Isk,
    request: LinkRequestBody,
}

/// Plays the issuing device on `first`'s session for the run `new` opened: takes the latest
/// hello, sends a reply keyed by `new`'s code, and opens its request.
pub fn answer(cloud: &Cloud, first: &Person, new: &mut NewDevice) -> Run {
    new.open_run(cloud, first);
    let (status, body) = cloud.get_as(HELLO_PATH, &first.token);
    assert_eq!(status, 200, "GET {HELLO_PATH}: {body}");
    let hello = unb64(body["hello"].as_str().expect("a hello"));
    let LinkMessage::Hello { session, share } =
        LinkMessage::from_cbor(&hello).expect("a link message")
    else {
        panic!("not a hello: {hello:?}");
    };
    let code = LinkCode::from_entry(&new.code()).expect("a code");
    let (reply, isk) = code
        .respond(first.account, &session, &share)
        .expect("the hello's share");
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
    /// The honest grant of `first`'s device for `run`.
    pub fn honest(cloud: &Cloud, first: &Person, new: &NewDevice, run: &Run) -> Self {
        let request = &run.request;
        Granted {
            cert: CertBody {
                account: first.account,
                device: new.device,
                kind: request.kind,
                signing: request.signing,
                hpke: request.hpke,
                issuer: Some(first.device),
                issued: now(),
                ssh: None,
                name: request.name.clone(),
                unknown: Unknown::default(),
            },
            cert_signer: copy(&first.key),
            master: first.master,
            link: LinkBody {
                device: new.device,
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

/// A device ID no device of the tests has.
fn stranger() -> DeviceId {
    DeviceId::from_signing_key(&SigningKey::generate().public_key())
}

/// What a refused grant changes in an honest one.
type Change = fn(&mut Granted);

/// The grants the new device must refuse, each by what it changes in an honest one.
fn refused() -> Vec<(&'static str, Change)> {
    vec![
        ("another account", |granted| {
            granted.cert.account = skald_proto::AccountId::generate();
        }),
        ("another device ID", |granted| {
            granted.cert.device = stranger()
        }),
        ("another signing key", |granted| {
            granted.cert.signing = SigningKey::generate().public_key();
        }),
        ("another hpke key", |granted| granted.cert.hpke = [0x42; 32]),
        ("another name", |granted| {
            granted.cert.name = "someone-elses-laptop".to_owned();
        }),
        ("another kind", |granted| {
            granted.cert.kind = DeviceKind::Service
        }),
        ("a signature by another key", |granted| {
            granted.cert_signer = SigningKey::generate();
        }),
        ("another master key", |granted| {
            granted.master = SigningKey::generate().public_key();
        }),
        ("a link object for another device", |granted| {
            granted.link.device = stranger();
        }),
        ("a link object by another key", |granted| {
            granted.link_signer = SigningKey::generate();
        }),
    ]
}

/// The played issuing device sends Ada's new device, run by run, each grant it must refuse, asking
/// it for a fresh code once its code failed 3 times. Each run fails at the new device; it sends no
/// request to store a certificate, and the server holds none of it. Then the played device's
/// honest grant, on a code asked for afresh, links it: it stores a certificate the server holds.
#[test]
fn the_new_device_refuses_a_grant_that_fails_its_checks() {
    let world = World::start();
    let cloud = &world.cloud;
    let ada = world.sign_up(4_175_601_001, "ada-gets-forged-grants", "ada-workstation");
    let mut new = NewDevice::start(&world, &ada, LAPTOP);

    for (failed, (what, change)) in refused().into_iter().enumerate() {
        if failed > 0 && failed % 3 == 0 {
            new.run().wait_for("that the code failed 3 times", |shown| {
                says(shown, &THREE_TIMES)
            });
            new.fresh_code();
        }
        let run = answer(cloud, &ada, &mut new);
        let mut granted = Granted::honest(cloud, &ada, &new, &run);
        change(&mut granted);
        granted.send(cloud, &ada, &run);
        new.wait_for_failures(failed + 1);
        assert_eq!(
            cloud.count_sent(new.started, STORE_CERT),
            0,
            "{what}: a certificate sent"
        );
    }
    let id = new.device;
    let certified = |cloud: &Cloud| {
        cloud
            .devices(&ada.token)
            .into_iter()
            .any(|device| device.id == id && device.cert.is_some())
    };
    assert!(!certified(cloud), "the server holds a certificate");

    new.fresh_code();
    let run = answer(cloud, &ada, &mut new);
    Granted::honest(cloud, &ada, &new, &run).send(cloud, &ada, &run);
    let shown = new.finish();
    assert!(shown.success, "the honest grant failed: {}", shown.text);
    assert!(certified(cloud), "the server holds no certificate");
}
