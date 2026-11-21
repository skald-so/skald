//! The person, an agent and the tool list the space's agents from the values that pass, each once
//! per device, with its state, since when, note, runtime, device and trust level; a value past its
//! `expires` shows its agents `offline` with the time.

use skald_proto::{AgentState, DeviceId};

use crate::agents::CLAUDE;
use crate::lines::assert_no_uuid;
use crate::presence::{AVAILABLE, BUSY};
use crate::senders::{BOB_AGENT, LAPTOP, LAPTOP_AGENT, OWN, PEER};
use crate::talk::BOB_HOST;
use crate::values::{
    Directory, Listed, OFFLINE, VALID, agent_and_tool_list, assert_warned_before_peers,
    listed_json, listed_of, object_of, person_line_of, person_lists, status, uploads_until,
};
use crate::world::{now, words};

/// Bob's other agents.
const BOB_CODEX: &str = "codex-w4k7rb";
const BOB_GONE: &str = "gemini-p2c5hd";

/// The runtimes.
const CLAUDE_CODE: &str = "claude-code";
const CODEX: &str = "codex";

/// The notes; none is anywhere else.
const REVIEWING: &str = "listing test: reviewing PR 12";
const MIGRATING: &str = "listing test: running the migration";
const BUILDING: &str = "listing test: the laptop builds the docs";

/// One agent as the list shows it.
struct Shown {
    /// Its owner's handle and its name.
    owner: String,
    name: &'static str,
    /// Its full address.
    address: String,
    device: DeviceId,
    device_name: &'static str,
    level: &'static str,
    state: &'static str,
    since: u64,
    note: Option<&'static str>,
    runtime: Option<&'static str>,
}

impl Shown {
    /// `owner/agent`, as an agent's form names it.
    fn short(&self) -> String {
        format!("{}/{}", self.owner, self.name)
    }

    /// Fails unless `line`, of an agent's form, shows this agent.
    #[track_caller]
    fn assert_line(&self, line: &Listed) {
        assert_eq!(
            (line.device.as_str(), line.level.as_str()),
            (self.device_name, self.level),
            "{line:?}"
        );
        assert!(words(&line.rest).contains(&self.state), "{line:?}");
        for part in self.note.iter().chain(&self.runtime) {
            assert!(line.rest.contains(part), "without {part:?}: {line:?}");
        }
    }
}

