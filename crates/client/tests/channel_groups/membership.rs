//! Keeping a channel's group in line with the roster: the device that signed a revision adding a
//! member to a channel adds the member's devices; a member device removes a device another
//! device's Commit added that the latest revision does not admit, and warns; `skald send` goes
//! through the mailbox before it sends.

use skald_crypto::{MlsContent, MlsFraming};
use skald_proto::{ChannelId, Role};

use crate::creating::{ADA_HOST, BOB_HOST, CAROL_HOST, Team, assert_one_welcome};
use crate::fixtures::{cert_of, fronted, msg_id, send};
use crate::front::{Front, Sent};
use crate::kit::{
    Change, assert_listed_with, assert_members, channels, create_space, group, group_on_server,
    latest, received, revise, warnings_naming,
};
use crate::played::Played;
use crate::world::{ACME, CREATE_CHANNEL, now, run};

/// What `skald recv` shows of the device's own message that a recipient device got.
const DELIVERED: &str = "delivered";

/// The last envelope of a message the front saw sent to `channel`: an `mls` application message.
fn sent_to(front: &Front, channel: ChannelId) -> Sent {
    front
        .sent()
        .into_iter()
        .rfind(|sent| {
            let envelope = sent.envelope.envelope();
            envelope.header.channel == channel
                && matches!(
                    MlsFraming::read(&envelope.payload),
                    Some(MlsFraming::Message {
                        content: MlsContent::Application,
                        ..
                    })
                )
        })
        .unwrap_or_else(|| panic!("no message to {channel}"))
}

/// Ada's device creates a channel, then signs a revision, stored as a fixture, that adds Carol to
/// the space, its default channel and that channel. Ada's next run adds Carol's device to both
/// groups; Carol's device joins from the Welcomes and holds both devices in the channel's group,
/// which Ada's machine lists with both.
#[test]
fn revision_ada_s_device_signs_adding_carol_to_a_channel_has_it_add_carol_s_device() {
    let (world, _front) = fronted();
    let cloud = &world.cloud;
    let ada = world.sign_up(4_135_053_001, "ada-adds-carol", ADA_HOST);
    let mut carol = Played::new(
        cloud,
        world.sign_up(4_135_053_002, "carol-added", CAROL_HOST),
    );
    let space = create_space(cloud, &ada, ACME);
    let created = run(&ada.machine, &[CREATE_CHANNEL, ACME]);
    assert!(created.success, "{created:?}");
    let previous = latest(cloud, &ada.token, space.id);
    let channel: ChannelId = previous.body().channels[1].id;
    revise(
        cloud,
        &previous,
        &ada,
        &Change {
            members: vec![(&carol.person, Role::Member)],
            join: vec![(channel, &carol.person)],
            ..Change::default()
        },
    );

    received(&ada.machine);

    let devices = [ada.device, carol.device()];
    for added in [space.default, channel] {
        let held = group(cloud, &ada.token, space.id, added);
        held.assert_is(2, &devices);
        assert_eq!(held.signer, ada.device, "the GroupInfo's signer");
    }
    carol.take_in(cloud);
    for added in [space.default, channel] {
        assert_one_welcome(&carol.joined_to(added), added, 2, &devices);
        assert_members(&carol.members(added), &devices);
    }
    let listed = channels(&ada.machine, ACME);
    assert_listed_with(&listed, channel, &[ADA_HOST, CAROL_HOST], &[]);
}

