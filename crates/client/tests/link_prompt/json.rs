//! `--json` carries the waiting device as a field.

use crate::agents::{Agents, CLAUDE};
use crate::fixtures::{send_args, strs};
use crate::kit::RECV;
use crate::lines::{JSON_FLAG, holds_text, json_objects};
use crate::machine::Finished;
use crate::prompt::{LAPTOP, waiting_device};
use crate::waits::WAIT_FLAG;
use crate::world::AGENT;

/// The kind of Ada's waiting laptop, as `--json` gives it.
const CLIENT: &str = "client";

/// Ada's laptop waits for linking. Her first machine's `skald recv --json`, as the person and as
/// claude in `~/work/acme`, prints only JSON objects on its standard output, one of which holds
/// `ada-laptop` and its kind, `client`, among its values.
#[test]
fn json_carries_the_waiting_device() {
    let agents = Agents::new(
        [4_178_401_001, 4_178_401_002],
        ["ada-reads-json-with-a-laptop", "bob-quiet-for-json"],
    );
    let talk = &agents.talk;
    let _new = waiting_device(&talk.world, &talk.ada, LAPTOP);
    let machine = &talk.ada.machine;

    let person = machine.start(&[RECV, JSON_FLAG]).finish();
    let agent = (machine.start_in(&agents.folder, &[AGENT, CLAUDE, RECV, JSON_FLAG])).finish();

    for (who, finished) in [("the person", person), ("claude", agent)] {
        assert_carries(who, &finished);
    }
}

/// Fails unless `finished`, the run `who` names, succeeded and printed only JSON objects on its
/// standard output, one of which holds `ada-laptop` and its kind, `client`, among its values.
#[track_caller]
fn assert_carries(who: &str, finished: &Finished) {
    assert!(finished.success, "{who}: {finished:?}");
    let objects = json_objects(&finished.stdout);
    let carrying: Vec<_> = (objects.iter())
        .filter(|object| holds_text(object, LAPTOP) && holds_text(object, CLIENT))
        .collect();
    assert_eq!(carrying.len(), 1, "{who}: {objects:#?}");
}

/// Ada's laptop waits for linking. Her first machine's `skald send --json` in the talk's channel,
/// and then her `skald recv --wait --json`, which shows the laptop and exits, each print only JSON
/// objects on their standard output, one of which holds `ada-laptop` and its kind, `client`.
#[test]
fn json_of_send_and_of_a_wait_carries_the_waiting_device() {
    let agents = Agents::new(
        [4_178_402_001, 4_178_402_002],
        ["ada-sends-json-with-a-laptop", "bob-quiet-for-json-sends"],
    );
    let talk = &agents.talk;
    let _new = waiting_device(&talk.world, &talk.ada, LAPTOP);
    let machine = &talk.ada.machine;
    let mut send = send_args(
        talk.channel,
        "link prompt test: sent as JSON while a laptop waits",
    );
    send.insert(1, JSON_FLAG.to_owned());

    let sent = machine.start(&strs(&send)).finish();
    let waited = machine.start(&[RECV, WAIT_FLAG, JSON_FLAG]).finish();

    assert_carries("skald send --json", &sent);
    assert_carries("skald recv --wait --json", &waited);
}
