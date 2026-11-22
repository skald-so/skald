//! The status of a message copied to a service agent lists the agent's device as a recipient.

use serde_json::Value;
use skald_proto::DeviceId;

use crate::kit::RECV;
use crate::machine::Machine;
use crate::scene::Scene;
use crate::world::run;

/// What Ada says to the whole channel; it is nowhere else.
const WATCHED: &str = "message status test: the canary rollout begins at noon";

/// The status of `device` among the recipients of the message `msg_id` in Ada's
/// `skald message-status --json`; fails unless it lists the device once.
#[track_caller]
fn status_of(machine: &Machine, msg_id: &str, device: DeviceId) -> String {
    let finished = run(machine, &["message-status", "--json", msg_id]);
    assert!(finished.success, "{finished:?}");
    let object: Value = serde_json::from_str(finished.stdout.trim())
        .unwrap_or_else(|err| panic!("not one JSON object: {err}: {finished:?}"));
    let id = device.to_string();
    let listed: Vec<&Value> = (object["recipients"].as_array().map(Vec::as_slice))
        .unwrap_or_default()
        .iter()
        .filter(|one| one["device"].as_str() == Some(id.as_str()))
        .collect();
    let [listed] = listed.as_slice() else {
        panic!("not one recipient {id}: {object}");
    };
    listed["status"].as_str().unwrap_or_default().to_owned()
}

/// Ada's message to the whole channel reaches `ci/deploy-bot` as an `hpke` copy. Ada's
/// `skald message-status --json` of it lists the agent's device at `stored`, and at `delivered`
/// once the agent took the copy in and Ada's `skald recv` took in the server's `delivered`.
#[test]
fn a_copied_message_lists_the_service_device() {
    let mut scene = Scene::new(
        [3_155_510_031, 3_155_510_032],
        ["ada-status-standin", "bob-status-standin"],
    );
    let channel = scene.talk.channel;
    scene.send_all(channel, WATCHED);
    let copies = scene.copies();
    let [copy] = copies.as_slice() else {
        panic!("not one copy: {copies:?}");
    };
    let msg_id = copy.header.msg_id.to_string();
    let (machine, bot) = (&scene.talk.ada.machine, scene.bot());

    assert_eq!(status_of(machine, &msg_id, bot), "stored");

    let received = run(machine, &[RECV]);
    assert!(received.success, "{received:?}");
    assert_eq!(status_of(machine, &msg_id, bot), "delivered");
}
