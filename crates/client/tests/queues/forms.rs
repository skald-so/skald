//! What the queue commands say: an agent's `skald queues` names no ID; with `--json`, `skald
//! queues` and `skald enqueue` give every field.

use serde_json::Value;

use crate::agents::{Agents, CLAUDE};
use crate::fixtures::CHANNEL;
use crate::lines::{JSON_FLAG, json_objects};
use crate::queuing::{QUEUES, Sending, enqueued, run_in, sending, the_one_task};

/// What claude enqueues; no text is anywhere else.
const AS_JSON: &str = "forms test: claude asks for the changelog as json";

/// The text field `field` of `object`.
#[track_caller]
fn text<'o>(object: &'o Value, field: &str) -> &'o str {
    (object[field].as_str()).unwrap_or_else(|| panic!("no text {field} in {object}"))
}

/// As claude in `~/work/acme`, `skald queues --channel <channel>` says `default` and neither the
/// channel's ID nor the queue's; `skald queues --channel <channel> --json` gives `default` as one
/// object with its ID, name, channel, mode, lease and max_attempts. `skald enqueue --channel
/// <channel> --json <text>` gives one object with the stored task's ID, its `#<ref>`, the channel,
/// the queue's ID and name, its deadline, as Bob may claim it, and no addressee's device.
#[test]
fn agent_s_lines_name_no_id_and_json_gives_every_field() {
    let agents = Agents::new(
        [4_265_075_001, 4_265_075_002],
        ["ada-reads-queue-json", "bob-sees-the-json-task"],
    );
    let claude = (agents.folder.as_path(), Some(CLAUDE));
    agents.run_as(CLAUDE);
    let talk = &agents.talk;
    let ada_s: Sending = sending(talk);
    let (channel, queue) = (ada_s.channel.as_str(), ada_s.queue.to_string());
    let ada = &talk.ada;

    let short = run_in(&ada.machine, claude, QUEUES, &[CHANNEL, channel]);
    let json = run_in(&ada.machine, claude, QUEUES, &[CHANNEL, channel, JSON_FLAG]);

    assert!(
        short.success && short.stdout.contains("default"),
        "{short:?}"
    );
    for id in [channel, &queue] {
        assert!(!short.shown().contains(id), "{id} in {short:?}");
    }
    let objects = json_objects(&json.stdout);
    let [object] = objects.as_slice() else {
        panic!("not one queue: {json:?}");
    };
    assert_eq!(text(object, "id"), queue, "{object}");
    assert_eq!(text(object, "name"), "default", "{object}");
    assert_eq!(text(object, "channel"), channel, "{object}");
    assert_eq!(text(object, "mode"), "at-least-once", "{object}");
    assert!(
        object["lease"].is_u64() && object["max_attempts"].is_u64(),
        "{object}"
    );

    let ran = enqueued(
        &ada.machine,
        claude,
        &[CHANNEL, channel, JSON_FLAG, AS_JSON],
    );

    let stored = the_one_task(talk.cloud(), ada, ada_s.at, ada_s.queue);
    let objects = json_objects(&ran.stdout);
    let [object] = objects.as_slice() else {
        panic!("not one task: {ran:?}");
    };
    assert_eq!(
        text(object, "task"),
        stored.task.task.to_string(),
        "{object}"
    );
    assert!(!text(object, "ref").is_empty(), "{object}");
    assert_eq!(text(object, "channel"), channel, "{object}");
    assert_eq!(text(object, "queue"), queue, "{object}");
    assert_eq!(text(object, "queue_name"), "default", "{object}");
    assert!(object["deadline"].is_u64(), "Bob may claim it: {object}");
    assert!(object["device"].is_null(), "{object}");
}
