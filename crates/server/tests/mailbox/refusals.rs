//! Refused sends (`docs/protocol/delivery.md`, "Send"): `send-refused` names the problem, the
//! server stores nothing of the envelope, and the connection goes on.
//! - `version-mismatch`: a `v` other than the connection's;
//! - `not-a-member`: a `channel` that the latest revision of `space` does not list, or a sending
//!   device whose account that revision does not list as a member of `channel`, or lists with the
//!   device in `excluded`; and any `mls` envelope, while no channel has an MLS group;
//! - `unknown-recipient`: under `hpke`, `recipients` that is not one device with a certificate on
//!   the server of an account that revision lists as a member of `channel`, not `excluded`.

use skald_proto::{
    Cbor, ChannelId, DeviceId, Envelope, Header, NonEmpty, PROTOCOL_VERSION, Scheme, SendKey,
    SpaceId,
};

use crate::api::Person;
use crate::common::devices::{Device, assert_taken, random};
use crate::fixture::{Fixture, channel_members, exclude};
use crate::rosters::first;
use crate::wire::{
    NOT_A_MEMBER, Socket, UNKNOWN_RECIPIENT, VERSION_MISMATCH, assert_nowhere, envelope,
    envelopes_of, header, key_of, version,
};

/// Checks that `socket` gets `send-refused` with `problem` for the `hpke` envelope of `header`,
/// to one recipient, and that the server stores nothing of it.
#[track_caller]
fn assert_refused(fixture: &Fixture, socket: &mut Socket, header: &Header, problem: &str) {
    let sent = envelope(header);
    let (key, refused) = socket.refusal(header.msg_id, &sent);
    assert_eq!(refused, problem, "the problem");
    assert_eq!(key, key_of(header), "the send-key");
    assert_nowhere(fixture.db(), &sent);
}

/// Checks that no mailbox holds an entry, and that `socket`, alice's, still stores an envelope.
#[track_caller]
fn assert_nothing_stored_and_going_on(fixture: &Fixture, socket: &mut Socket) {
    let t = &fixture.team;
    let mut devices = fixture.laptops().to_vec();
    devices.extend([t.alice.uncertified.id, t.bob.uncertified.id]);
    for device in devices {
        assert_eq!(envelopes_of(fixture.db(), device), Vec::<Vec<u8>>::new());
    }
    let header = fixture.to(t.bob.laptop.id);
    let sent = envelope(&header);
    socket.store(&header, &sent);
    assert_eq!(envelopes_of(fixture.db(), t.bob.laptop.id), [sent]);
}

/// Checks that `person`'s laptop may not send in the channel, and that alice's may not send to
/// `device` there; gives alice's connection.
#[track_caller]
fn assert_cut_off(fixture: &Fixture, person: &Person, device: DeviceId) -> Socket {
    let t = &fixture.team;
    let mut socket = Socket::of(&t.flow, person);
    let to_alice = fixture.to(t.alice.laptop.id);
    assert_refused(fixture, &mut socket, &to_alice, NOT_A_MEMBER);
    let mut alice = Socket::of(&t.flow, &t.alice);
    assert_refused(fixture, &mut alice, &fixture.to(device), UNKNOWN_RECIPIENT);
    alice
}

#[test]
fn an_envelope_of_another_version_is_refused() {
    let fixture = Fixture::new(3_121_230_201);
    let t = &fixture.team;
    let mut alice = Socket::of(&t.flow, &t.alice);
    let mut header = fixture.to(t.bob.laptop.id);
    header.v = version(PROTOCOL_VERSION.get() + 1);

    assert_refused(&fixture, &mut alice, &header, VERSION_MISMATCH);

    assert_nothing_stored_and_going_on(&fixture, &mut alice);
}

#[test]
fn an_envelope_in_a_channel_the_latest_revision_does_not_list_is_refused() {
    let fixture = Fixture::new(3_121_230_211);
    let t = &fixture.team;
    let alice = &t.alice;
    let other = alice.create_space("other");
    assert_taken(
        &alice
            .session
            .store(other, &alice.sign(&first(other, "other", alice))),
    );
    let unlisted = alice.session.channel(t.space);
    let bob = t.bob.laptop.id;

    let mut socket = Socket::of(&t.flow, alice);
    let cases = [
        (
            "a channel issued for the space but in no revision",
            header(t.space, unlisted, bob),
        ),
        (
            "a channel the server never issued",
            header(t.space, ChannelId::generate(), bob),
        ),
        (
            "the channel in another space of the sender's",
            header(other, fixture.general, bob),
        ),
        (
            "the channel in a space that does not exist",
            header(SpaceId::generate(), fixture.general, bob),
        ),
    ];
    for (what, header) in &cases {
        eprintln!("{what}");
        assert_refused(&fixture, &mut socket, header, NOT_A_MEMBER);
    }

    assert_nothing_stored_and_going_on(&fixture, &mut socket);
}

#[test]
fn a_device_whose_account_is_not_a_member_of_the_channel_is_refused() {
    let fixture = Fixture::new(3_121_230_221);
    let t = &fixture.team;

    // dave is a member of the space but not of the channel; eve of neither.
    for person in [&t.dave, &fixture.eve] {
        let mut socket = Socket::of(&t.flow, person);
        assert_refused(
            &fixture,
            &mut socket,
            &fixture.to(t.alice.laptop.id),
            NOT_A_MEMBER,
        );
    }

    let mut alice = Socket::of(&t.flow, &t.alice);
    assert_nothing_stored_and_going_on(&fixture, &mut alice);
}

