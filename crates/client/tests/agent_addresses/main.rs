//! Agent addresses: an agent is a named endpoint on a device, with the address
//! `space/owner/agent@server` (`docs/protocol/README.md`, "Agent address") in the space of its
//! folder: that space's name, the account's handle in its last accepted roster revision, the
//! agent's name and the server's name. An agent name that is not a valid `name`, or is `user`, is
//! refused. Without a valid server name, such as with an IP address as the host of the server's
//! public address, an agent gets no address, and the CLI says so. The agent's name never reaches
//! the server (`docs/threat-model.md`, "The server does not see").
//!
//! Against a `skald-server` with GitHub sign-in through the GitHub stand-in, on PostgreSQL, behind
//! the recording relay; a test plays the person on the verification page in headless Chrome. The
//! server runs with the settings of the tests of a folder's binding, its public address
//! `http://localhost:<port>`; with no server name of its own, its name is that address's host,
//! `localhost`. Each machine keeps its state in a directory of its own and its folders in its home
//! there. The device is in three spaces: its personal space, named its account's username,
//! `clienta` and `clientb`.
//!
//! The CLI, as these tests use it, beside the commands of the tests of a folder's binding and of
//! `skald set-username <username>`:
//! - `skald --agent <name> <command>`: runs `<command>` as the agent `<name>`; `--agent=<name>` is
//!   the same;
//! - `skald --agent <name> address`: shows the address of the agent `<name>` in the space of the
//!   working directory, as the only agent address in its output. A refused agent name fails the
//!   run and shows no address. When the agent gets no address, the run shows none; without a valid
//!   server name, a line says `no address`.

// Each test target uses a part of the shared modules.
#![allow(dead_code)]

#[path = "../sign_in/machine.rs"]
mod machine;
#[path = "../roster_revisions/world.rs"]
mod world;

#[path = "../../../server/tests/common/browser.rs"]
mod browser;
#[path = "../sign_in/relay.rs"]
mod relay;

#[path = "../sign_in/cloud.rs"]
mod cloud;
#[path = "../common/mod.rs"]
mod common;
#[path = "../device_keys/keys.rs"]
mod keys;

use std::fs;
use std::path::{Path, PathBuf};

use base64::Engine as _;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use machine::{Finished, Machine};
use serde_json::json;
use skald_crypto::SigningKey;
use skald_proto::{Address, Cbor, Name, Roster};
use world::{
    Person, SPACES, World, create, next, next_by, revisions_request, run, store, warnings,
};

/// The option that names the agent, and the command that shows its address.
const AGENT: &str = "--agent";
const ADDRESS: &str = "address";

/// The commands that add a rule, approve a `.skald.toml` and set the account's username.
const ADD_RULE: &str = "add-rule";
const APPROVE: &str = "approve-binding";
const SET_USERNAME: &str = "set-username";

/// The server's name: the host of its public address.
const SERVER: &str = "localhost";

/// The team spaces the device is in.
const CLIENT_A: &str = "clienta";
const CLIENT_B: &str = "clientb";

/// What the CLI says when an agent gets no address for want of a valid server name.
const NO_ADDRESS: &str = "no address";

/// A person's first machine, in its personal space, `clienta` and `clientb`.
struct Desk {
    world: World,
    ada: Person,
    /// The name of the account's personal space: its username.
    personal: String,
    /// Revision 1 of `clienta`.
    client_a: Roster,
}

impl Desk {
    /// The desk on a server whose public address is under `host`.
    fn on(host: &str, id: u64, login: &str) -> Self {
        let world = World::on_host(host);
        let ada = world.sign_up(id, login, "ada-workstation");
        let client_a = create(&world.cloud, &ada, CLIENT_A);
        create(&world.cloud, &ada, CLIENT_B);
        Desk {
            personal: ada.handle.to_string(),
            world,
            ada,
            client_a,
        }
    }

    fn start(id: u64, login: &str) -> Self {
        Self::on(SERVER, id, login)
    }

    /// The machine's folder `rel`, such as `work/clientA`, under its home; made if missing.
    fn dir(&self, rel: &str) -> PathBuf {
        let dir = self
            .ada
            .machine
            .root()
            .join("home")
            .join(rel.split('/').collect::<PathBuf>());
        fs::create_dir_all(&dir).expect("make a folder");
        dir
    }

    /// Runs `skald <args>` on the machine; fails unless it succeeds.
    fn ok(&self, args: &[&str]) -> Finished {
        let finished = run(&self.ada.machine, args);
        assert!(finished.success, "{args:?}: {finished:?}");
        finished
    }

    /// Binds `dir` and everything below it to `space` by a rule.
    fn add_rule(&self, dir: &Path, space: &str) {
        self.ok(&[ADD_RULE, path(dir), space]);
    }

    /// Runs `skald --agent=<agent> address` on the machine in `dir`.
    fn address(&self, dir: &Path, agent: &str) -> Finished {
        address_in(&self.ada.machine, dir, agent)
    }

