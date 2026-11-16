//! As an agent, the CLI's output names nothing of a device set change of another space's member;
//! the person's next run warns of it.

use crate::kit::{
    assert_names_none, bind_client_a, create_space, received, received_as_agent, warnings_naming,
};
use crate::revocations::revoked_by;
use crate::talk::{BOB_HOST, Talk};
use crate::world::{CLIENT_A, CLIENT_B};

/// The agent the tests run as.
const CLAUDE: &str = "claude-r7c2vd";

/// Ada's machine follows `clientb`, with a channel whose group holds Bob's device, and `clienta`,
/// to which `~/work/clientA` is bound; Bob is not in `clienta`. Bob's device is revoked. As an
/// agent in `~/work/clientA`, `skald recv` names nothing of `clientb`, its channels or Bob's device;
/// the person's next run warns naming Bob's device.
#[test]
fn agent_names_no_device_set_change_of_another_space_s_member_which_waits_for_the_person() {
    let talk = Talk::in_space(
        CLIENT_B,
        &[4_139_095_001, 4_139_095_002],
        &["ada-desk-revocations", "bob-client-revoked"],
        false,
    );
    create_space(&talk.world.cloud, &talk.ada, CLIENT_A);
    let folder = bind_client_a(&talk.ada.machine);
    let bob = &talk.played[0].person;
    revoked_by(&talk.world.cloud, bob, bob.device);
    let space = &talk.space;
    let names = [
        CLIENT_B.to_owned(),
        space.id.to_string(),
        space.default.to_string(),
        talk.channel.to_string(),
        bob.device.to_string(),
        BOB_HOST.to_owned(),
    ];

    let agent = received_as_agent(&talk.ada.machine, &folder, CLAUDE);

    assert_names_none(&agent, &names);

    let person = received(&talk.ada.machine);

    assert!(
        !warnings_naming(&person, bob.device).is_empty(),
        "the person's run does not warn of Bob's device: {person:?}"
    );
}
