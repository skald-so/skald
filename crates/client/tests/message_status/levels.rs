//! The status of a message from another member's device gives that device and the level `peer`.

use crate::agents::{Agents, CLAUDE};
use crate::senders::PEER;
use crate::statuses::{Asked, assert_of, bob_asks_claude, json_status};
use crate::world::{ACME, now};

/// What Bob's device says to Ada's claude; it is nowhere else.
const ASKED: &str = "message status test: bob's claude asks ada's claude for the release notes";

/// Bob's device says something to Ada's claude, which claude's `skald recv` shows. claude's
/// `skald message-status --json`, and the person's, by its `msg_id` give Bob's device, the
/// channel, the message's time and the level `peer`.
#[test]
fn message_from_a_member_s_device_gives_peer() {
    let mut agents = Agents::new(
        [4_380_009_001, 4_380_009_002],
        ["ada-hears-a-peer", "bob-speaks-as-a-peer"],
    );
    agents.run_as(CLAUDE);
    let since = now();
    let at = (ACME, agents.talk.at());
    let msg_id = bob_asks_claude(&mut agents.talk, at, (1, ASKED), Asked::default());
    assert!(agents.run_as(CLAUDE).shown().contains(ASKED));
    let bob = agents.talk.played[0].device();
    let channel = agents.talk.channel;

    for agent in [Some(CLAUDE), None] {
        let object = json_status(
            &agents.talk.ada.machine,
            (&agents.folder, agent),
            &msg_id.to_string(),
        );
        assert_of(&object, msg_id, (channel, bob), since);
        assert_eq!(object["level"].as_str(), Some(PEER), "{agent:?}: {object}");
    }
}
