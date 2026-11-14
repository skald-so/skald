//! Isolation of an agent to its space: as an agent, the CLI covers only the space of the agent's
//! folder. It shows that space's channels and members, lists no other space and refuses a command
//! that names one; a folder in no space gives the agent nothing. As an agent, it also refuses the
//! person's commands: adding, listing and removing rules, approving a `.skald.toml`, creating
//! spaces and channels; a refused command changes nothing, on the device or on the server.
//!
//! Against a `skald-server` with GitHub sign-in through the GitHub stand-in, on PostgreSQL, behind
//! the recording relay, with no limit on the spaces an account owns; a test plays the person on the
//! verification page in headless Chrome. The server's public address is `http://localhost:<port>`,
//! as in the tests of agent addresses. Each machine keeps its state in a directory of its own and
//! its folders in its home there. Ada's device is in three spaces: its personal space, named its
//! account's username, `clienta` and `clientb`, each but the personal one with a channel. Bob, a
//! member of `clientb` only, is in no other space of Ada's.
//!
//! The CLI, as these tests use it, beside the commands of the tests of a folder's binding, of
//! creating channels and of agent addresses:
//! - `skald --agent <name> spaces`: lists the space of the working directory, as `skald spaces`
//!   lists it, with its name and ID, and no other space; in a folder in no space, none;
//! - `skald --agent <name> channels <space>` and `skald --agent <name> members <space>`: as without
//!   `--agent` when `<space>` is the space of the working directory; for any other space name,
//!   another of the device's spaces or none of them, the run fails, and its output names no space
//!   but the agent's own, not even the one it was given, so a space of the device's is refused
//!   just as a space the device is not in;
//! - `skald --agent <name> folder <folder>`: never shows another space than the agent's own;
//! - `skald --agent <name> add-rule|rules|remove-rule|approve-binding|create-space|create-channel
//!   ...`: fails and changes nothing; the refusal of `rules` lists no rule.

// Each test target uses a part of the shared modules.
#![allow(dead_code)]

#[path = "../roster_revisions/world.rs"]
mod world;

#[path = "../sign_in/cloud.rs"]
mod cloud;
#[path = "../sign_in/machine.rs"]
mod machine;
#[path = "../sign_in/relay.rs"]
mod relay;

#[path = "../../../server/tests/common/browser.rs"]
mod browser;
#[path = "../common/mod.rs"]
mod common;
#[path = "../device_keys/keys.rs"]
mod keys;

use std::fs;
use std::path::{Path, PathBuf};
use std::time::Instant;

use cloud::Cloud;
use machine::Finished;
use skald_proto::{ChannelId, Name, Role, SpaceId};
use world::{
    ADMIN, CREATE_SPACE, MEMBERS, PERSONAL_REQUEST, Person, SPACES, World, assert_role, create,
    holds_word, next_by, plain_lines, run, store,
};

/// The option that names the agent.
const AGENT: &str = "--agent";

/// The commands of a space's channels and of a folder's binding.
const CHANNELS: &str = "channels";
const CREATE_CHANNEL: &str = "create-channel";
const ADD_RULE: &str = "add-rule";
const RULES: &str = "rules";
const REMOVE_RULE: &str = "remove-rule";
const APPROVE: &str = "approve-binding";
const FOLDER: &str = "folder";

/// The host of the server's public address, as in the tests of agent addresses.
const HOST: &str = "localhost";

/// The team spaces Ada's device is in, a space no device is in, and one an agent tries to create.
const CLIENT_A: &str = "clienta";
const CLIENT_B: &str = "clientb";
const INITECH: &str = "initech";
const GLOBEX: &str = "globex";

/// The requests that create a space, issue a channel ID and store a revision, as their lines start.
const SPACE_WRITES: &str = "POST /spaces";

/// A space of Ada's device: its ID, its name and the IDs of its channels.
struct Space {
    id: SpaceId,
    name: String,
    channels: Vec<ChannelId>,
}

impl Space {
    /// What would show of the space: its ID and its channels' IDs.
    fn traces(&self) -> Vec<String> {
        std::iter::once(self.id.to_string())
            .chain(self.channels.iter().map(ToString::to_string))
            .collect()
    }
}

/// Ada's first machine in its personal space, `clienta` with a channel and `clientb` with a
/// channel and Bob as a member.
struct Office {
    world: World,
    ada: Person,
    bob: Name,
    personal: Space,
    client_a: Space,
    client_b: Space,
}

