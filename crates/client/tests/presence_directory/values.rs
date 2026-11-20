//! What the tests share: presence values a device seals with its keys and uploads, values a proxy
//! puts into or leaves out of the server's answers, and what the CLI shows of the agents.

use std::path::Path;

use base64::Engine as _;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use serde_json::{Value, json};
use skald_crypto::MlsDevice;
use skald_proto::{
    AgentState, AgentStatus, Cbor as _, ChannelId, DeviceId, PROOF_HEADER, PROTOCOL_VERSION,
    Presence, PresenceBody, REQUEST_LABEL, RequestProof, SpaceId, VERSION_HEADER,
};

use crate::agents::Agents;
use crate::client::{CLIENT, Called, session_in};
use crate::cloud::{Cloud, agent, unb64};
use crate::devices::second_machine;
use crate::kit::received;
use crate::lines::{JSON_FLAG, json_objects};
use crate::machine::{Finished, Machine};
use crate::played::Played;
use crate::senders::{OWN, data_warnings};
use crate::world::{ACME, AGENT, HOST, Person, WARNING, now, post_as, warnings, words};

/// The command that lists the agents, and the MCP tool that does.
pub const AGENTS: &str = "agents";
pub const AGENTS_TOOL: &str = "agents";

/// The states as the CLI shows them.
pub const AVAILABLE: &str = "available";
pub const BUSY: &str = "busy";
pub const OFFLINE: &str = "offline";

/// The word of a warning of a dropped value.
pub const PRESENCE: &str = "presence";

/// How long a value the tests upload stays valid, as a device sets `expires`.
pub const VALID: u64 = 900;

/// The status of `agent`, a full address, in `state` since `since`, with `note` and `runtime`.
pub fn status(
    agent: &str,
    state: AgentState,
    since: u64,
    (note, runtime): (Option<&str>, Option<&str>),
) -> AgentStatus {
    AgentStatus {
        agent: agent
            .parse()
            .unwrap_or_else(|err| panic!("{agent}: {err:?}")),
        state,
        since,
        note: note.map(str::to_owned),
        runtime: runtime.map(|name| name.parse().expect("a runtime name")),
        unknown: Default::default(),
    }
}

/// The value of `device`'s machine in `at`'s channel, sealed under the presence key of the epoch
/// `keys` holds as the channel's current one, signed by the device's key: `revision`, `expires`,
/// `agents` and no queue.
#[track_caller]
pub fn sealed(
    keys: &MlsDevice,
    at: (SpaceId, ChannelId),
    device: &Person,
    (revision, expires): (u64, u64),
    agents: Vec<AgentStatus>,
) -> Presence {
    let channel = at.1;
    let epoch =
        (keys.epoch(channel)).unwrap_or_else(|err| panic!("no group of {channel}: {err:?}"));
    let key = (keys.history_key(channel, epoch))
        .unwrap_or_else(|| panic!("no history key of {channel} in epoch {epoch}"))
        .presence_key();
    let body = PresenceBody {
        device: device.device,
        channel,
        epoch,
        revision,
        expires,
        agents,
        serves: Vec::new(),
        unknown: Default::default(),
    };
    key.seal(&body, &device.key)
        .expect("a sealed presence value")
}

/// The path of `at`'s channel's presence.
pub fn presence_path((space, channel): (SpaceId, ChannelId)) -> String {
    format!("/spaces/{space}/channels/{channel}/presence")
}

/// `device`'s machine uploads `value` as its value in `at`'s channel, proving its device on its
/// session as `/mailbox/send` takes it; fails unless the server keeps it.
#[track_caller]
pub fn upload(cloud: &Cloud, device: &Person, at: (SpaceId, ChannelId), value: &Presence) {
    let (status, body) = post_as(cloud, &device.token, "/session/challenge", None);
    assert!(
        (200..300).contains(&status),
        "no challenge: {status} {body}"
    );
    let challenge = (body["challenge"].as_str())
        .map(unb64)
        .unwrap_or_else(|| panic!("a challenge in base64url: {body}"));
    let signature = (device.key).sign(REQUEST_LABEL, &[&challenge, cloud.public.as_bytes()]);
    let proof = RequestProof {
        challenge,
        device: device.device,
        signature,
    };
    let path = presence_path(at);
    let mut response = agent()
        .put(format!("{}{path}", cloud.url))
        .header(VERSION_HEADER, PROTOCOL_VERSION.to_string())
        .header("authorization", format!("Bearer {}", device.token))
        .header(PROOF_HEADER, URL_SAFE_NO_PAD.encode(proof.to_cbor()))
        .header("content-type", "application/cbor")
        .send(&value.to_cbor()[..])
        .unwrap_or_else(|err| panic!("PUT {path}: {err}"));
    let status = response.status().as_u16();
    let answer = response.body_mut().read_to_string().unwrap_or_default();
    assert!(
        (200..300).contains(&status),
        "the value of {} is not kept: {status} {answer}",
        device.device
    );
}

