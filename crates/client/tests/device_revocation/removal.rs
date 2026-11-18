//! Removing a revoked device: a member device that learns of a revocation warns of the device set
//! change, removes the device from each channel by a Commit that carries the revocation, and lists
//! it revoked; one that learns of it from such a Commit holds the device revoked. Its waiting
//! messages expire, and a revision it signs is not accepted.

use skald_proto::{ChannelMember, MessageId, NonEmpty, Role};

use crate::fixtures::fronted;
use crate::kit::{
    Change, assert_listed_with, assert_members, channels, create_space, group, latest, received,
    revise, rosters, warnings_naming,
};
use crate::revocations::{
    account_revocations, assert_listed, assert_removed_carrying, carried, hide_revocations, naming,
    revoked_by, revoked_by_cli, space_revocations,
};
use crate::talk::{ADA_HOST, BOB_HOST, CAROL_HOST, Talk, one_message_sent, received_until};
use crate::world::{ACME, members, next_with, now, plain_lines, serve_revisions, warnings, words};

/// What `skald recv` shows of the device's own message that expired for a recipient device.
const EXPIRED: &str = "expired";

/// Bob's machine revokes its own device. The server holds the revocation, signed by Bob's device.
/// Ada's next run warns naming Bob's device and removes it from both of the space's channels by
/// Commits that carry that revocation, which Carol's device enters; the server's GroupInfo of the
/// channel lists Ada's and Carol's devices; `skald members` lists Bob's device revoked.
#[test]
fn self_revoked_device_is_removed_by_a_commit_carrying_its_revocation() {
    let mut talk = Talk::new(
        &[4_139_092_001, 4_139_092_002, 4_139_092_003],
        &["ada-removes-bob", "bob-revokes-himself", "carol-stays"],
        false,
    );
    let cloud = &talk.world.cloud;
    let bob = &talk.played[0].person;
    revoked_by_cli(&bob.machine, bob.device);
    let stored = naming(&account_revocations(cloud, &bob.token), bob.device);
    assert_eq!(
        stored.len(),
        1,
        "not one revocation of Bob's device: {stored:?}"
    );
    assert_eq!(stored[0].body().signer, Some(bob.device), "{stored:?}");
    let bob = bob.device;

    let ran = received(&talk.ada.machine);

    assert!(
        !warnings_naming(&ran, bob).is_empty(),
        "no warning of Bob's device set: {ran:?}"
    );
    let carol = &mut talk.played[1];
    carol.take_in(cloud);
    for channel in [talk.space.default, talk.channel] {
        assert_removed_carrying(carol, channel, (talk.ada.device, bob), &stored[0]);
        let held = group(cloud, &talk.ada.token, talk.space.id, channel);
        assert_members(&held.members, &[talk.ada.device, carol.device()]);
    }
    assert_listed(&members(&talk.ada.machine, ACME), BOB_HOST, true);
}

/// Bob's device is revoked; the relay hides every revocation from Ada's machine. Carol's device
/// commits the Remove of Bob's device in the channel, carrying the revocation. Ada's next run
/// enters that epoch, warns naming Bob's device, lists the channel without it and Bob's device
/// revoked, and removes it from the default channel by a Commit carrying the same revocation.
#[test]
fn device_that_learns_a_revocation_from_a_commit_holds_the_device_revoked() {
    let mut talk = Talk::new(
        &[4_139_092_101, 4_139_092_102, 4_139_092_103],
        &[
            "ada-told-by-commit",
            "bob-revoked-quietly",
            "carol-removes-bob",
        ],
        false,
    );
    let (at, ada) = (talk.at(), talk.ada.device);
    let cloud = &talk.world.cloud;
    let bob = &talk.played[0].person;
    let revocation = revoked_by(cloud, bob, bob.device);
    let bob = bob.device;
    hide_revocations(cloud, at.0);
    let carol = &mut talk.played[1];
    let learned = naming(&space_revocations(cloud, &carol.person.token, at.0), bob);
    let commit = (carol.mls)
        .remove_devices(at.1, &[bob], &carried(&learned), now())
        .expect("a Remove");
    carol.commits(cloud, at, &commit);
    carol.uploads_group_info(cloud, at);

    let ran = received(&talk.ada.machine);

    assert!(
        !warnings_naming(&ran, bob).is_empty(),
        "no warning of Bob's device set: {ran:?}"
    );
    let listed = channels(&talk.ada.machine, ACME);
    assert_listed_with(&listed, at.1, &[ADA_HOST, CAROL_HOST], &[BOB_HOST]);
    assert_listed(&members(&talk.ada.machine, ACME), BOB_HOST, true);
    let carol = &mut talk.played[1];
    carol.take_in(cloud);
    assert_removed_carrying(carol, talk.space.default, (ada, bob), &revocation);
}

