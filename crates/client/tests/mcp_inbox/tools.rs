//! The answer to `tools/list`: it lists `inbox`, and its size stays within a bound that the later
//! tools keep.

use serde_json::Value;

use crate::client::INBOX;
use crate::talk::ADA_HOST;
use crate::world::{World, dir_of};
use crate::{CLIENT, TOOLS_BOUND, session_in};

/// Ada's machine, signed in, starts a session of `claude-code` in `~/work`. Its `tools/list` lists
/// `inbox`, and its result is at most [`TOOLS_BOUND`] bytes as compact JSON.
#[test]
fn tools_list_names_inbox_and_stays_within_its_bound() {
    let world = World::start();
    let ada = world.sign_up(4_627_051_001, "ada-lists-the-tools", ADA_HOST);
    let folder = dir_of(&ada.machine, "work");
    let mut session = session_in(&ada.machine, &folder, &[], CLIENT);

    let listed = session.tools();

    let names: Vec<&str> = (listed["tools"].as_array().map(Vec::as_slice))
        .unwrap_or_default()
        .iter()
        .filter_map(|tool| tool["name"].as_str())
        .collect();
    assert!(names.contains(&INBOX), "no inbox: {listed}");
    let size = Value::to_string(&listed).len();
    assert!(size <= TOOLS_BOUND, "{size} bytes: {listed}");
}
