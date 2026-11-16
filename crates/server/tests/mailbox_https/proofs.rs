//! The proof of each request ("Connecting"): without one, or with one the server refuses, a
//! request reads, acks and stores nothing. The server refuses what it refuses in a WebSocket's
//! `auth`, and a signature with `auth`'s label; `auth` does not take a request's label.
//! `docs/protocol/test-vectors.md` publishes the proof's hex.

use std::cell::Cell;
use std::net::TcpStream;
use std::thread;
use std::time::Duration;

use skald_proto::{DeviceId, DeviceKind, PROTOCOL_VERSION};
use tungstenite::Message;
use tungstenite::protocol::frame::coding::CloseCode;
use tungstenite::stream::MaybeTlsStream;

use crate::common::devices::{Device, Key, auth_frame, machine, random, write_devices};
use crate::common::{Closed, hex};
use crate::fixture::Fixture;
use crate::https::{
    AUTH_LABEL, AUTH_REQUIRED, CHALLENGE_REFUSED, DEVICE_REFUSED, Https, REQUEST_LABEL,
    SIGNATURE_REFUSED, assert_problem, batch_of, labelled, proof, proof_bytes, signature,
    stored_through,
};
use crate::kit::by_bobs_laptop;
use crate::link::{bytes, messages, seqs};
use crate::wire::{assert_nowhere, envelope};

/// Past the lifetime of a challenge: a minute.
const PAST_LIFETIME: Duration = Duration::from_secs(62);

/// The document of the test vectors.
const VECTORS: &str = include_str!("../../../../docs/protocol/test-vectors.md");
/// The section of the proof's vector.
const VECTOR: &str = "HTTPS proof";
/// The public key of RFC 8032, §7.1, "TEST 1": Alice's device's in the vectors.
const TEST_1: &str = "d75a980182b10ab7d54bfed3c964073a0ee172f3daa62325af021a68f707511a";

/// The scene of the refusals: bob's laptop, whose mailbox holds two envelopes from alice's.
struct Scene {
    f: Fixture,
    bob: Https,
    sent: Vec<Vec<u8>>,
}

impl Scene {
    fn new(github: u64) -> Self {
        let f = Fixture::new(github);
        let t = &f.team;
        let alice = Https::of(&t.flow, &t.alice);
        let sent = bytes(&stored_through(&alice, &f, t.bob.laptop.id, 2));
        let bob = Https::of(&t.flow, &t.bob);
        Scene { f, bob, sent }
    }

    /// The proof of `device`, signed by `key` over the server's public address, on a new
    /// challenge of bob's session.
    fn proof_of(&self, device: DeviceId, key: &Key) -> Option<String> {
        Some(proof(
            &self.bob.challenge(),
            device,
            &key.key,
            self.bob.public(),
        ))
    }

    /// Checks that a long-poll from cursor 2, an ack of 2 and a send to alice by bob's session,
    /// each with what `proof` gives, are refused with `401` and the problem `name`, any if `None`:
    /// bob's mailbox still holds both entries, and nothing of the send is stored.
    #[track_caller]
    fn assert_refused(&self, what: &str, name: Option<&str>, proof: impl Fn() -> Option<String>) {
        let to_alice = envelope(&self.f.to(self.f.team.alice.laptop.id));
        let requests = [
            ("a long-poll", self.bob.poll_request(2, 0, proof())),
            ("an ack", self.bob.ack_request(2, proof())),
            ("a send", self.bob.send_request(&to_alice, true, proof())),
        ];
        for (request_kind, request) in requests {
            eprintln!("{what}: {request_kind}");
            assert_problem(&request.call(), 401, name);
        }
        assert_eq!(
            seqs(self.f.db(), self.f.team.bob.laptop.id),
            [1, 2],
            "{what}"
        );
        assert_nowhere(self.f.db(), &to_alice);
    }

    /// Checks that bob's laptop, proving itself, still gets both entries.
    #[track_caller]
    fn assert_untouched(&self) {
        assert_eq!(self.bob.poll_now(0).entries, messages(1, &self.sent));
    }
}

/// A request with bob's session but without a proof is refused with `auth-required`.
#[test]
fn a_session_without_a_proof_reads_acks_and_stores_nothing() {
    let s = Scene::new(3_141_410_401);

    s.assert_refused("no proof", Some(AUTH_REQUIRED), || None);

    s.assert_untouched();
}