/// `played` takes in its mailbox, then seals with its own keys its value of `revision` in `at`'s
/// channel, valid for [`VALID`] seconds, listing `agents`, and uploads it. Gives the value.
#[track_caller]
pub fn uploads(
    cloud: &Cloud,
    played: &mut Played,
    (at, revision): ((SpaceId, ChannelId), u64),
    agents: Vec<AgentStatus>,
) -> Presence {
    uploads_until(cloud, played, (at, revision), now() + VALID, agents)
}

/// As [`uploads`], with `expires`.
#[track_caller]
pub fn uploads_until(
    cloud: &Cloud,
    played: &mut Played,
    (at, revision): ((SpaceId, ChannelId), u64),
    expires: u64,
    agents: Vec<AgentStatus>,
) -> Presence {
    played.take_in(cloud);
    let value = sealed(&played.mls, at, &played.person, (revision, expires), agents);
    upload(cloud, &played.person, at, &value);
    value
}

/// From now on, a proxy gives the machines `at`'s channel's values without those of the devices
/// `withheld`, and with `added` after them.
pub fn serve_values(
    cloud: &Cloud,
    at: (SpaceId, ChannelId),
    (withheld, added): (&[DeviceId], &[Presence]),
) {
    let withheld = withheld.to_vec();
    let added: Vec<String> = (added.iter())
        .map(|value| URL_SAFE_NO_PAD.encode(value.to_cbor()))
        .collect();
    let request = format!("GET {}", presence_path(at));
    cloud.relay.rewrite(&request, move |answer| {
        let mut body: Value =
            serde_json::from_str(answer).unwrap_or_else(|err| panic!("not JSON: {err}: {answer}"));
        let values =
            (body["values"].as_array_mut()).unwrap_or_else(|| panic!("no values in {answer}"));
        values.retain(|text| {
            let bytes = unb64(text.as_str().expect("a value in base64url"));
            let value = Presence::from_cbor(&bytes).expect("a presence value");
            !withheld.contains(&value.device)
        });
        values.extend(added.iter().cloned().map(Value::String));
        body.to_string()
    });
}

/// Runs `skald <prefix> agents <args>` on `machine` in `dir`; fails unless it succeeds.
#[track_caller]
pub fn listed_in(machine: &Machine, dir: &Path, prefix: &[&str], args: &[&str]) -> Finished {
    let all: Vec<&str> = (prefix.iter().copied())
        .chain([AGENTS])
        .chain(args.iter().copied())
        .collect();
    let listed = machine.start_in(dir, &all).finish();
    assert!(listed.success, "skald {}: {listed:?}", all.join(" "));
    listed
}

/// Runs `skald agents` as the person on `machine` in `dir`.
#[track_caller]
pub fn person_lists(machine: &Machine, dir: &Path) -> Finished {
    listed_in(machine, dir, &[], &[])
}

/// Runs `skald --agent <agent> agents` on `machine` in `dir`.
#[track_caller]
pub fn agent_lists(machine: &Machine, dir: &Path, agent: &str) -> Finished {
    listed_in(machine, dir, &[AGENT, agent], &[])
}

/// What the agent `agent` lists on `machine` in `dir`: its `skald agents`, then the tool `agents`
/// of a session of `claude-code` it starts there as `--agent <agent>`; fails if the tool fails.
#[track_caller]
pub fn agent_and_tool_list(machine: &Machine, dir: &Path, agent: &str) -> (Finished, Called) {
    let listed = agent_lists(machine, dir, agent);
    let mut session = session_in(machine, dir, &[AGENT, agent], CLIENT);
    let tool = session.call(AGENTS_TOOL, json!({}));
    assert!(!tool.error, "the tool failed: {tool:?}");
    (listed, tool)
}

