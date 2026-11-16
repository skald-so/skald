//! A revoked device and a full copy of its disk: the copy opens no message sent after the device's
//! removal, even from a compromised server that hides the revocation, and what the copy sends is
//! dropped; the device itself says it is revoked and signs and sends nothing.

use serde_json::json;
use skald_proto::{DeviceId, RawEnvelope};

use crate::kit::{RECV, group_info_request, received, warnings_naming};
use crate::revocations::{
    REVOKED, carried, copy_of, hide_revocations, naming, revoked_by, revoked_by_cli,
    serve_as_not_revoked, space_revocations,
};
use crate::talk::{ADA_HOST, Line, Talk, assert_read, encrypted, stored};
use crate::world::{ACME, CREATE_CHANNEL, now, run};

/// What Bob says after Ada's device is removed; neither text is anywhere else.
const AFTER: [&str; 2] = [
    "the new vault passphrase is in the usual place",
    "rotate the deploy keys before friday",
];

/// What the copy of Bob's disk says after his device is revoked.
const FROM_THE_COPY: &str = "please resend the vault passphrase to me";

/// A full copy of the disk of Ada's machine is taken; then her machine revokes its own device. Bob's
/// device learns of the revocation and removes Ada's device from the channel by a Commit carrying
/// it; Carol's device enters that epoch; Bob's device then says two lines, which Carol's device
/// reads. A compromised server serves the copy as a device that is not revoked, hides the
/// revocation and every GroupInfo from it, and puts Bob's two lines into its catch-up, where they
/// reach it. The copy's `skald recv` shows neither line and warns that it cannot open them.
#[test]
fn copy_of_a_revoked_device_s_disk_opens_no_message_sent_after_its_removal() {
    let mut talk = Talk::new(
        &[4_139_093_001, 4_139_093_002, 4_139_093_003],
        &["ada-copied-disk", "bob-says-after", "carol-reads-after"],
        false,
    );
    let (at, ada) = (talk.at(), talk.ada.device);
    let copy = copy_of(&talk.ada.machine, ADA_HOST);
    revoked_by_cli(&talk.ada.machine, ada);
    let cloud = &talk.world.cloud;
    let from_bob = talk.address(&talk.played[0].person);
    let bob = &mut talk.played[0];
    let learned = naming(&space_revocations(cloud, &bob.person.token, at.0), ada);
    assert!(!learned.is_empty(), "Ada's device is not revoked");
    let commit = (bob.mls)
        .remove_devices(at.1, &[ada], &carried(&learned), now())
        .expect("a Remove");
    bob.commits(cloud, at, &commit);
    bob.uploads_group_info(cloud, at);
    talk.played[1].take_in(cloud);
    let bob = &mut talk.played[0];
    let said: Vec<RawEnvelope> = (1..)
        .zip(AFTER)
        .map(|(counter, text)| encrypted(bob, at, at.1, &Line::new(&from_bob, counter, text)))
        .collect();
    for envelope in &said {
        stored(cloud, bob, envelope);
    }
    let bob_device = bob.device();
    let carol = &mut talk.played[1];
    carol.take_in(cloud);
    let expected: Vec<_> = (1..)
        .zip(AFTER)
        .zip(&said)
        .map(|((counter, text), envelope)| (envelope.envelope().header.msg_id, counter, text))
        .collect();
    assert_read(carol, at.1, (bob_device, &from_bob), &expected);
    serve_as_not_revoked(cloud, ada);
    hide_revocations(cloud, at.0);
    for channel in [talk.space.default, at.1] {
        cloud
            .relay
            .rewrite(&group_info_request(at.0, channel), |_| {
                json!({ "group": true, "group_info": null }).to_string()
            });
    }
    talk.front.inject(ada, said.clone());

    let ran = run(&copy, &[RECV]);

    assert_eq!(talk.front.injected(), 2, "not put into catch-up: {ran:?}");
    let shown = ran.shown();
    for text in AFTER {
        assert!(!shown.contains(text), "the copy shows {text:?}: {ran:?}");
    }
    assert!(
        !warnings_naming(&ran, at.1).is_empty() || !warnings_naming(&ran, bob_device).is_empty(),
        "no warning that it cannot open the messages: {ran:?}"
    );
}

/// Bob's device is revoked and Ada's next run removes it. Bob's device, kept as it was, as a full
/// copy of its disk would be, says a line in the channel, which a proxy puts into Ada's catch-up.
/// Ada's `skald recv` drops it with a warning and shows no text.
#[test]
fn what_a_copy_of_a_revoked_device_s_disk_sends_is_dropped_with_a_warning() {
    let mut talk = Talk::new(
        &[4_139_093_101, 4_139_093_102],
        &["ada-ignores-copy", "bob-copied-disk"],
        false,
    );
    let at = talk.at();
    let cloud = &talk.world.cloud;
    let bob = &talk.played[0].person;
    revoked_by(cloud, bob, bob.device);
    received(&talk.ada.machine);
    let from_bob = talk.address(&talk.played[0].person);
    let copy = &mut talk.played[0];
    let envelope = encrypted(copy, at, at.1, &Line::new(&from_bob, 1, FROM_THE_COPY));
    let bob: DeviceId = copy.device();
    talk.front.inject(talk.ada.device, vec![envelope]);

    let ran = talk.ada_drops(FROM_THE_COPY);

    assert!(
        !warnings_naming(&ran, bob).is_empty() || !warnings_naming(&ran, at.1).is_empty(),
        "no warning of the dropped message: {ran:?}"
    );
}

/// Ada's machine revokes its own device. Its `skald send` fails, says the device is revoked and
/// sends nothing; its `skald recv` says so too.
#[test]
fn revoked_device_says_so_and_sends_nothing() {
    let talk = Talk::new(
        &[4_139_093_201, 4_139_093_202],
        &["ada-revokes-herself", "bob-hears-nothing"],
        false,
    );
    revoked_by_cli(&talk.ada.machine, talk.ada.device);
    let from = talk.front.sent().len();

    let said = talk.send("is anyone still reading this channel");

    assert!(!said.success, "{said:?}");
    assert!(
        said.shown().to_lowercase().contains(REVOKED),
        "does not say revoked: {said:?}"
    );
    assert_eq!(talk.front.sent().len(), from, "sent: {said:?}");
    let ran = run(&talk.ada.machine, &[RECV]);
    assert!(
        ran.shown().to_lowercase().contains(REVOKED),
        "does not say revoked: {ran:?}"
    );
}

/// Ada's machine, whose account is the admin of `acme`, revokes its own device. Its
/// `skald create-channel acme` fails, says the device is revoked and sends the server nothing.
#[test]
fn revoked_admin_device_creates_no_channel() {
    let talk = Talk::new(
        &[4_139_093_301, 4_139_093_302],
        &["ada-admin-revoked", "bob-sees-no-channel"],
        false,
    );
    revoked_by_cli(&talk.ada.machine, talk.ada.device);
    let mark = talk.cloud().relay.mark();

    let said = run(&talk.ada.machine, &[CREATE_CHANNEL, ACME]);

    assert!(!said.success, "{said:?}");
    assert!(
        said.shown().to_lowercase().contains(REVOKED),
        "does not say revoked: {said:?}"
    );
    assert_eq!(talk.cloud().count_sent(mark, "POST "), 0, "sent: {said:?}");
}
