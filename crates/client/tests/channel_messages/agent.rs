//! As an agent, the CLI's output names nothing of a message it drops in another space's channel;
//! the person's next run warns of it.

use skald_proto::MessageId;

use crate::kit::{
    assert_names_none, bind_client_a, create_space, received, received_as_agent, warnings_naming,
};
use crate::talk::{Line, Talk, encrypted};
use crate::world::{CLIENT_A, CLIENT_B};

/// The agent the tests run as.
const CLAUDE: &str = "claude-v6j1pt";

/// The text of the message dropped; it is nowhere else.
const DROPPED: &str = "status report for the client b rollout";

/// Ada's machine follows `clientb`, with a channel whose group holds Bob's device, and `clienta`,
/// to which `~/work/clientA` is bound. A proxy puts into Ada's catch-up Bob's content in that
/// channel whose `messageId` is not its `msg_id`. As an agent in `~/work/clientA`, `skald recv`
/// names nothing of `clientb`, its channels, Bob's device or the text; the person's next run warns
/// naming Bob's device and shows no text.
#[test]
fn agent_names_no_drop_in_another_space_s_channel_and_the_person_s_next_run_warns_of_it() {
    let mut talk = Talk::in_space(
        CLIENT_B,
        &[4_136_063_001, 4_136_063_002],
        &["ada-desk-messages", "bob-client-messages"],
        false,
    );
    create_space(&talk.world.cloud, &talk.ada, CLIENT_A);
    let folder = bind_client_a(&talk.ada.machine);
    let mut line = Line::new(&talk.address(&talk.played[0].person), 1, DROPPED);
    line.message_id = Some(MessageId::generate());
    let (at, ada_device) = (talk.at(), talk.ada.device);
    let bob = &mut talk.played[0];
    let envelope = encrypted(bob, at, at.1, &line);
    let bob = bob.device();
    talk.front.inject(ada_device, vec![envelope]);
    let space = &talk.space;
    let names = [
        CLIENT_B.to_owned(),
        space.id.to_string(),
        space.default.to_string(),
        at.1.to_string(),
        bob.to_string(),
        DROPPED.to_owned(),
    ];

    let agent = received_as_agent(&talk.ada.machine, &folder, CLAUDE);

    assert_eq!(talk.front.injected(), 1, "not put into catch-up: {agent:?}");
    assert_names_none(&agent, &names);

    let person = received(&talk.ada.machine);

    assert!(
        !warnings_naming(&person, bob).is_empty(),
        "the person's run does not warn of the dropped message: {person:?}"
    );
    assert!(!person.shown().contains(DROPPED), "{person:?}");
}
