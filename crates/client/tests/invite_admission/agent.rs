//! As an agent in another space's folder, the admin's device admits a newcomer, and names nothing
//! of that space: its warnings wait for the person's run.

use crate::kit::{assert_names_none, bind_client_a, create_space, received, received_as_agent};
use crate::world::{CLIENT_A, CLIENT_B};
use crate::{Scene, assert_warned, lists};

/// The agent Ada's machine runs as.
const CLAUDE: &str = "claude-k2v9rq";
/// The host name of Eve's machine, its certificate's `name`.
const EVE_HOST: &str = "eve-notebook";

/// The scene is in `clientb`; Ada's machine also creates `clienta` and binds `~/work/clientA` to
/// it. Bob accepts one invite to `clientb`; Eve, of a third account, accepts another one asking for
/// Carol's handle. As an agent in `~/work/clientA`, Ada's `skald recv` admits Bob to `clientb`, and
/// names neither `clientb`, its ID or channels, Bob's handle nor Carol's. The person's next run
/// warns of Eve's acceptance, holding Carol's handle.
#[test]
fn as_an_agent_the_admin_s_device_admits_and_the_warnings_wait_for_the_person_s_run() {
    let scene = Scene::new(
        CLIENT_B,
        &[
            (4_855_701_001, "ada-agent-admits"),
            (4_855_701_002, "carol-of-clientb"),
            (4_855_701_003, "bob-joins-the-client"),
        ],
    );
    let (cloud, ada) = (scene.cloud(), &scene.ada);
    let eve = scene
        .world
        .sign_up(4_855_701_004, "eve-wants-carol", EVE_HOST);
    create_space(cloud, ada, CLIENT_A);
    let work = bind_client_a(&ada.machine);
    let for_bob = scene.invite();
    scene.bob_accepts(&for_bob);
    let for_eve = scene.invite();
    let taken = scene.carol.person.handle.to_string();
    for_eve.accepted_by(cloud, &eve, &taken);
    let names = [
        CLIENT_B.to_owned(),
        scene.space.id.to_string(),
        scene.space.default.to_string(),
        scene.channel.to_string(),
        scene.bob.person.handle.to_string(),
        taken.clone(),
    ];

    let agent = received_as_agent(&ada.machine, &work, CLAUDE);

    assert_names_none(&agent, &names);
    assert!(
        lists(&scene.latest(), &scene.bob.person),
        "Bob is not admitted: {agent:?}"
    );
    assert!(!lists(&scene.latest(), &eve), "Eve is admitted: {agent:?}");

    let person = received(&ada.machine);

    assert_warned(&person, &taken, &["handle"]);
}