/// Fails unless `shown`, an agent's output, has a line of its form at a level other than `own`, and
/// one warning line, before the first such line.
#[track_caller]
pub fn assert_warned_before_peers(shown: &str) {
    let first = (listed_lines(shown).into_iter())
        .find(|(_, line)| line.level != OWN)
        .map(|(at, _)| at)
        .unwrap_or_else(|| panic!("no line of another level: {shown}"));
    let warnings = data_warnings(shown);
    assert!(
        matches!(warnings.as_slice(), [warning] if *warning < first),
        "not one warning line before the first line of another level: {shown}"
    );
}

/// The objects `skald agents --json` prints as the person on `machine` in `dir`.
#[track_caller]
pub fn listed_json(machine: &Machine, dir: &Path) -> Vec<Value> {
    json_objects(&listed_in(machine, dir, &[], &[JSON_FLAG]).stdout)
}

/// The objects of `objects` of the agent `agent`, a full address.
pub fn objects_of<'o>(objects: &'o [Value], agent: &str) -> Vec<&'o Value> {
    (objects.iter())
        .filter(|object| object["agent"].as_str() == Some(agent))
        .collect()
}

/// The note `skald agents --json` gives, as the person on `machine` in `dir`, of the agent `agent`
/// on the device `device`.
#[track_caller]
pub fn listed_note(machine: &Machine, dir: &Path, agent: &str, device: DeviceId) -> Option<String> {
    let objects = listed_json(machine, dir);
    let note = &object_of(&objects, agent, device)["note"];
    note.as_str().map(str::to_owned)
}

/// The one object of `objects` of the agent `agent` on the device `device`; fails unless there is
/// one.
#[track_caller]
pub fn object_of<'o>(objects: &'o [Value], agent: &str, device: DeviceId) -> &'o Value {
    let id = device.to_string();
    let found: Vec<&Value> = (objects_of(objects, agent).into_iter())
        .filter(|object| object["device"].as_str() == Some(id.as_str()))
        .collect();
    let [object] = found.as_slice() else {
        panic!("not one object of {agent} on {device}: {objects:#?}");
    };
    object
}

/// A line of an agent's form: `[<agent> · <device> · <level>] <rest>`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Listed {
    /// `owner/agent`.
    pub agent: String,
    /// The device's certificate `name`, as the line shows it.
    pub device: String,
    pub level: String,
    /// The state, runtime and note.
    pub rest: String,
}

/// `line` read as a line of an agent's form, if it is one: the brackets end at its first `]`, and
/// hold three parts between ` · `.
pub fn listed(line: &str) -> Option<Listed> {
    let (who, rest) = line.strip_prefix('[')?.split_once(']')?;
    let parts: Vec<&str> = who.split(" · ").collect();
    let [agent, device, level] = parts.as_slice() else {
        return None;
    };
    Some(Listed {
        agent: (*agent).to_owned(),
        device: (*device).to_owned(),
        level: (*level).to_owned(),
        rest: rest.trim().to_owned(),
    })
}

/// The lines of an agent's form in `shown`, each with its line's index.
pub fn listed_lines(shown: &str) -> Vec<(usize, Listed)> {
    (shown.lines().enumerate())
        .filter_map(|(at, line)| listed(line).map(|got| (at, got)))
        .collect()
}

/// The one line of an agent's form in `shown` of `agent`, as `owner/agent`; fails unless there is
/// one.
#[track_caller]
pub fn listed_of(shown: &str, agent: &str) -> Listed {
    let found: Vec<Listed> = (listed_lines(shown).into_iter())
        .map(|(_, got)| got)
        .filter(|got| got.agent == agent)
        .collect();
    let [got] = found.as_slice() else {
        panic!("not one line of {agent}: {shown}");
    };
    got.clone()
}

/// The lines of `finished` that are no warnings and hold `name` as a word.
pub fn plain_lines_naming(finished: &Finished, name: &str) -> Vec<String> {
    (finished.shown().lines())
        .filter(|line| !line.to_lowercase().contains(WARNING))
        .filter(|line| words(line).contains(&name))
        .map(str::to_owned)
        .collect()
}

/// The one line of the person's form in `finished` that holds the agent's name `name`; fails unless
/// there is one.
#[track_caller]
pub fn person_line_of(finished: &Finished, name: &str) -> String {
    match plain_lines_naming(finished, name).as_slice() {
        [line] => line.clone(),
        other => panic!("not one line of {name}: {other:?} in {finished:?}"),
    }
}