impl Office {
    fn open(ids: [u64; 2], logins: [&str; 2]) -> Self {
        let world = World::on_host(HOST);
        let ada = world.sign_up(ids[0], logins[0], "ada-workstation");
        let bob = world.sign_up(ids[1], logins[1], "bob-laptop");
        let cloud = &world.cloud;
        create(cloud, &ada, CLIENT_A);
        let first = create(cloud, &ada, CLIENT_B);
        let with_bob = next_by(&first, &ada, |members| {
            members.push(bob.member(Role::Member));
        });
        store(cloud, &ada.token, &with_bob);
        for space in [CLIENT_A, CLIENT_B] {
            let created = run(&ada.machine, &[CREATE_CHANNEL, space]);
            assert!(created.success, "a channel in {space}: {created:?}");
        }
        let personal = ada.handle.to_string();
        Office {
            personal: space_of(cloud, &ada, &personal),
            client_a: space_of(cloud, &ada, CLIENT_A),
            client_b: space_of(cloud, &ada, CLIENT_B),
            bob: bob.handle,
            world,
            ada,
        }
    }

    /// The machine's folder `rel`, such as `work/clientA`, under its home; made if missing.
    fn folder(&self, rel: &str) -> PathBuf {
        let mut dir = self.ada.machine.root().join("home");
        dir.extend(rel.split('/'));
        fs::create_dir_all(&dir).expect("make a folder");
        dir
    }

    /// Runs `skald <args>` as the person in `dir`; fails unless it succeeds.
    fn person(&self, dir: &Path, args: &[&str]) -> Finished {
        let finished = self.ada.machine.start_in(dir, args).finish();
        assert!(finished.success, "{args:?}: {finished:?}");
        finished
    }

    /// Runs `skald --agent <agent> <args>` in `dir`.
    fn agent(&self, dir: &Path, agent: &str, args: &[&str]) -> Finished {
        let mut all = vec![AGENT, agent];
        all.extend_from_slice(args);
        self.ada.machine.start_in(dir, &all).finish()
    }

    /// As [`Office::agent`]; fails unless the run succeeds.
    fn agent_ok(&self, dir: &Path, agent: &str, args: &[&str]) -> Finished {
        let finished = self.agent(dir, agent, args);
        assert!(
            finished.success,
            "{agent} {args:?} in {}: {finished:?}",
            dir.display()
        );
        finished
    }

    /// As [`Office::agent`]; fails unless the run fails.
    fn agent_refused(&self, dir: &Path, agent: &str, args: &[&str]) -> Finished {
        let finished = self.agent(dir, agent, args);
        assert!(
            !finished.success,
            "{agent} {args:?} in {}: {finished:?}",
            dir.display()
        );
        finished
    }

    /// Binds `dir` and everything below it to `space` by a rule.
    fn add_rule(&self, dir: &Path, space: &str) {
        let home = self.folder("");
        self.person(&home, &[ADD_RULE, path(dir), space]);
    }

    /// The three spaces.
    fn spaces(&self) -> [&Space; 3] {
        [&self.personal, &self.client_a, &self.client_b]
    }

    /// Fails if `shown` shows anything of a space of the device but `own`: the ID of a space or
    /// of its channels, the name of a team space, or Bob's handle unless `own` is `clientb`.
    #[track_caller]
    fn assert_sees_no_other(&self, shown: &Finished, own: Option<&Space>, what: &str) {
        let text = shown.shown();
        let lines: Vec<String> = text.lines().map(str::to_owned).collect();
        for space in self.spaces() {
            if own.is_some_and(|own| own.id == space.id) {
                continue;
            }
            for trace in space.traces() {
                assert!(
                    !text.contains(&trace),
                    "{what} shows {trace} of {}: {shown:?}",
                    space.name
                );
            }
            if space.id != self.personal.id {
                assert!(
                    !holds_word(&lines, &space.name),
                    "{what} names {}: {shown:?}",
                    space.name
                );
            }
        }
        if own.is_none_or(|own| own.id != self.client_b.id) {
            assert!(
                !holds_word(&lines, self.bob.as_str()),
                "{what} shows Bob, of clientb: {shown:?}"
            );
        }
    }