/// Ada sends a message to the channel while Bob's device is offline; Bob's machine then revokes its
/// own device. Ada's `skald recv` shows `expired` for the message and Bob's device.
#[test]
fn messages_waiting_for_the_revoked_device_expire_and_their_sender_shows_expired() {
    let talk = Talk::new(
        &[4_139_092_201, 4_139_092_202],
        &["ada-waits-for-bob", "bob-revoked-away"],
        false,
    );
    let from = talk.front.sent().len();
    let said = talk.send("the quarterly numbers are final");
    assert!(said.success, "{said:?}");
    let msg_id: MessageId = one_message_sent(&talk.front, talk.channel, from);
    let bob = &talk.played[0].person;
    revoked_by_cli(&bob.machine, bob.device);
    let (msg_id, bob) = (msg_id.to_string(), bob.device.to_string());
    let expired = |shown: &str| {
        shown
            .lines()
            .filter(|line| line.to_lowercase().contains(EXPIRED))
            .any(|line| line.contains(&msg_id) && line.contains(&bob))
    };

    let shown = received_until(&talk.ada.machine, expired);

    assert!(expired(&shown), "{shown}");
}

/// Bob, an admin of `acme`, has his device revoked; the relay then gives Ada's machine a revision
/// his device signed adding Carol. Ada's machine does not accept it: `skald members` lists no
/// Carol, and warns.
#[test]
fn revision_a_revoked_device_signs_is_not_accepted() {
    let (world, _front) = fronted();
    let cloud = &world.cloud;
    let ada = world.sign_up(4_139_092_301, "ada-checks-signer", ADA_HOST);
    let bob = world.sign_up(4_139_092_302, "bob-admin-revoked", BOB_HOST);
    let carol = world.sign_up(4_139_092_303, "carol-never-added", CAROL_HOST);
    let space = create_space(cloud, &ada, ACME);
    let change = Change {
        members: vec![(&bob, Role::Admin)],
        ..Change::default()
    };
    revise(cloud, &latest(cloud, &ada.token, space.id), &ada, &change);
    received(&ada.machine);
    revoked_by(cloud, &bob, bob.device);
    let held = rosters(cloud, &ada.token, space.id);
    let previous = held.last().expect("a revision");
    let forged = next_with(previous, (bob.device, &bob.key), |body| {
        let mut list = body.members.clone().into_vec();
        list.push(carol.member(Role::Member));
        body.members = NonEmpty::from_vec(list).expect("a member");
        body.channels[0].members.push(ChannelMember {
            account: carol.account,
            history: true,
        });
    });
    let mut served: Vec<_> = held.iter().collect();
    served.push(&forged);
    serve_revisions(cloud, space.id, &served);

    let shown = members(&ada.machine, ACME);

    let carol = carol.handle.as_str();
    assert!(
        !plain_lines(&shown)
            .iter()
            .any(|line| words(line).contains(&carol)),
        "the revision was accepted: {shown:?}"
    );
    assert!(!warnings(&shown).is_empty(), "no warning: {shown:?}");
}
