//! A member's message reaches the service agent whose grant lists the channel as one `hpke` copy
//! that opens with the agent's key, with its own counter: a message to the whole channel and one
//! to the agent, but not one to another agent. The server holds only the copies' ciphertext.

use crate::fixtures::database;
use crate::scene::Scene;

/// Two messages of Ada's to the whole channel, and one of Bob's agents.
const FIRST: &str = "the nightly deploy starts at two";
const SECOND: &str = "the staging cluster is drained";
const TO_BOT: &str = "deploy the hotfix branch to staging";
const TO_BOB: &str = "for bob's claude: rerun the flaky suite";

/// Ada's two messages without `to` in the channel of `ci/deploy-bot`'s grant each give the agent
/// one copy, which opens with its key secret's HPKE key and is signed by Ada's device: counters 1
/// and 2, no `to`, her text. The database holds neither text.
#[test]
fn a_message_to_the_whole_channel_reaches_the_service_agent_once() {
    let mut scene = Scene::new(
        [3_155_510_001, 3_155_510_002],
        ["ada-copies-standin", "bob-copies-standin"],
    );
    let channel = scene.talk.channel;

    scene.send_all(channel, FIRST);
    scene.send_all(channel, SECOND);

    let copies = scene.copies();
    scene.assert_copies(&copies, channel, &[(FIRST, 1, None), (SECOND, 2, None)]);
    let held = database(scene.cloud());
    for text in [FIRST, SECOND] {
        assert!(!held.contains(text), "the database holds {text:?}");
    }
}

/// Ada's message to one of Bob's agents gives `ci/deploy-bot` no copy; her next one, `--to
/// ci/deploy-bot`, gives it one, with `to` its full address and counter 1, the first copy it gets.
#[test]
fn a_message_to_the_service_agent_reaches_it_and_one_to_another_agent_does_not() {
    let mut scene = Scene::new(
        [3_155_510_011, 3_155_510_012],
        ["ada-addressed-standin", "bob-addressed-standin"],
    );
    let channel = scene.talk.channel;
    let bob_claude = format!("{}/claude", scene.talk.played[0].person.handle);

    scene.send_to(&bob_claude, channel, TO_BOB);
    scene.send_to("ci/deploy-bot", channel, TO_BOT);

    let copies = scene.copies();
    let address = scene.address.clone();
    scene.assert_copies(&copies, channel, &[(TO_BOT, 1, Some(address.as_str()))]);
}