/// A proof that names bob's laptop is refused with `signature-refused` when it is signed by
/// another key, over the address the test reaches the server at or another server's, or with the
/// label of a WebSocket's `auth`; and a WebSocket's `auth` signed with the label of requests is
/// refused too.
#[test]
fn a_proof_by_another_key_over_another_origin_or_with_another_label_is_refused() {
    let s = Scene::new(3_141_410_411);
    let t = &s.f.team;
    let bob = &t.bob.laptop;
    let server = &t.flow.stand.server;
    let other_key = Key::generate();
    assert_ne!(server.base(), s.bob.public());

    s.assert_refused("another key", Some(SIGNATURE_REFUSED), || {
        s.proof_of(bob.id, &other_key)
    });
    for origin in [server.base(), "https://skald.example"] {
        s.assert_refused(origin, Some(SIGNATURE_REFUSED), || {
            Some(proof(&s.bob.challenge(), bob.id, &bob.signing.key, origin))
        });
    }
    s.assert_refused("the label of auth", Some(SIGNATURE_REFUSED), || {
        let challenge = s.bob.challenge();
        let public = s.bob.public();
        Some(labelled(
            AUTH_LABEL,
            &challenge,
            bob.id,
            &bob.signing.key,
            public,
        ))
    });
    s.assert_untouched();

    let challenge = s.bob.challenge();
    let offer = skald_proto::subprotocol(PROTOCOL_VERSION);
    let (mut socket, _) = server
        .open_websocket("/ws", Some(&offer))
        .unwrap_or_else(|response| panic!("/ws refused: {}", response.status));
    if let MaybeTlsStream::Plain(stream) = socket.get_ref() {
        TcpStream::set_read_timeout(stream, Some(Duration::from_secs(15))).expect("a timeout");
    }
    let by_request_label = signature(&bob.signing.key, REQUEST_LABEL, &challenge, s.bob.public());
    let auth = auth_frame(&challenge, bob.id, &by_request_label);
    socket
        .send(Message::Binary(auth.into()))
        .expect("send auth");
    let closed = loop {
        match socket.read() {
            Ok(Message::Close(frame)) => break Closed::of(frame),
            Ok(Message::Ping(_) | Message::Pong(_)) => {}
            other => panic!("a close, not {other:?}"),
        }
    };
    let refused = Closed::Frame(CloseCode::Policy, SIGNATURE_REFUSED.to_owned());
    assert_eq!(closed, refused, "auth with the label of requests");
}

/// A proof of a device other than a certified `client` device of the session's account is refused
/// with `device-refused`: one of eve's account, bob's device without a certificate, a device the
/// server does not know, bob's `browser` and `service` devices, and his revoked phone.
#[test]
fn a_proof_of_a_device_the_server_refuses_is_refused() {
    let s = Scene::new(3_141_410_421);
    let t = &s.f.team;
    let bob = &t.bob;
    let unknown = Device::new("Unknown");
    let mut kinds = Vec::new();
    for (kind, name) in [
        (DeviceKind::Browser, "Browser"),
        (DeviceKind::Service, "CI"),
    ] {
        let device = Device::new(name);
        let cert = device.cert_as(bob.account, kind, &bob.master);
        write_devices(s.f.db(), bob.account, kind, &[(&device, Some(&cert))]);
        kinds.push(device);
    }
    let phone = by_bobs_laptop(&s.f, "Phone");
    bob.revoke(&bob.by_device(&bob.laptop, phone.id));

    let refused = [
        ("eve's laptop", &s.f.eve.laptop),
        ("bob's device without a certificate", &bob.uncertified),
        ("an unknown device", &unknown),
        ("bob's browser", &kinds[0]),
        ("bob's service device", &kinds[1]),
        ("bob's revoked phone", &phone),
    ];
    for (what, device) in refused {
        s.assert_refused(what, Some(DEVICE_REFUSED), || {
            s.proof_of(device.id, &device.signing)
        });
    }
    s.assert_untouched();
}

/// A challenge works once, used or refused, on any instance: a proof on a challenge a request
/// used, through this instance or another, on one a refused proof used, and on one the server
/// never issued, is refused with `challenge-refused`.
#[test]
fn a_used_or_unknown_challenge_is_refused() {
    let s = Scene::new(3_141_410_431);
    let t = &s.f.team;
    let other = t.flow.stand.instance(&[], &[]);
    let on_other = Https::to(&other, &t.flow, &t.bob, &t.bob.laptop);

    let used = s.bob.challenge();
    let taken = s.bob.poll_request(0, 0, Some(s.bob.proof_on(&used))).call();
    assert_eq!(batch_of(&taken).entries, messages(1, &s.sent));
    s.assert_refused("a used challenge", Some(CHALLENGE_REFUSED), || {
        Some(s.bob.proof_on(&used))
    });

    let used_on_other = s.bob.challenge();
    let taken = on_other
        .poll_request(0, 0, Some(on_other.proof_on(&used_on_other)))
        .call();
    assert_eq!(batch_of(&taken).entries, messages(1, &s.sent));
    s.assert_refused(
        "a challenge used on another instance",
        Some(CHALLENGE_REFUSED),
        || Some(s.bob.proof_on(&used_on_other)),
    );

    let refused_once = s.bob.challenge();
    let bob = &t.bob.laptop;
    let by_other_key = proof(&refused_once, bob.id, &Key::generate().key, s.bob.public());
    let refused = s.bob.poll_request(0, 0, Some(by_other_key)).call();
    assert_problem(&refused, 401, Some(SIGNATURE_REFUSED));
    s.assert_refused(
        "a challenge a refused proof used",
        Some(CHALLENGE_REFUSED),
        || Some(s.bob.proof_on(&refused_once)),
    );

    s.assert_refused("an unknown challenge", Some(CHALLENGE_REFUSED), || {
        Some(s.bob.proof_on(&random()))
    });
    s.assert_untouched();
}