/// Bob, an admin, signs a revision that adds Carol to the channel, then one that takes her out
/// again; his device, kept on the first, adds Carol's device. Ada's device processes that Commit,
/// fetches the latest revision, commits the Remove of Carol's device, uploads the new GroupInfo,
/// and warns naming it. Bob's device gets the Remove.
#[test]
fn device_another_member_adds_that_the_latest_revision_does_not_admit_is_removed_with_a_warning() {
    let (world, _front) = fronted();
    let cloud = &world.cloud;
    let ada = world.sign_up(4_135_053_101, "ada-removes", ADA_HOST);
    let mut bob = Played::new(cloud, world.sign_up(4_135_053_102, "bob-behind", BOB_HOST));
    let mut carol = Played::new(
        cloud,
        world.sign_up(4_135_053_103, "carol-let-in", CAROL_HOST),
    );
    let space = create_space(cloud, &ada, ACME);
    let channel = crate::kit::issue_channel(cloud, &ada.token, space.id);
    let previous = latest(cloud, &ada.token, space.id);
    let third = revise(
        cloud,
        &previous,
        &ada,
        &Change {
            members: vec![(&bob.person, Role::Admin), (&carol.person, Role::Member)],
            channels: vec![(channel, vec![&ada, &bob.person])],
            ..Change::default()
        },
    );
    received(&ada.machine);
    bob.take_in(cloud);
    carol.take_in(cloud);
    assert_members(&bob.members(channel), &[ada.device, bob.device()]);
    let fourth = revise(
        cloud,
        &third,
        &bob.person,
        &Change {
            join: vec![(channel, &carol.person)],
            ..Change::default()
        },
    );
    revise(
        cloud,
        &fourth,
        &bob.person,
        &Change {
            leave: vec![(channel, &carol.person)],
            ..Change::default()
        },
    );
    let package = carol.mls.key_package(now()).expect("a KeyPackage");
    let carol_cert = cert_of(cloud, &carol.person);
    let commit = (bob.mls)
        .add_devices(channel, &[package.as_ref()], &[carol_cert], now())
        .expect("a Commit");
    bob.commits(cloud, (space.id, channel), &commit);

    let ran = received(&ada.machine);

    assert!(
        !warnings_naming(&ran, carol.device()).is_empty(),
        "no warning naming the device added: {ran:?}"
    );
    let held = group(cloud, &ada.token, space.id, channel);
    held.assert_is(3, &[ada.device, bob.device()]);
    assert_eq!(held.signer, ada.device, "the GroupInfo's signer");
    bob.take_in(cloud);
    let changes = bob.changes_of(channel);
    let last = changes.last().expect("Ada's Commit");
    assert_eq!(last.sender, ada.device, "the Commit's sender");
    let removed: Vec<_> = last.removed.iter().map(|member| member.device).collect();
    assert_eq!(removed, [carol.device()], "the devices removed");
    assert_members(&bob.members(channel), &[ada.device, bob.device()]);
}

/// Ada's device sends a message to the channel, which Bob's device acks, so Ada's mailbox gets its
/// `delivered`; Bob's device then commits in the channel without uploading a GroupInfo. Ada's
/// `skald send` goes through the mailbox first: it processes the Commit, and, as a member, uploads
/// the new epoch's GroupInfo, and shows nothing of the entries; a later `skald recv` shows the
/// `delivered`.
#[test]
fn send_goes_through_the_mailbox_first_and_shows_none_of_it() {
    let mut team = Team::new(
        [4_135_053_201, 4_135_053_202],
        ["ada-sends-later", "bob-commits-meanwhile"],
        false,
    );
    team.form();
    let at = team.at();
    let first = send(
        &team.ada.machine,
        team.channel,
        "the release train leaves at noon",
    );
    assert!(first.success, "{first:?}");
    let first_id = msg_id(&sent_to(&team.front, team.channel));
    let cloud = &team.world.cloud;
    team.bob.take_in(cloud);
    let empty = (team.bob.mls)
        .commit_empty(team.channel, b"", now())
        .expect("an empty Commit");
    team.bob.commits(cloud, at, &empty);

    let second = send(
        &team.ada.machine,
        team.channel,
        "carol joined, see the wiki",
    );

    assert!(second.success, "{second:?}");
    let shown = second.shown().to_lowercase();
    assert!(
        !shown.contains(DELIVERED) && !shown.contains(&first_id.to_string()),
        "skald send showed a mailbox entry: {second:?}"
    );
    let held = match group_on_server(cloud, &team.ada.token, at.0, at.1) {
        (true, Some(held)) => held,
        other => panic!("no GroupInfo after skald send: {other:?}"),
    };
    held.assert_is(2, &[team.ada.device, team.bob.device()]);
    assert_eq!(held.signer, team.ada.device, "the GroupInfo's signer");

    let later = received(&team.ada.machine);

    let (id, bob_device) = (first_id.to_string(), team.bob.device().to_string());
    let lines: Vec<&str> = later
        .stdout
        .lines()
        .chain(later.stderr.lines())
        .filter(|line| {
            line.to_lowercase().contains(DELIVERED)
                && line.contains(&id)
                && line.contains(&bob_device)
        })
        .collect();
    assert_eq!(lines.len(), 1, "the delivered is not shown once: {later:?}");
}