    /// Fails unless an agent in `dir`, whose folder is in `own`, sees `own` and nothing of the
    /// device's other spaces: its space list shows `own` only; its channels and members show; and
    /// each command naming another space, or a space the device is not in, is refused alike,
    /// naming nothing of the other spaces.
    #[track_caller]
    fn assert_sees_only(&self, dir: &Path, agent: &str, own: &Space) {
        let listed = self.agent_ok(dir, agent, &[SPACES]);
        assert!(
            listed.shown().contains(&own.id.to_string())
                && holds_word(&plain_lines(&listed), &own.name),
            "{agent} does not list {}: {listed:?}",
            own.name
        );
        self.assert_sees_no_other(&listed, Some(own), "the space list");

        let channels = self.agent_ok(dir, agent, &[CHANNELS, &own.name]);
        for channel in &own.channels {
            assert!(
                channels.shown().contains(&channel.to_string()),
                "{agent} does not see the channel {channel} of {}: {channels:?}",
                own.name
            );
        }
        self.assert_sees_no_other(&channels, Some(own), "the channels");

        let members = self.agent_ok(dir, agent, &[MEMBERS, &own.name]);
        assert_role(&members, &self.ada.handle, ADMIN);
        self.assert_sees_no_other(&members, Some(own), "the members");

        for command in [CHANNELS, MEMBERS] {
            let unknown = self.agent_refused(dir, agent, &[command, GLOBEX]);
            self.assert_sees_no_other(&unknown, Some(own), "a refusal");
            for other in self.spaces() {
                if other.id == own.id {
                    continue;
                }
                let refused = self.agent_refused(dir, agent, &[command, &other.name]);
                self.assert_sees_no_other(&refused, Some(own), "a refusal");
                assert_eq!(
                    refused.shown(),
                    unknown.shown(),
                    "{command} {} is refused otherwise than a space the device is not in",
                    other.name
                );
            }
        }
    }
}

fn path(dir: &Path) -> &str {
    dir.to_str().expect("a UTF-8 path")
}

/// The space `name` of `person`'s account, as the server lists it, with the channels of its last
/// revision.
fn space_of(cloud: &Cloud, person: &Person, name: &str) -> Space {
    let (status, body) = cloud.get_as("/spaces", &person.token);
    assert_eq!(status, 200, "GET /spaces: {body}");
    let id: SpaceId = body["spaces"]
        .as_array()
        .and_then(|spaces| spaces.iter().find(|space| space["name"] == name))
        .and_then(|space| space["space"].as_str())
        .and_then(|id| id.parse().ok())
        .unwrap_or_else(|| panic!("the account has no space {name}: {body}"));
    let revisions = cloud.revisions(&person.token, id);
    let (_, last) = revisions.last().expect("a revision");
    Space {
        id,
        name: name.to_owned(),
        channels: last
            .body()
            .channels
            .iter()
            .map(|channel| channel.id)
            .collect(),
    }
}

/// The spaces of `person`'s account on the server, and the number of revisions of each.
fn server_spaces(cloud: &Cloud, person: &Person) -> Vec<(String, usize)> {
    let (status, body) = cloud.get_as("/spaces", &person.token);
    assert_eq!(status, 200, "GET /spaces: {body}");
    let mut spaces: Vec<(String, usize)> = body["spaces"]
        .as_array()
        .unwrap_or_else(|| panic!("GET /spaces: {body}"))
        .iter()
        .map(|space| {
            let id: SpaceId = space["space"]
                .as_str()
                .and_then(|id| id.parse().ok())
                .unwrap_or_else(|| panic!("a space ID: {space}"));
            (id.to_string(), cloud.revisions(&person.token, id).len())
        })
        .collect();
    spaces.sort();
    spaces
}

/// The person's space list shows all three spaces. An agent in `~/work/clientA` or below it sees
/// only clientA: its space list, clientA's channels and members, and a refusal, alike for each, of
/// the channels and members of clientb, of the personal space and of a space the device is not in.
/// Its `folder` of another folder shows no other space. An agent in `~/work/clientB` sees only
/// clientb, and one in a folder without a binding only the personal space.
#[test]
fn agent_sees_only_the_space_of_its_folder() {
    let office = Office::open(
        [4_121_001_001, 4_121_001_002],
        ["ada-isolated-standin", "bob-isolated-standin"],
    );
    let a = office.folder("work/clientA");
    let b = office.folder("work/clientB");
    office.add_rule(&a, CLIENT_A);
    office.add_rule(&b, CLIENT_B);
    let notes = office.folder("notes");

    let listed = office.person(&notes, &[SPACES]);
    for space in office.spaces() {
        assert!(
            listed.shown().contains(&space.id.to_string()),
            "the person's list misses {}: {listed:?}",
            space.name
        );
    }

    let claude = "claude-k3s8vq";
    office.assert_sees_only(&a, claude, &office.client_a);
    let src = office.folder("work/clientA/app/src");
    office.assert_sees_only(&src, "codex-r2m7tx", &office.client_a);
    for other in [&b, &notes] {
        let shown = office.agent(&a, claude, &[FOLDER, path(other)]);
        office.assert_sees_no_other(&shown, Some(&office.client_a), "another folder");
    }

    office.assert_sees_only(&b, claude, &office.client_b);
    office.assert_sees_only(&notes, claude, &office.personal);
}

