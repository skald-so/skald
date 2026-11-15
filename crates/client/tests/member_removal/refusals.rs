//! What the commands refuse: each one on the device of a member that is no admin, and removing a
//! member from the default channel, which keeps every member. A refused command sends nothing.

use std::time::Instant;

use skald_proto::{Role, Roster, SpaceId};

use crate::cloud::Cloud;
use crate::edits::{DEFAULT, Edit, SPACE_CHANGE, assert_refused, assert_refused_as_no_admin, edit};
use crate::fixtures::fronted;
use crate::front::Front;
use crate::kit::{Change, create_space, issue_channel, latest, received, revise, rosters};
use crate::machine::Finished;
use crate::talk::{ADA_HOST, BOB_HOST, CAROL_HOST};
use crate::world::{ACME, MEMBER, Person, assert_role, members, next_by, set_role, store};

/// Fails unless, since `since` and the front's first `from` sends, the run `ran` sent no request
/// that changes a space and no envelope, and `space` still has the revisions `held`, for the
/// session `token`.
#[track_caller]
fn assert_sent_nothing(
    (cloud, front): (&Cloud, &Front),
    (since, from): (Instant, usize),
    (space, token, held): (SpaceId, &str, &[Roster]),
    ran: &Finished,
) {
    assert_eq!(
        cloud.count_sent(since, SPACE_CHANGE),
        0,
        "a request that changes the space: {ran:?}"
    );
    assert_eq!(front.sent().len(), from, "an envelope sent: {ran:?}");
    assert_eq!(rosters(cloud, token, space), held, "the space's revisions");
}

/// Bob's machine creates `acme` and its device signs the revision that adds Ada as an admin, Carol
/// as a member and a channel with Bob and Ada; Ada's device signs the next one, which makes Bob a
/// member. Bob's device, which accepts it, refuses each command, saying Bob is no admin, and sends
/// nothing: removing Carol from the space, barring Carol's device, adding Carol to the channel and
/// removing Ada from it.
#[test]
fn each_command_on_the_device_of_a_member_that_is_no_admin_refuses_and_sends_nothing() {
    let (world, front) = fronted();
    let cloud = &world.cloud;
    let bob = world.sign_up(4_140_104_001, "bob-no-longer-admin", BOB_HOST);
    let ada = world.sign_up(4_140_104_002, "ada-demotes-bob", ADA_HOST);
    let carol = world.sign_up(4_140_104_003, "carol-left-alone", CAROL_HOST);
    let space = create_space(cloud, &bob, ACME).id;
    let channel = issue_channel(cloud, &bob.token, space);
    let change = Change {
        members: vec![(&ada, Role::Admin), (&carol, Role::Member)],
        channels: vec![(channel, vec![&bob, &ada])],
        ..Change::default()
    };
    let with_ada = revise(cloud, &latest(cloud, &bob.token, space), &bob, &change);
    let demoted = next_by(&with_ada, &ada, |list| {
        set_role(list, bob.account, Role::Member)
    });
    store(cloud, &ada.token, &demoted);
    received(&bob.machine);
    assert_role(&members(&bob.machine, ACME), &bob.handle, MEMBER);
    let held = rosters(cloud, &bob.token, space);
    let edits: [Edit; 4] = [
        Edit::RemoveMember(&carol),
        Edit::Exclude(&carol),
        Edit::AddToChannel(channel, &carol, true),
        Edit::RemoveFromChannel(channel, &ada),
    ];

    for refused_edit in edits {
        let (since, from) = (cloud.relay.mark(), front.sent().len());

        let refused = edit(&bob.machine, ACME, refused_edit);

        assert_refused_as_no_admin(&refused);
        assert_sent_nothing(
            (cloud, &front),
            (since, from),
            (space, &bob.token, &held),
            &refused,
        );
    }
}

/// Ada's device, an admin's, refuses to remove Bob from the default channel, saying it is the
/// default channel, and sends nothing.
#[test]
fn removing_a_member_from_the_default_channel_is_refused_and_sends_nothing() {
    let (world, front) = fronted();
    let cloud = &world.cloud;
    let ada = world.sign_up(4_140_104_101, "ada-keeps-default", ADA_HOST);
    let bob = world.sign_up(4_140_104_102, "bob-stays-default", BOB_HOST);
    let space = create_space(cloud, &ada, ACME);
    with_member(cloud, &ada, space.id, &bob);
    received(&ada.machine);
    let held = rosters(cloud, &ada.token, space.id);
    let (since, from) = (cloud.relay.mark(), front.sent().len());

    let refused = edit(
        &ada.machine,
        ACME,
        Edit::RemoveFromChannel(space.default, &bob),
    );

    assert_refused(&refused, DEFAULT);
    assert_sent_nothing(
        (cloud, &front),
        (since, from),
        (space.id, &ada.token, &held),
        &refused,
    );
}

/// `signer`'s device signs the revision that adds `person` as a member of `space`, in its default
/// channel.
fn with_member(cloud: &Cloud, signer: &Person, space: SpaceId, person: &Person) {
    let change = Change {
        members: vec![(person, Role::Member)],
        ..Change::default()
    };
    revise(cloud, &latest(cloud, &signer.token, space), signer, &change);
}
