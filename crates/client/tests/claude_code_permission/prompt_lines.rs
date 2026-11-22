//! The person's lines of recorded prompts in a run's output, and what an agent's runs show.

use std::path::Path;

use crate::kit::{RECV, received_as_agent};
use crate::lines::JSON_FLAG;
use crate::machine::Machine;
use crate::world::AGENT;

/// The word of the person's line of a recorded prompt.
pub const PERMISSION_WORD: &str = "permission";

/// The lines of `shown`, a run's output, that hold the word [`PERMISSION_WORD`], in any case.
pub fn permission_lines(shown: &str) -> Vec<String> {
    (shown.lines())
        .filter(|line| line.to_lowercase().contains(PERMISSION_WORD))
        .map(str::to_owned)
        .collect()
}

/// What `agent`'s `skald recv`, then its `skald recv --json`, show in `dir` on `machine`, each
/// with its name.
#[track_caller]
pub fn agent_recv_shown(machine: &Machine, dir: &Path, agent: &str) -> Vec<(&'static str, String)> {
    let plain = received_as_agent(machine, dir, agent).shown();
    let json = (machine.start_in(dir, &[AGENT, agent, RECV, JSON_FLAG])).finish();
    assert!(json.success, "{json:?}");
    vec![("skald recv", plain), ("skald recv --json", json.shown())]
}