    /// Fails unless `skald --agent <agent> address` in `dir` succeeds and shows the address
    /// `<space>/<owner>/<agent>@localhost`, and no other.
    fn assert_address(&self, dir: &Path, agent: &str, space: &str, owner: &str) {
        let shown = self.address(dir, agent);
        assert!(shown.success, "{agent} in {}: {shown:?}", dir.display());
        let expected = format!("{space}/{owner}/{agent}@{SERVER}");
        let shown_addresses: Vec<String> =
            addresses(&shown).iter().map(ToString::to_string).collect();
        assert_eq!(
            shown_addresses,
            vec![expected],
            "{agent} in {}: {shown:?}",
            dir.display()
        );
    }

    /// Fails unless the machine's runs sent the server none of `agents`.
    fn assert_never_sent(&self, agents: &[&str]) {
        assert_never_sent(&self.world.cloud.relay.sent_bytes(), agents);
    }
}

fn path(dir: &Path) -> &str {
    dir.to_str().expect("a UTF-8 path")
}

/// Runs `skald --agent=<agent> address` on `machine` in the working directory `dir`.
fn address_in(machine: &Machine, dir: &Path, agent: &str) -> Finished {
    let agent = format!("{AGENT}={agent}");
    machine.start_in(dir, &[&agent, ADDRESS]).finish()
}

/// The agent addresses `shown` shows, on either output: each word between spaces, quotes or
/// brackets that is a valid address, with the punctuation around it trimmed.
fn addresses(shown: &Finished) -> Vec<Address> {
    shown
        .shown()
        .split(|char: char| char.is_whitespace() || "\"'`<>()[]{},;".contains(char))
        .map(|word| word.trim_matches(|char: char| ".:!?".contains(char)))
        .filter_map(|word| word.parse().ok())
        .collect()
}

/// Fails if `shown` shows an agent address.
#[track_caller]
fn assert_no_address(shown: &Finished, what: &str) {
    let shown_addresses = addresses(shown);
    assert!(
        shown_addresses.is_empty(),
        "{what}: {shown_addresses:?}: {shown:?}"
    );
}

/// Fails if `sent`, every byte the machines sent, holds one of `agents`, as text or in hex.
#[track_caller]
fn assert_never_sent(sent: &[u8], agents: &[&str]) {
    let sent_text = String::from_utf8_lossy(sent).to_lowercase();
    for agent in agents {
        let hex: String = agent.bytes().map(|byte| format!("{byte:02x}")).collect();
        for form in [agent.to_string(), hex] {
            assert!(
                !sent_text.contains(&form),
                "the server got the agent name {agent} as {form:?}"
            );
        }
    }
}

/// An agent in `~/work/clientA` and its subfolders, bound by a rule, gets its address in
/// `clienta`; in `~/work/clientB`, in `clientb`; in a folder an approved `.skald.toml` binds, in
/// that space; in a folder without a binding, in the personal space; each with the account's
/// handle, the agent's name and the server's name. Another agent in the same folder gets its own
/// address. A folder bound to a space the device is not in gives no address. No request carries
/// an agent's name, also of the runs of other commands as an agent.
#[test]
fn agent_gets_the_address_of_its_folders_space() {
    let desk = Desk::start(4_120_901_001, "ada-address-standin");
    let handle = desk.ada.handle.to_string();
    let claude = "claude-q7v3kd";
    let codex = "codex-z8m2wp";

    let a = desk.dir("work/clientA");
    let b = desk.dir("work/clientB");
    desk.add_rule(&a, CLIENT_A);
    desk.add_rule(&b, CLIENT_B);

    desk.assert_address(&a, claude, CLIENT_A, &handle);
    let src = desk.dir("work/clientA/app/src");
    desk.assert_address(&src, claude, CLIENT_A, &handle);
    desk.assert_address(&src, codex, CLIENT_A, &handle);
    desk.assert_address(&b, claude, CLIENT_B, &handle);

    let repo = desk.dir("work/repo");
    fs::write(
        repo.join(".skald.toml"),
        format!("space = \"{CLIENT_B}\"\n"),
    )
    .expect("write a .skald.toml");
    desk.ok(&[APPROVE, path(&repo)]);
    desk.assert_address(&repo, codex, CLIENT_B, &handle);

    let notes = desk.dir("notes");
    desk.assert_address(&notes, claude, &desk.personal, &handle);
    desk.assert_address(&notes, codex, &desk.personal, &handle);

    let initech = desk.dir("work/initech");
    desk.add_rule(&initech, "initech");
    let shown = desk.address(&initech, claude);
    assert_no_address(&shown, "a folder in no space");

    for command in [SPACES, "folder"] {
        let agent = format!("{AGENT}={claude}");
        let finished = desk.ada.machine.start_in(&a, &[&agent, command]).finish();
        assert!(finished.success, "{command} as {claude}: {finished:?}");
        let finished = desk
            .ada
            .machine
            .start_in(&a, &[AGENT, codex, command])
            .finish();
        assert!(finished.success, "{command} as {codex}: {finished:?}");
    }

    desk.assert_never_sent(&[claude, codex]);
}

