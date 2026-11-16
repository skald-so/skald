//! As an agent, the CLI's output names nothing of a device set change of another space's member;
//! the person's next run warns of it.

use crate::kit::{RECV, create_space, received, warnings_naming};
use crate::revocations::revoked_by;
use crate::talk::{BOB_HOST, Talk};
use crate::world::{ADD_RULE, AGENT, CLIENT_A, CLIENT_B, dir_of, path, run};

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
    let folder = dir_of(&talk.ada.machine, "work/clientA");
    create_space(&talk.world.cloud, &talk.ada, CLIENT_A);
    let bound = run(&talk.ada.machine, &[ADD_RULE, path(&folder), CLIENT_A]);
    assert!(bound.success, "not bound: {bound:?}");
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

    let agent = (talk.ada.machine)
        .start_in(&folder, &[AGENT, CLAUDE, RECV])
        .finish();

    assert!(agent.success, "{agent:?}");
    let shown = agent.shown().to_lowercase();
    for name in names {
        let name = name.to_lowercase();
        assert!(
            !shown.contains(&name),
            "the agent's output names {name}: {agent:?}"
        );
    }

    let person = received(&talk.ada.machine);

    assert!(
        !warnings_naming(&person, bob.device).is_empty(),
        "the person's run does not warn of Bob's device: {person:?}"
    );
}
