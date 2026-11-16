//! Refused streams ("Connecting"): a stream without a proof, or with one the requests over HTTPS
//! refuse, opens nothing, reads nothing and acks nothing, with the requests' refusals; so does a
//! stream whose `Last-Event-ID` is not a `seq`. A challenge works once, whether a stream or a
//! request used it.

use std::thread;
use std::time::Duration;

use skald_proto::{AccountId, DeviceKind};

use crate::common::device_flow::json;
use crate::common::devices::{Device, Key, machine, random, write_devices};
use crate::fixture::Fixture;
use crate::https::{
    AUTH_LABEL, AUTH_REQUIRED, CHALLENGE_REFUSED, DEVICE_REFUSED, SIGNATURE_REFUSED,
    assert_problem, batch_of, labelled, proof,
};
use crate::kit::by_bobs_laptop;
use crate::link::{bytes, messages, seqs};
use crate::sse::{Sse, sent_by_alice};

/// The refusal of an invalid request, a problem `https://skald.so/problems/<name>`.
const INVALID_REQUEST: &str = "invalid-request";

/// Past the lifetime of a challenge: a minute.
const PAST_LIFETIME: Duration = Duration::from_secs(62);

/// bob's laptop, whose mailbox holds two envelopes from alice's, and its streams.
struct Scene {
    f: Fixture,
    bob: Sse,
    sent: Vec<Vec<u8>>,
}

impl Scene {
    fn new(github: u64) -> Self {
        let f = Fixture::new(github);
        let t = &f.team;
        let sent = bytes(&sent_by_alice(&f, t.bob.laptop.id, 2));
        let bob = Sse::of(&t.flow, &t.bob);
        Scene { f, bob, sent }
    }

    /// The proof that names `device` and is signed by `key` over `origin`, on a new challenge of
    /// bob's session.
    fn signed(&self, device: &Device, key: &Key, origin: &str) -> String {
        let challenge = self.bob.https().challenge();
        proof(&challenge, device.id, &key.key, origin)
    }

    /// Checks that a stream of bob's session from cursor 2 with `proof` is refused with `401` and
    /// the problem `name`, any if `None`, and that bob's mailbox still holds both entries.
    #[track_caller]
    fn assert_refused(&self, what: &str, name: Option<&str>, proof: Option<&str>) {
        let refused = self.bob.refused(Some(2), proof);
        assert_problem(&refused, 401, name);
        let left = seqs(self.f.db(), self.f.team.bob.laptop.id);
        assert_eq!(left, [1, 2], "{what}: acked");
    }

    /// Checks that a stream of bob's laptop, proving itself, still gets both entries.
    #[track_caller]
    fn assert_untouched(&self) {
        assert_eq!(self.bob.open(None).catch_up(), messages(1, &self.sent));
    }
}

/// A stream of bob's session without a proof is refused with `auth-required`.
#[test]
fn a_stream_without_a_proof_opens_nothing() {
    let s = Scene::new(3_141_420_201);

    s.assert_refused("no proof", Some(AUTH_REQUIRED), None);

    s.assert_untouched();
}

/// A proved stream that states no protocol version is refused with the problem
/// `protocol-version-invalid`.
#[test]
fn a_stream_that_states_no_version_opens_nothing() {
    let s = Scene::new(3_141_420_211);
    let proof = s.bob.https().proof();

    let Err(refused) = s.bob.request(Some(2), Some(&proof), false) else {
        panic!("a stream opened without a version");
    };

    assert_problem(&refused, 400, None);
    assert_eq!(
        json(&refused)["type"],
        "https://skald.so/problems/protocol-version-invalid"
    );
    assert_eq!(seqs(s.f.db(), s.f.team.bob.laptop.id), [1, 2]);
    s.assert_untouched();
}

/// A proved stream whose cursor is not one `Last-Event-ID` of digits, or that states two, is refused
/// with `invalid-request`.
#[test]
fn a_stream_whose_last_event_id_is_not_a_seq_opens_nothing() {
    let s = Scene::new(3_141_420_271);

    for cursor in [&["two"][..], &["-1"], &["+2"], &["1.5"], &["1", "2"]] {
        let cursor: Vec<String> = cursor.iter().map(|value| (*value).to_owned()).collect();
        let proof = s.bob.https().proof();
        let Err(refused) = s.bob.request_from(&cursor, Some(&proof), true) else {
            panic!("a stream opened from {cursor:?}");
        };

        assert_problem(&refused, 400, Some(INVALID_REQUEST));
        let left = seqs(s.f.db(), s.f.team.bob.laptop.id);
        assert_eq!(left, [1, 2], "{cursor:?}: acked");
    }

    s.assert_untouched();
}

/// A proof that names bob's laptop is refused with `signature-refused` when it is signed by another
/// key, over the address the test reaches the server at or another server's, or with the label of
/// a WebSocket's `auth`.
#[test]
fn a_stream_with_a_proof_by_another_key_over_another_origin_or_label_opens_nothing() {
    let s = Scene::new(3_141_420_221);
    let laptop = &s.f.team.bob.laptop;
    let public = s.bob.https().public().to_owned();
    let base = s.f.team.flow.stand.server.base().to_owned();
    assert_ne!(base, public);

    let by_other_key = s.signed(laptop, &Key::generate(), &public);
    s.assert_refused("another key", Some(SIGNATURE_REFUSED), Some(&by_other_key));
    for origin in [base.as_str(), "https://skald.example"] {
        let over_origin = s.signed(laptop, &laptop.signing, origin);
        s.assert_refused(origin, Some(SIGNATURE_REFUSED), Some(&over_origin));
    }
    let challenge = s.bob.https().challenge();
    let key = &laptop.signing.key;
    let of_auth = labelled(AUTH_LABEL, &challenge, laptop.id, key, &public);
    s.assert_refused("auth's label", Some(SIGNATURE_REFUSED), Some(&of_auth));

    s.assert_untouched();
}