/// A challenge works only with its own session's token: bob's laptop, with his session's token,
/// proves itself on a challenge of another live session of his account, and is refused with
/// `challenge-refused`.
#[test]
fn a_challenge_of_another_session_of_the_account_is_refused() {
    let s = Scene::new(3_141_410_461);
    let t = &s.f.team;
    let user = t.flow.stand.user(3_141_410_462, "bob-spaces-standin");
    let other = machine(&t.flow, &user);
    let server = &t.flow.stand.server;
    let on_other = Https::on_session(server, &t.flow, &other.token, &t.bob.laptop);
    assert_eq!(on_other.poll_now(0).entries, messages(1, &s.sent));

    s.assert_refused(
        "a challenge of another session",
        Some(CHALLENGE_REFUSED),
        || Some(s.bob.proof_on(&on_other.challenge())),
    );
    s.assert_untouched();
}

/// A challenge expires within a minute: past it, its proof is refused with `challenge-refused`.
#[test]
fn an_expired_challenge_is_refused() {
    let s = Scene::new(3_141_410_441);
    let challenges: Vec<Vec<u8>> = (0..3).map(|_| s.bob.challenge()).collect();

    thread::sleep(PAST_LIFETIME);

    let next = Cell::new(0);
    s.assert_refused("an expired challenge", Some(CHALLENGE_REFUSED), || {
        let challenge = &challenges[next.get()];
        next.set(next.get() + 1);
        Some(s.bob.proof_on(challenge))
    });
    s.assert_untouched();
}

/// A challenge of a session that has ended is refused, with that session's token: bob's laptop
/// proves itself on a challenge of another session of his account, which then signs out.
#[test]
fn a_challenge_of_an_ended_session_is_refused() {
    let s = Scene::new(3_141_410_451);
    let t = &s.f.team;
    let user = t.flow.stand.user(3_141_410_452, "bob-spaces-standin");
    let second = machine(&t.flow, &user);
    let server = &t.flow.stand.server;
    let ending = Https::on_session(server, &t.flow, &second.token, &t.bob.laptop);
    let challenges: Vec<Vec<u8>> = (0..3).map(|_| ending.challenge()).collect();
    assert_eq!(second.sign_out().status, 204);

    let requests = [
        ending.poll_request(2, 0, Some(ending.proof_on(&challenges[0]))),
        ending.ack_request(2, Some(ending.proof_on(&challenges[1]))),
    ];
    for request in requests {
        assert_problem(&request.call(), 401, None);
    }
    let to_alice = envelope(&s.f.to(t.alice.laptop.id));
    let send = ending.send_request(&to_alice, true, Some(ending.proof_on(&challenges[2])));
    assert_problem(&send.call(), 401, None);

    assert_eq!(seqs(s.f.db(), t.bob.laptop.id), [1, 2]);
    assert_nowhere(s.f.db(), &to_alice);
    s.assert_untouched();
}

/// The bytes in the `hex` block of the section `name` of the test vectors.
fn published_hex(name: &str) -> String {
    let heading = format!("## {name}");
    let mut lines = VECTORS
        .lines()
        .skip_while(|line| *line != heading)
        .skip(1)
        .take_while(|line| !line.starts_with("## "));
    lines
        .by_ref()
        .find(|line| *line == "```hex")
        .unwrap_or_else(|| panic!("no hex block under {heading:?}"));
    lines.take_while(|line| *line != "```").collect()
}

/// The proof the tests send is the published vector's encoding: for Alice's device, of the key of
/// RFC 8032, §7.1, "TEST 1", the 32-byte challenge 00 01 … 1f, and the placeholder signature of 64
/// bytes 5a, as in the vector of `auth`.
#[test]
fn the_proof_is_the_published_test_vector() {
    let key: Vec<u8> = (0..TEST_1.len())
        .step_by(2)
        .map(|at| u8::from_str_radix(&TEST_1[at..at + 2], 16).expect("hex"))
        .collect();
    let key: [u8; 32] = key.try_into().expect("a 32-byte key");
    let alice = DeviceId::from_signing_key(&key);
    let challenge: Vec<u8> = (0..32).collect();

    let encoded = proof_bytes(&challenge, alice, &[0x5a; 64]);

    assert_eq!(published_hex(VECTOR), hex(&encoded));
}