/// Fails unless a warning line of `finished`, the person's run, holds the word [`PRESENCE`] and
/// names `device` by `name` or its ID.
#[track_caller]
pub fn assert_warned_of_drop(finished: &Finished, (device, name): (DeviceId, &str)) {
    let id = device.to_string();
    let name = name.to_lowercase();
    let found = (warnings(finished).into_iter()).any(|line| {
        words(&line).contains(&PRESENCE) && (line.contains(&id) || line.contains(&name))
    });
    assert!(found, "no warning of {name}'s value: {finished:?}");
}

/// Fails if a line of `finished` that is no warning holds `name`.
#[track_caller]
pub fn assert_not_listed(finished: &Finished, name: &str) {
    let lines: Vec<String> = (finished.shown().lines())
        .filter(|line| !line.to_lowercase().contains(WARNING))
        .filter(|line| line.contains(name))
        .map(str::to_owned)
        .collect();
    assert!(lines.is_empty(), "{name} listed: {lines:?} in {finished:?}");
}

/// Ada's agents in `~/work/acme` with Bob's played device in the channel and its space's default
/// channel, as the tests of handing messages over have them.
pub struct Directory {
    pub agents: Agents,
}

impl Directory {
    /// Ada, GitHub user `ids[0]` with `logins[0]`, and Bob, the next ones.
    pub fn new(ids: [u64; 2], logins: [&str; 2]) -> Self {
        Directory {
            agents: Agents::new(ids, logins),
        }
    }

    pub fn cloud(&self) -> &Cloud {
        &self.agents.talk.world.cloud
    }

    /// Ada's machine.
    pub fn machine(&self) -> &Machine {
        &self.agents.talk.ada.machine
    }

    /// `~/work/acme` on Ada's machine.
    pub fn folder(&self) -> &Path {
        &self.agents.folder
    }

    /// The channel, in `acme`.
    pub fn at(&self) -> (SpaceId, ChannelId) {
        self.agents.talk.at()
    }

    /// `acme`'s default channel.
    pub fn default_at(&self) -> (SpaceId, ChannelId) {
        (self.agents.talk.space.id, self.agents.talk.space.default)
    }

    /// Bob's played device.
    pub fn bob(&mut self) -> &mut Played {
        &mut self.agents.talk.played[0]
    }

    /// Ada's handle, and Bob's.
    pub fn ada_handle(&self) -> String {
        self.agents.talk.ada.handle.to_string()
    }

    pub fn bob_handle(&self) -> String {
        self.agents.talk.played[0].person.handle.to_string()
    }

    /// The full address of Ada's agent `name` in `acme`, and of Bob's.
    pub fn ada_agent(&self, name: &str) -> String {
        format!("{ACME}/{}/{name}@{HOST}", self.ada_handle())
    }

    pub fn bob_agent(&self, name: &str) -> String {
        format!("{ACME}/{}/{name}@{HOST}", self.bob_handle())
    }

    /// A second machine of Ada's, or of Bob's if `bob`, with the certificate `name` `host`, linked
    /// by hand and played; Bob's device adds it to the channel's group, it joins from the Welcome,
    /// and Ada's machine takes in the Commit.
    #[track_caller]
    pub fn second_machine(&mut self, bob: bool, host: &'static str) -> Played {
        let talk = &mut self.agents.talk;
        let first = if bob {
            &talk.played[0].person
        } else {
            &talk.ada
        };
        let person = second_machine(&talk.world, first, host);
        let cloud = &talk.world.cloud;
        let mut second = Played::new(cloud, person);
        let at = talk.at();
        talk.played[0].welcomes(cloud, at, &second.person);
        second.take_in(cloud);
        received(&talk.ada.machine);
        second
    }

    /// `played`, a device of the scene, uploads its value of `revision` in `at`'s channel listing
    /// `agents`, as [`uploads`] does.
    #[track_caller]
    pub fn uploads(
        &self,
        played: &mut Played,
        (at, revision): ((SpaceId, ChannelId), u64),
        agents: Vec<AgentStatus>,
    ) -> Presence {
        uploads(self.cloud(), played, (at, revision), agents)
    }

    /// Bob's device uploads its value of `revision` in `at`'s channel listing `agents`.
    #[track_caller]
    pub fn bob_uploads(
        &mut self,
        (at, revision): ((SpaceId, ChannelId), u64),
        agents: Vec<AgentStatus>,
    ) -> Presence {
        let talk = &mut self.agents.talk;
        uploads(
            &talk.world.cloud,
            &mut talk.played[0],
            (at, revision),
            agents,
        )
    }
}