/// A device of `kind` with a certificate of `account` by `master`, which the server's API does not
/// register.
fn certified_as(f: &Fixture, account: AccountId, master: &Key, kind: DeviceKind) -> Device {
    let device = Device::new(&format!("{kind:?}"));
    let cert = device.cert_as(account, kind, master);
    write_devices(f.db(), account, kind, &[(&device, Some(&cert))]);
    device
}

/// A proof of a device other than a certified `client` device of the session's account is refused
/// with `device-refused`: eve's laptop, bob's device without a certificate, a device the server
/// does not know, bob's `browser` and `service` devices, and his revoked phone.
#[test]
fn a_stream_with_a_proof_of_a_device_the_server_refuses_opens_nothing() {
    let s = Scene::new(3_141_420_231);
    let bob = &s.f.team.bob;
    let browser = certified_as(&s.f, bob.account, &bob.master, DeviceKind::Browser);
    let service = certified_as(&s.f, bob.account, &bob.master, DeviceKind::Service);
    let phone = by_bobs_laptop(&s.f, "Phone");
    bob.revoke(&bob.by_device(&bob.laptop, phone.id));
    let unknown = Device::new("Unknown");
    let public = s.bob.https().public().to_owned();

    for (what, device) in [
        ("eve's laptop", &s.f.eve.laptop),
        ("bob's uncertified device", &bob.uncertified),
        ("an unknown device", &unknown),
        ("bob's browser", &browser),
        ("bob's service device", &service),
        ("bob's revoked phone", &phone),
    ] {
        let of_device = s.signed(device, &device.signing, &public);
        s.assert_refused(what, Some(DEVICE_REFUSED), Some(&of_device));
    }

    s.assert_untouched();
}

/// A challenge works once, used or refused, by a stream or a request: a stream on a challenge
/// another stream used, a long-poll used or a refused stream used, and on one the server never
/// issued, is refused with `challenge-refused`; so is a long-poll on a challenge a stream used.
#[test]
fn a_stream_with_a_used_or_unknown_challenge_opens_nothing() {
    let s = Scene::new(3_141_420_241);
    let https = s.bob.https();

    let by_stream = https.challenge();
    let opened = s.bob.open_on(None, &https.proof_on(&by_stream)).catch_up();
    assert_eq!(opened, messages(1, &s.sent));
    let again = https.proof_on(&by_stream);
    s.assert_refused("a stream's", Some(CHALLENGE_REFUSED), Some(&again));
    let poll = https.poll_request(0, 0, Some(https.proof_on(&by_stream)));
    assert_problem(&poll.call(), 401, Some(CHALLENGE_REFUSED));

    let by_poll = https.challenge();
    let polled = https
        .poll_request(0, 0, Some(https.proof_on(&by_poll)))
        .call();
    assert_eq!(batch_of(&polled).entries, messages(1, &s.sent));
    let again = https.proof_on(&by_poll);
    s.assert_refused("a long-poll's", Some(CHALLENGE_REFUSED), Some(&again));

    let by_refused = https.challenge();
    let laptop = &s.f.team.bob.laptop;
    let other_key = proof(&by_refused, laptop.id, &Key::generate().key, https.public());
    s.assert_refused("by another key", Some(SIGNATURE_REFUSED), Some(&other_key));
    let again = https.proof_on(&by_refused);
    s.assert_refused("a refused stream's", Some(CHALLENGE_REFUSED), Some(&again));

    let unknown = https.proof_on(&random());
    s.assert_refused("an unknown one", Some(CHALLENGE_REFUSED), Some(&unknown));

    s.assert_untouched();
}

/// A challenge expires within a minute: past it, a stream on it is refused with
/// `challenge-refused`.
#[test]
fn a_stream_with_an_expired_challenge_opens_nothing() {
    let s = Scene::new(3_141_420_251);
    let expiring = s.bob.https().proof();

    thread::sleep(PAST_LIFETIME);

    s.assert_refused("expired", Some(CHALLENGE_REFUSED), Some(&expiring));
    s.assert_untouched();
}

/// A stream of bob's laptop on another session of his account, proved on a challenge of that
/// session, is refused once the session has signed out.
#[test]
fn a_stream_on_a_challenge_of_an_ended_session_opens_nothing() {
    let s = Scene::new(3_141_420_261);
    let t = &s.f.team;
    let user = t.flow.stand.user(3_141_420_262, "bob-spaces-standin");
    let second = machine(&t.flow, &user);
    let server = &t.flow.stand.server;
    let ending = Sse::on_session(server, &t.flow, &second.token, &t.bob.laptop);
    let proved = ending.https().proof();
    assert_eq!(second.sign_out().status, 204);

    assert_problem(&ending.refused(Some(2), Some(&proved)), 401, None);

    assert_eq!(seqs(s.f.db(), t.bob.laptop.id), [1, 2]);
    s.assert_untouched();
}
