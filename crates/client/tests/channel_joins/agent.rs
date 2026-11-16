//! As an agent, the CLI's output names nothing of the warnings of joining and rejoining in another
//! space's channels; the person's next run shows them.

use skald_proto::Role;

use crate::joins::{
    bob_adds_channel, forked_group_info, intruder, intruder_joins, kept, removal_warnings,
    serve_group_info,
};
use crate::kit::{
    assert_names_none, bind_client_a, create_space, received, received_as_agent, warnings_naming,
};
use crate::talk::Talk;
use crate::world::{CLIENT_A, CLIENT_B, now};

/// The agent the tests run as.
const CLAUDE: &str = "claude-j8w3kp";

/// Ada's machine follows `clientb`, where Bob is an admin, and `clienta`, to which `~/work/clientA`
/// is bound. In `clientb`: Bob's device removes Ada's from the talk's channel, which the latest
/// revision admits her to; a device on Bob's account that the members do not admit joins the
/// default channel by itself; and the relay gives, as the GroupInfo of a channel Bob's device
/// created alone, that of a fork listing another such device. As an agent in `~/work/clientA`,
/// `skald recv` names nothing of `clientb`, its channels or those devices; the person's next run
/// warns of the removal, of the joiner and of the device the GroupInfo lists.
#[test]
fn agent_names_no_warning_of_another_space_s_channels_and_the_person_s_next_run_shows_them() {
    let mut talk = Talk::with(
        CLIENT_B,
        &[4_138_085_001, 4_138_085_002],
        &["ada-desk-joins", "bob-client-joins"],
        false,
        (Role::Admin, |_| {}),
    );
    let (at, channel, default) = (talk.at(), talk.channel, talk.space.default);
    let ada = talk.ada.device;
    let cloud = &talk.world.cloud;
    let machine = &talk.ada.machine;
    create_space(cloud, &talk.ada, CLIENT_A);
    let work = bind_client_a(machine);
    let bob = &mut talk.played[0];
    let removal = (bob.mls)
        .remove_devices(channel, &[ada], b"", now())
        .expect("a Commit removing Ada's device");
    kept(cloud, bob, at, &removal);
    let joiner = intruder(cloud, &bob.person, "bob-server-device");
    intruder_joins(cloud, &joiner, (at.0, default));
    let other = bob_adds_channel(cloud, bob, at.0, &talk.ada);
    let stranger = intruder(cloud, &bob.person, "bob-stranger");
    let forked = forked_group_info(&bob.person, other, &stranger);
    serve_group_info(cloud, (at.0, other), &forked);
    let names = [
        CLIENT_B.to_owned(),
        at.0.to_string(),
        default.to_string(),
        channel.to_string(),
        other.to_string(),
        bob.device().to_string(),
        joiner.id().to_string(),
        stranger.id().to_string(),
    ];

    let agent = received_as_agent(machine, &work, CLAUDE);

    assert_names_none(&agent, &names);

    let person = received(machine);

    assert!(
        !removal_warnings(&person, channel).is_empty(),
        "no warning of the removal: {person:?}"
    );
    assert!(
        !warnings_naming(&person, joiner.id()).is_empty(),
        "no warning naming the joiner: {person:?}"
    );
    assert!(
        !warnings_naming(&person, stranger.id()).is_empty(),
        "no warning naming the device the GroupInfo lists: {person:?}"
    );
}