/// The owner in an address is the account's handle in the space's last accepted revision: it
/// stays when the account's username changes, follows a revision an admin's device signed that
/// changes it, and stays when a revision that fails the checks would change it again.
#[test]
fn owner_is_the_handle_in_the_last_accepted_revision() {
    let desk = Desk::start(4_120_901_101, "ada-handle-standin");
    let cloud = &desk.world.cloud;
    let handle = desk.ada.handle.to_string();
    let agent = "claude-h4n9le";
    let a = desk.dir("work/clientA");
    desk.add_rule(&a, CLIENT_A);
    let notes = desk.dir("notes");

    desk.ok(&[SET_USERNAME, "ada-renamed"]);
    desk.assert_address(&a, agent, CLIENT_A, &handle);
    desk.assert_address(&notes, agent, &desk.personal, &handle);

    let renamed: Name = "ada-at-clienta".parse().expect("a valid handle");
    let second = next_by(&desk.client_a, &desk.ada, |members| {
        members[0].handle = renamed.clone();
    });
    store(cloud, &desk.ada.token, &second);
    desk.ok(&[SPACES]);
    desk.assert_address(&a, agent, CLIENT_A, renamed.as_str());

    let forged = next(
        &second,
        (desk.ada.device, &SigningKey::generate()),
        |members| {
            members[0].handle = "mallory".parse().expect("a valid handle");
        },
    );
    let list: Vec<String> = [&desk.client_a, &second, &forged]
        .iter()
        .map(|roster| URL_SAFE_NO_PAD.encode(roster.to_cbor()))
        .collect();
    let space = desk.client_a.body().space;
    cloud.relay.rewrite(&revisions_request(space), move |_| {
        json!({ "revisions": list }).to_string()
    });
    let listed = desk.ok(&[SPACES]);
    assert!(!warnings(&listed).is_empty(), "no warning: {listed:?}");
    desk.assert_address(&a, agent, CLIENT_A, renamed.as_str());
    desk.assert_address(&notes, agent, &desk.personal, &handle);

    desk.assert_never_sent(&[agent]);
}

/// An agent name that is not a valid `name`, or is `user`, is refused, and the run shows no
/// address; one of 63 characters, the longest, gets its address in the same folder.
#[test]
fn invalid_agent_names_and_user_are_refused() {
    let desk = Desk::start(4_120_901_201, "ada-names-standin");
    let handle = desk.ada.handle.to_string();
    let a = desk.dir("work/clientA");
    desk.add_rule(&a, CLIENT_A);

    let longest = format!("agent{}", "x".repeat(58));
    assert_eq!(longest.len(), 63);
    desk.assert_address(&a, &longest, CLIENT_A, &handle);
    desk.assert_address(&a, "0", CLIENT_A, &handle);

    let too_long = format!("{longest}x");
    let refused = [
        "",
        "user",
        "claude_api",
        "claude.api",
        "claude api",
        "claude/api",
        "claude@api",
        "-claude",
        "claude-",
        "clàude",
        too_long.as_str(),
    ];
    for agent in refused {
        let shown = desk.address(&a, agent);
        assert!(!shown.success, "{agent:?} is taken: {shown:?}");
        assert_no_address(&shown, &format!("{agent:?}"));
    }
}

/// With an IP address as the host of the server's public address, the server has no name: an agent
/// gets no address in its folder's space, and the CLI says so. Its name still never reaches the
/// server.
#[test]
fn without_a_valid_server_name_an_agent_gets_no_address() {
    let desk = Desk::on("127.0.0.1", 4_120_901_301, "ada-unnamed-standin");
    let agent = "claude-n0n4me";
    let a = desk.dir("work/clientA");
    desk.add_rule(&a, CLIENT_A);

    for dir in [a, desk.dir("notes")] {
        let shown = desk.address(&dir, agent);
        assert_no_address(&shown, "a server without a name");
        assert!(
            shown.shown().to_lowercase().contains(NO_ADDRESS),
            "the CLI does not say the agent has no address: {shown:?}"
        );
    }

    desk.assert_never_sent(&[agent]);
}

/// The address follows a revision an admin's device stored that changes the owner's handle, with
/// no other command run before it.
#[test]
fn address_accepts_the_revisions_the_server_has() {
    let desk = Desk::start(4_120_901_401, "ada-synced-standin");
    let agent = "claude-5ync3d";
    let a = desk.dir("work/clientA");
    desk.add_rule(&a, CLIENT_A);
    desk.assert_address(&a, agent, CLIENT_A, desk.ada.handle.as_str());

    let renamed: Name = "ada-synced-at-clienta".parse().expect("a valid handle");
    let second = next_by(&desk.client_a, &desk.ada, |members| {
        members[0].handle = renamed.clone();
    });
    store(&desk.world.cloud, &desk.ada.token, &second);
    desk.assert_address(&a, agent, CLIENT_A, renamed.as_str());

    desk.assert_never_sent(&[agent]);
}