/// An agent in a folder in no space, one a rule binds to a space the device is not in, sees
/// nothing: its space list shows none of the device's spaces, and the channels and members of each
/// are refused. The same agent in `~/work/clientA` sees clientA.
#[test]
fn agent_in_a_folder_in_no_space_sees_nothing() {
    let office = Office::open(
        [4_121_001_101, 4_121_001_102],
        ["ada-nowhere-standin", "bob-nowhere-standin"],
    );
    let a = office.folder("work/clientA");
    office.add_rule(&a, CLIENT_A);
    let initech = office.folder("work/initech");
    office.add_rule(&initech, INITECH);
    let agent = "claude-n0wh3r";
    office.assert_sees_only(&a, agent, &office.client_a);

    let listed = office.agent(&initech, agent, &[SPACES]);
    office.assert_sees_no_other(&listed, None, "the space list");
    for command in [CHANNELS, MEMBERS] {
        for space in office.spaces() {
            let refused = office.agent_refused(&initech, agent, &[command, &space.name]);
            office.assert_sees_no_other(&refused, None, "a refusal");
        }
    }
}

/// As an agent, each of the person's commands is refused and changes nothing: adding a rule that
/// would move a subfolder of clientA to clientb, listing and removing rules, approving a
/// `.skald.toml` of the agent's own folder, creating a space and a channel, while the same agent's
/// commands of its own space work. The rules, the folders' spaces, the device's spaces and their
/// channels stay as they were, and the server gets no new space, channel or revision.
#[test]
fn person_commands_are_refused_to_an_agent_and_change_nothing() {
    let office = Office::open(
        [4_121_001_201, 4_121_001_202],
        ["ada-person-standin", "bob-person-standin"],
    );
    let cloud = &office.world.cloud;
    let a = office.folder("work/clientA");
    let b = office.folder("work/clientB");
    office.add_rule(&a, CLIENT_A);
    office.add_rule(&b, CLIENT_B);
    let deeper = office.folder("work/clientA/vendor");
    let repo = office.folder("work/clientA/repo");
    fs::write(
        repo.join(".skald.toml"),
        format!("space = \"{CLIENT_B}\"\n"),
    )
    .expect("write a .skald.toml");
    let home = office.folder("");

    // What the person sees, as one snapshot.
    let snapshot = || {
        let shown: Vec<String> = [
            office.person(&home, &[RULES]),
            office.person(&home, &[FOLDER, path(&a)]),
            office.person(&home, &[FOLDER, path(&deeper)]),
            office.person(&home, &[FOLDER, path(&repo)]),
            office.person(&home, &[SPACES]),
            office.person(&home, &[CHANNELS, CLIENT_A]),
            office.person(&home, &[CHANNELS, CLIENT_B]),
        ]
        .iter()
        .map(Finished::shown)
        .collect();
        (shown, server_spaces(cloud, &office.ada))
    };
    let agent = "claude-p3rs0n";
    office.assert_sees_only(&a, agent, &office.client_a);
    let before = snapshot();
    let since = Instant::now();

    office.agent_refused(&a, agent, &[ADD_RULE, path(&deeper), CLIENT_B]);
    let rules = office.agent_refused(&a, agent, &[RULES]);
    office.assert_sees_no_other(&rules, Some(&office.client_a), "the refusal of rules");
    office.agent_refused(&a, agent, &[REMOVE_RULE, path(&a)]);
    office.agent_refused(&repo, agent, &[APPROVE, path(&repo)]);
    office.agent_refused(&a, agent, &[CREATE_SPACE, GLOBEX]);
    office.agent_refused(&a, agent, &[CREATE_CHANNEL, CLIENT_A]);

    assert_eq!(
        cloud.count_sent(since, SPACE_WRITES),
        0,
        "an agent's command wrote to the server"
    );
    let after = snapshot();
    assert_eq!(after, before, "an agent's command changed something");
    assert_eq!(
        cloud.count_sent(since, SPACE_WRITES),
        0,
        "an agent's command left a write for the person's next run"
    );
}