#[test]
fn an_excluded_device_neither_sends_nor_receives() {
    let mut fixture = Fixture::new(3_121_230_231);
    let (bob_account, bob_laptop) = (fixture.team.bob.account, fixture.team.bob.laptop.id);
    fixture.revise(|body| exclude(body, bob_account, &[bob_laptop]));
    let t = &fixture.team;

    let mut alice = assert_cut_off(&fixture, &t.bob, bob_laptop);

    // Another member's device still receives.
    let header = fixture.to(t.carol.laptop.id);
    let sent = envelope(&header);
    alice.store(&header, &sent);
    assert_eq!(envelopes_of(fixture.db(), t.carol.laptop.id), [sent]);
    assert_eq!(
        envelopes_of(fixture.db(), bob_laptop),
        Vec::<Vec<u8>>::new()
    );
}

#[test]
fn an_hpke_envelope_for_a_device_that_cannot_receive_is_refused() {
    let fixture = Fixture::new(3_121_230_241);
    let t = &fixture.team;
    let mut alice = Socket::of(&t.flow, &t.alice);

    let unknown = Device::new("Unknown").id;
    let cases: [(&str, DeviceId); 5] = [
        (
            "a device of a space member outside the channel",
            t.dave.laptop.id,
        ),
        (
            "a device of an account outside the space",
            fixture.eve.laptop.id,
        ),
        (
            "a channel member's device without a certificate",
            t.bob.uncertified.id,
        ),
        (
            "the sender's own device without a certificate",
            t.alice.uncertified.id,
        ),
        ("a device the server does not know", unknown),
    ];
    for (what, recipient) in cases {
        eprintln!("{what}");
        assert_refused(
            &fixture,
            &mut alice,
            &fixture.to(recipient),
            UNKNOWN_RECIPIENT,
        );
    }

    // Not one device: two, and none.
    let mut two = fixture.to(t.bob.laptop.id);
    two.recipients = NonEmpty::from_vec(vec![t.bob.laptop.id, t.carol.laptop.id]);
    let mut none = fixture.to(t.bob.laptop.id);
    none.recipients = None;
    for header in [two, none] {
        let sent = envelope(&header);
        let (_, problem) = alice.refusal(header.msg_id, &sent);
        assert_eq!(problem, UNKNOWN_RECIPIENT, "{:?}", header.recipients);
        assert_nowhere(fixture.db(), &sent);
    }

    assert_nothing_stored_and_going_on(&fixture, &mut alice);
}

#[test]
fn membership_follows_the_latest_revision() {
    let mut fixture = Fixture::new(3_121_230_251);
    let general = fixture.general;
    // carol leaves the channel and dave joins it.
    let t = &fixture.team;
    let accounts = [t.alice.account, t.bob.account, t.dave.account];
    fixture.revise(|body| channel_members(body, general, &accounts));
    let t = &fixture.team;

    let mut alice = assert_cut_off(&fixture, &t.carol, t.carol.laptop.id);

    let mut dave = Socket::of(&t.flow, &t.dave);
    let header = fixture.to(t.alice.laptop.id);
    let from_dave = envelope(&header);
    dave.store(&header, &from_dave);
    let header = fixture.to(t.dave.laptop.id);
    let to_dave = envelope(&header);
    alice.store(&header, &to_dave);
    assert_eq!(envelopes_of(fixture.db(), t.alice.laptop.id), [from_dave]);
    assert_eq!(envelopes_of(fixture.db(), t.dave.laptop.id), [to_dave]);
    assert_eq!(
        envelopes_of(fixture.db(), t.carol.laptop.id),
        Vec::<Vec<u8>>::new()
    );

    // A revision without the channel.
    fixture.revise(|body| body.channels.retain(|channel| channel.id != general));
    let t = &fixture.team;
    let mut alice = Socket::of(&t.flow, &t.alice);
    assert_refused(
        &fixture,
        &mut alice,
        &fixture.to(t.bob.laptop.id),
        NOT_A_MEMBER,
    );
    assert_eq!(
        envelopes_of(fixture.db(), t.bob.laptop.id),
        Vec::<Vec<u8>>::new()
    );
}

#[test]
fn an_mls_envelope_is_refused_as_not_a_member() {
    let fixture = Fixture::new(3_121_230_261);
    let t = &fixture.team;
    let mut alice = Socket::of(&t.flow, &t.alice);

    let mut to_channel = fixture.to(t.bob.laptop.id);
    to_channel.scheme = Scheme::Mls;
    to_channel.recipients = None;
    let mut to_bob = fixture.to(t.bob.laptop.id);
    to_bob.scheme = Scheme::Mls;
    for header in [to_channel, to_bob] {
        let mut payload = random().to_vec();
        payload.extend_from_slice(&random());
        let sent = Envelope {
            header: header.clone(),
            payload,
        }
        .to_cbor();
        let (key, problem) = alice.refusal(header.msg_id, &sent);
        assert_eq!(problem, NOT_A_MEMBER, "{:?}", header.recipients);
        if header.recipients.is_none() {
            let expected = SendKey {
                msg_id: header.msg_id,
                recipient: None,
            };
            assert_eq!(key, expected, "an mls send-key has no recipient");
        }
        assert_nowhere(fixture.db(), &sent);
    }

    assert_nothing_stored_and_going_on(&fixture, &mut alice);
}
