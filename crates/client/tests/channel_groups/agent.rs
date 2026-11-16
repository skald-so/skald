//! As an agent, the CLI's output, warnings included, names nothing of other spaces' channels; the
//! person's next run shows those warnings.

use skald_proto::Role;

use crate::creating::{ADA_HOST, BOB_HOST};
use crate::fixtures::fronted;
use crate::kit::{
    assert_names_none, bind_client_a, create_space, received, received_as_agent, warnings_naming,
    with_channel,
};
use crate::played::{Played, mls_envelope, stranger, welcome_from};
use crate::world::{CLIENT_A, CLIENT_B};

/// The agent the tests run as.
const CLAUDE: &str = "claude-q4n8wz";

/// Ada's machine follows `clienta` and `clientb`, and `~/work/clientA` is bound to `clienta`. In a
/// channel of `clientb` with Bob, whose group Bob's device created alone, a proxy puts the Welcome
/// of a device that is not admitted into Ada's catch-up. As an agent in `~/work/clientA`, `skald
/// recv` names nothing of `clientb`, its channel or that device; the person's next run warns naming
/// the device.
#[test]
fn agent_s_output_names_no_warning_of_another_space_s_channel_which_waits_for_the_person() {
    let (world, front) = fronted();
    let cloud = &world.cloud;
    let ada = world.sign_up(4_135_055_001, "ada-desk-groups", ADA_HOST);
    let mut bob = Played::new(
        cloud,
        world.sign_up(4_135_055_002, "bob-client-groups", BOB_HOST),
    );
    create_space(cloud, &ada, CLIENT_A);
    let client_b = create_space(cloud, &ada, CLIENT_B);
    let folder = bind_client_a(&ada.machine);
    let channel = with_channel(cloud, client_b.id, &ada, &[(&bob.person, Role::Member)]);
    let at = (client_b.id, channel);
    bob.creates_alone(cloud, at);
    let mut eve = stranger();
    let welcome = welcome_from(cloud, &mut eve, channel, (&ada, &bob.person.token));
    front.inject(ada.device, vec![mls_envelope(at, &welcome, &[ada.device])]);

    let agent = received_as_agent(&ada.machine, &folder, CLAUDE);

    assert_eq!(front.injected(), 1, "not put into catch-up: {agent:?}");
    let names = [
        CLIENT_B.to_owned(),
        client_b.id.to_string(),
        client_b.default.to_string(),
        channel.to_string(),
        eve.id().to_string(),
    ];
    assert_names_none(&agent, &names);

    let person = received(&ada.machine);

    assert!(
        !warnings_naming(&person, eve.id()).is_empty(),
        "the person's run does not warn of the Welcome: {person:?}"
    );
}