/// Each of the person's commands refused to an agent says that it is the person's command, naming
/// it, and that an agent cannot run it.
#[test]
fn refusal_of_a_persons_command_names_it_as_the_persons() {
    let office = Office::open(
        [4_121_001_301, 4_121_001_302],
        ["ada-refusal-standin", "bob-refusal-standin"],
    );
    let a = office.folder("work/clientA");
    office.add_rule(&a, CLIENT_A);
    let agent = "claude-r3fus3";
    let commands: [&[&str]; 6] = [
        &[ADD_RULE, path(&a), CLIENT_B],
        &[RULES],
        &[REMOVE_RULE, path(&a)],
        &[APPROVE, path(&a)],
        &[CREATE_SPACE, GLOBEX],
        &[CREATE_CHANNEL, CLIENT_A],
    ];
    for args in commands {
        let refused = office.agent_refused(&a, agent, args);
        let said = refused.stderr.to_lowercase();
        assert!(
            said.contains(&format!("skald {}", args[0]))
                && said.contains("person's command")
                && said.contains("agent cannot run it"),
            "{args:?}: {refused:?}"
        );
    }
}

/// An agent in `~/work/clientA` is shown another folder of clientA, such as one deeper or bound by
/// the same rule, with clientA's name and ID.
#[test]
fn agent_sees_another_folder_of_its_space() {
    let office = Office::open(
        [4_121_001_401, 4_121_001_402],
        ["ada-folders-standin", "bob-folders-standin"],
    );
    let a = office.folder("work/clientA");
    office.add_rule(&a, CLIENT_A);
    let src = office.folder("work/clientA/app/src");
    let agent = "claude-f0ld3r";
    for (dir, other) in [(&a, &src), (&src, &a)] {
        let shown = office.agent_ok(dir, agent, &[FOLDER, path(other)]);
        let own = &office.client_a;
        assert!(
            shown.stdout.contains(&own.id.to_string())
                && holds_word(&plain_lines(&shown), &own.name),
            "{} from {}: {shown:?}",
            other.display(),
            dir.display()
        );
        office.assert_sees_no_other(&shown, Some(own), "a folder of the agent's space");
    }
}

/// On a machine whose account has no personal space yet, as the server refused it at sign-in, an
/// agent's runs in `~/work/clientA`, its sign-in among them, neither ask for the personal space nor
/// write to the server; the person's next run then sets it up.
#[test]
fn agent_runs_do_not_take_the_personal_space() {
    let world = World::on_host(HOST);
    let cloud = &world.cloud;
    cloud.relay.refuse(PERSONAL_REQUEST, "404 Not Found");
    let user = cloud.user(4_121_001_501, "ada-untaken-standin");
    let host = "ada-workstation";
    // Its result does not matter.
    let (machine, _) = cloud.first_machine(&world.browser.page(), &user, host);
    let ada = world.person(user, machine, host);
    for args in [[CREATE_SPACE, CLIENT_A], [CREATE_CHANNEL, CLIENT_A]] {
        let done = run(&ada.machine, &args);
        assert!(done.success, "{args:?}: {done:?}");
    }
    cloud.relay.stop_refusing();
    let mut a = ada.machine.root().join("home");
    a.extend(["work", "clientA"]);
    fs::create_dir_all(&a).expect("make a folder");
    let added = run(&ada.machine, &[ADD_RULE, path(&a), CLIENT_A]);
    assert!(added.success, "{added:?}");
    let before = server_spaces(cloud, &ada);
    assert_eq!(before.len(), 1, "only clienta: {before:?}");
    let since = Instant::now();

    let agent = "claude-unt4k3";
    let runs: [&[&str]; 6] = [
        &[SPACES],
        &[CHANNELS, CLIENT_A],
        &[MEMBERS, CLIENT_A],
        &["address"],
        &[FOLDER],
        &["sign-in", "--server", &cloud.url],
    ];
    for args in runs {
        let mut all = vec![AGENT, agent];
        all.extend_from_slice(args);
        let finished = ada.machine.start_in(&a, &all).finish();
        assert!(finished.success, "{args:?}: {finished:?}");
    }

    assert_eq!(
        cloud.count_sent(since, PERSONAL_REQUEST),
        0,
        "an agent's run asked for the personal space"
    );
    assert_eq!(
        cloud.count_sent(since, SPACE_WRITES),
        0,
        "an agent's run wrote to the server"
    );
    assert_eq!(server_spaces(cloud, &ada), before);

    let listed = run(&ada.machine, &[SPACES]);
    assert!(listed.success, "{listed:?}");
    assert_eq!(
        server_spaces(cloud, &ada).len(),
        2,
        "the person's run sets up the personal space: {listed:?}"
    );
}