/// Bob's device uploads, in the channel and in the default channel, his agent `claude-q3x6np`
/// available with a note and `claude-code`, `codex-w4k7rb` busy with a note and `codex`, and
/// `gemini-p2c5hd` offline; Ada's second machine, played, uploads in the channel her agent
/// `codex-v5d8tm` busy with a note and `codex`.
/// - `skald agents --json` gives one object per agent with its full address, state, since, note,
///   runtime, device, device name and level, `peer` for Bob's and `own` for the laptop's;
/// - the person's `skald agents` shows each once, on a line with its full address, state, note,
///   runtime, device name and level;
/// - claude's `skald agents` shows each on its line `[<owner>/<agent> · <device> · <level>]` with
///   its state, note and runtime, after one warning line, and no ID;
/// - the tool `agents` of claude's session gives the same lines.
#[test]
fn lists_each_agent_with_its_state_note_runtime_device_and_level() {
    let mut directory = Directory::new(
        [4_214_001_001, 4_214_001_002],
        ["ada-lists-the-agents", "bob-shows-his-agents"],
    );
    let mut laptop = directory.second_machine(false, LAPTOP);
    let since = now() - 600;
    let (ada, bob) = (directory.ada_handle(), directory.bob_handle());
    let bob_device = directory.bob().device();
    let shown = [
        Shown {
            owner: bob.clone(),
            name: BOB_AGENT,
            address: directory.bob_agent(BOB_AGENT),
            device: bob_device,
            device_name: BOB_HOST,
            level: PEER,
            state: AVAILABLE,
            since,
            note: Some(REVIEWING),
            runtime: Some(CLAUDE_CODE),
        },
        Shown {
            owner: bob.clone(),
            name: BOB_CODEX,
            address: directory.bob_agent(BOB_CODEX),
            device: bob_device,
            device_name: BOB_HOST,
            level: PEER,
            state: BUSY,
            since: since + 60,
            note: Some(MIGRATING),
            runtime: Some(CODEX),
        },
        Shown {
            owner: bob,
            name: BOB_GONE,
            address: directory.bob_agent(BOB_GONE),
            device: bob_device,
            device_name: BOB_HOST,
            level: PEER,
            state: OFFLINE,
            since: since + 120,
            note: None,
            runtime: None,
        },
        Shown {
            owner: ada,
            name: LAPTOP_AGENT,
            address: directory.ada_agent(LAPTOP_AGENT),
            device: laptop.device(),
            device_name: LAPTOP,
            level: OWN,
            state: BUSY,
            since: since + 180,
            note: Some(BUILDING),
            runtime: Some(CODEX),
        },
    ];
    let states = [AgentState::Available, AgentState::Busy, AgentState::Offline];
    let bobs: Vec<_> = (shown.iter().zip(states))
        .map(|(one, state)| status(&one.address, state, one.since, (one.note, one.runtime)))
        .collect();
    for at in [directory.default_at(), directory.at()] {
        directory.bob_uploads((at, 1), bobs.clone());
    }
    let laptops = &shown[3];
    let laptop_status = status(
        &laptops.address,
        AgentState::Busy,
        laptops.since,
        (laptops.note, laptops.runtime),
    );
    let at = directory.at();
    directory.uploads(&mut laptop, (at, 1), vec![laptop_status]);
    let (machine, folder) = (directory.machine(), directory.folder());

    let objects = listed_json(machine, folder);
    let person = person_lists(machine, folder);
    let (as_claude, tool) = agent_and_tool_list(machine, folder, CLAUDE);

    for one in &shown {
        let object = object_of(&objects, &one.address, one.device);
        assert_eq!(object["state"].as_str(), Some(one.state), "{object}");
        assert_eq!(object["since"].as_u64(), Some(one.since), "{object}");
        assert_eq!(object["note"].as_str(), one.note, "{object}");
        assert_eq!(object["runtime"].as_str(), one.runtime, "{object}");
        assert_eq!(
            object["device_name"].as_str(),
            Some(one.device_name),
            "{object}"
        );
        assert_eq!(object["level"].as_str(), Some(one.level), "{object}");

        let line = person_line_of(&person, one.name);
        assert!(line.contains(&one.address), "without the address: {line}");
        for word in [one.state, one.device_name, one.level] {
            assert!(words(&line).contains(&word), "without {word:?}: {line}");
        }
        for part in one.note.iter().chain(&one.runtime) {
            assert!(line.contains(part), "without {part:?}: {line}");
        }

        one.assert_line(&listed_of(&as_claude.stdout, &one.short()));
        one.assert_line(&listed_of(&tool.text, &one.short()));
    }
    assert_warned_before_peers(&as_claude.stdout);
    assert_no_uuid(&as_claude.shown());
    assert_warned_before_peers(&tool.text);
    assert_no_uuid(&tool.text);
}

/// What Bob's device says of the agent whose value expires.
const LAST_NOTE: &str = "expiry test: watching the nightly build";
const SLEEPER: &str = "claude-s6n3vq";

/// Bob's device uploads a value whose `expires` passed a minute ago, listing his agent
/// `claude-s6n3vq` available. `skald agents --json` gives it `offline`, last seen at that
/// `expires`, and the person's line says `offline`, not `available`.
#[test]
fn value_past_its_expires_shows_its_agents_offline_with_the_time() {
    let mut directory = Directory::new(
        [4_214_002_001, 4_214_002_002],
        ["ada-sees-an-old-value", "bob-left-a-value"],
    );
    let expires = now() - 60;
    let agent = directory.bob_agent(SLEEPER);
    let since = expires - VALID;
    let listed = status(
        &agent,
        AgentState::Available,
        since,
        (Some(LAST_NOTE), Some(CLAUDE_CODE)),
    );
    let at = directory.at();
    let talk = &mut directory.agents.talk;
    uploads_until(
        &talk.world.cloud,
        &mut talk.played[0],
        (at, 1),
        expires,
        vec![listed],
    );
    let bob_device = directory.bob().device();
    let (machine, folder) = (directory.machine(), directory.folder());

    let objects = listed_json(machine, folder);
    let person = person_lists(machine, folder);

    let object = object_of(&objects, &agent, bob_device);
    assert_eq!(object["state"].as_str(), Some(OFFLINE), "{object}");
    assert_eq!(object["last_seen"].as_u64(), Some(expires), "{object}");
    let line = person_line_of(&person, SLEEPER);
    assert!(words(&line).contains(&OFFLINE), "{line}");
    assert!(!words(&line).contains(&AVAILABLE), "{line}");
}
