//! `--json` carries the waiting device as a field.

use crate::agents::{Agents, CLAUDE};
use crate::kit::RECV;
use crate::lines::{JSON_FLAG, holds_text, json_objects};
use crate::prompt::{LAPTOP, waiting_device};
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
        assert!(finished.success, "{who}: {finished:?}");
        let objects = json_objects(&finished.stdout);
        let carrying: Vec<_> = (objects.iter())
            .filter(|object| holds_text(object, LAPTOP) && holds_text(object, CLIENT))
            .collect();
        assert_eq!(carrying.len(), 1, "{who}: {objects:#?}");
    }
}
