//! Key changes until the person sees them (`docs/protocol/membership.md`, "Device checks";
//! `docs/threat-model.md`, "An agent on the person's own device").
//!
//! - A device keeps each key change it warns of in its local state until its person sees it. Key
//!   changes: a member's `master` changes, also a verified one's; an account has two master keys,
//!   or a device's certificate is swapped or has two bodies; an account record's `recovery` changes
//!   or a record fails its check; a KeyPackage the server hands out fails its checks; an `ssh`
//!   signature fails; a domain lists another fingerprint or misses an admin.
//! - Until its person has seen them, every command, as the person or as an agent, through the
//!   helper or one-shot, prints one line: how many key changes wait and the command that shows
//!   them, run at a terminal. A waiting `skald recv --wait` shows it as a warning, `--json` has it
//!   as a field, and the MCP server's tool results say it too. As an agent, the line names nothing
//!   of other spaces.
//! - That command shows them only as the person with its output a terminal, then counts them as
//!   seen; from a pipe or as an agent it refuses and leaves them. A run killed after showing them
//!   and before counting them seen leaves them waiting.
//!
//! Each key change is caused as the tests of its own iteration cause it, against a `skald-server`
//! with GitHub sign-in through the GitHub stand-in, behind the recording relay, with fixture
//! revisions the tests sign with the devices' keys (`roster_revisions/main.rs`): the master keys as
//! the tests of roster revisions, account fingerprints and swapped keys do
//! (`account_fingerprints/main.rs`, `swapped_keys/main.rs`); the certificates as the tests of
//! swapped keys; the account records as the tests of the history through the recovery key
//! (`recovery_history/main.rs`); the KeyPackages as the tests of channel groups
//! (`channel_groups/main.rs`); the SSH signatures as the tests of checking them against GitHub
//! (`github_checks/main.rs`), with the server's GitHub stand-in as GitHub's API; the domains as the
//! tests of the DNS root of trust (`dns_root_of_trust/main.rs`), with their DNS-over-HTTPS
//! stand-in. The lines everywhere, as the tests of the link prompt (`link_prompt/main.rs`): Ada's
//! machine runs the CLI, her agents run in `~/work/acme`, bound to `acme`, and Bob's device is
//! played by the tests; there the key change is a space `globex` of Ada's whose revision lists
//! Bob with another master key than `acme` does. A person at a terminal is the pseudo-terminal of
//! the tests of the recovery key (`recovery_key/terminal.rs`).
//!
//! What the tests chose, beside what the spec names:
//! - [`KEY_CHANGES`], `skald key-changes`, the command that shows the waiting key changes;
//! - the line: one line, on either output, holding `skald key-changes` and, as a word, the number
//!   of key changes that wait; no other line a command prints holds `skald key-changes`;
//! - with `--json`, standard output holds only JSON objects, and one of them holds among its values
//!   the number of key changes that wait and the text `skald key-changes`;
//! - `skald key-changes` as the person at a terminal: shows each waiting key change, naming the
//!   member's handle or account ID, or the device's ID, its warning names; then asks the person to
//!   press Enter, on a line holding [`ENTER`], and only on Enter counts them seen and exits with
//!   success. Killed while it asks, it counts nothing seen. With none waiting, it says so and exits
//!   with success without asking. From a pipe, or with `--agent <name>`, also at a terminal, it
//!   fails, shows none of them and counts nothing seen.

// Each test target uses a part of the shared modules.
#![allow(dead_code)]

mod certificates;
mod domains;
mod everywhere;
mod key_packages;
mod masters;
mod other_spaces;
mod records;
mod refusals;
mod ssh;

// The modules the tests of the channel history share (`history_common/mod.rs`).
#[path = "../history_common/mod.rs"]
mod history_common;
use history_common::*;

// What the tests share with other test targets: Ada's agents, as the tests of handing messages
// over have them; the DNS-over-HTTPS stand-in of the tests of the DNS root of trust; the messages of
// the tests of gap detection; the runs of a machine's own copy of `skald`, as the tests of the
// helper have them; what an agent reads, as the tests of compact output have it; the agent CLI of
// the tests of the MCP server; the terminal of the tests of the recovery key; and the waits of the
// tests of `skald recv --wait`.
#[path = "../agent_handover/agents.rs"]
mod agents;
#[path = "../dns_root_of_trust/doh.rs"]
mod doh;
#[path = "../gap_detection/gaps.rs"]
mod gaps;
#[path = "../helper/helped.rs"]
mod helped;
#[path = "../compact_output/lines.rs"]
mod lines;
#[path = "../mcp_inbox/client.rs"]
mod mcp_client;
#[path = "../recovery_key/terminal.rs"]
mod terminal;
#[path = "../recv_wait/waits.rs"]
mod waits;

use std::thread;
use std::time::{Duration, Instant};

use skald_crypto::SigningKey;
use skald_proto::{Member, Role};

use agents::Agents;
use kit::{create_space, latest};
use machine::{Finished, Machine, RUN_LIMIT};
use terminal::{Shown, Terminal};
use world::{
    ACME, GLOBEX, MEMBERS, Person, World, create_with_member, members, next_by, run, store, words,
};

/// The command that shows the key changes that wait.
pub const KEY_CHANGES: &str = "key-changes";

/// What the line holds: the command, as the person runs it.
pub const SHOWN_BY: &str = "skald key-changes";

/// What the question to press Enter holds.
pub const ENTER: &str = "Enter";

/// The host names of the people's machines, their certificates' `name`.
pub const ADA_HOST: &str = "ada-workstation";
pub const BOB_HOST: &str = "bob-workstation";
pub const CAROL_HOST: &str = "carol-laptop";

/// The lines of `shown` that hold [`SHOWN_BY`].
pub fn key_change_lines(shown: &str) -> Vec<String> {
    (shown.lines())
        .filter(|line| line.contains(SHOWN_BY))
        .map(str::to_owned)
        .collect()
}

/// The numbers `line` holds as words.
fn numbers_in(line: &str) -> Vec<u64> {
    (words(line).into_iter())
        .filter_map(|word| word.parse().ok())
        .collect()
}

/// Fails unless one line of `shown` holds [`SHOWN_BY`] and, as a word, a number of key changes
/// above zero; `count` if given. Gives that line.
#[track_caller]
pub fn assert_says_waiting(shown: &str, count: Option<u64>) -> String {
    let found = key_change_lines(shown);
    let [line] = found.as_slice() else {
        panic!("not one line of the key changes that wait: {shown}");
    };
    let numbers = numbers_in(line);
    match count {
        Some(count) => assert!(
            numbers.contains(&count),
            "the line does not say {count} key changes wait: {line:?}"
        ),
        None => assert!(
            numbers.iter().any(|number| *number > 0),
            "the line says no number of key changes: {line:?}"
        ),
    }
    line.clone()
}

/// Fails if a line of `shown` holds [`SHOWN_BY`].
#[track_caller]
pub fn assert_says_none(shown: &str) {
    let found = key_change_lines(shown);
    assert!(found.is_empty(), "key changes said to wait: {shown}");
}

/// Runs `skald <args>` on `machine`; fails unless it succeeds and prints the line of
/// [`assert_says_waiting`]. Gives the line.
#[track_caller]
pub fn assert_run_says_waiting(machine: &Machine, args: &[&str]) -> String {
    let finished = run(machine, args);
    assert!(finished.success, "{args:?}: {finished:?}");
    assert_says_waiting(&finished.shown(), None)
}

/// Runs `skald <args>` on `machine`; fails unless it succeeds and prints no line of the key
/// changes.
#[track_caller]
pub fn assert_run_says_none(machine: &Machine, args: &[&str]) -> Finished {
    let finished = run(machine, args);
    assert!(finished.success, "{args:?}: {finished:?}");
    assert_says_none(&finished.shown());
    finished
}

/// `skald key-changes` at a terminal on `machine`, as its person: waits until it asks to press
/// Enter, then gives the run, still asking.
#[track_caller]
pub fn asking(machine: &Machine) -> Terminal {
    let mut terminal = Terminal::start(machine, &[KEY_CHANGES]);
    terminal.wait_to_show(ENTER);
    terminal
}

/// The person on `machine` runs `skald key-changes` at a terminal and presses Enter once it asks;
/// fails unless it succeeds. Gives what it showed.
#[track_caller]
pub fn seen_at_terminal(machine: &Machine) -> Shown {
    let terminal = asking(machine);
    terminal.enter();
    let shown = terminal.finish();
    assert!(shown.success, "{}", shown.text);
    shown
}

/// Whether `text` holds one of `names`, case aside.
pub fn names_one_of(text: &str, names: &[String]) -> bool {
    let lower = text.to_lowercase();
    names
        .iter()
        .any(|name| lower.contains(&name.to_lowercase()))
}

/// `skald key-changes` at a terminal on `machine` with nothing waiting: fails unless it exits with
/// success without asking to press Enter and names none of `names`.
#[track_caller]
pub fn assert_nothing_to_show(machine: &Machine, names: &[String]) {
    let mut terminal = Terminal::start(machine, &[KEY_CHANGES]);
    let started = Instant::now();
    while !terminal.exited() {
        let shown = terminal.shown();
        assert!(
            !shown.contains(ENTER),
            "asked with nothing waiting: {shown}"
        );
        assert!(started.elapsed() < RUN_LIMIT, "never exited: {shown}");
        thread::sleep(Duration::from_millis(50));
    }
    let shown = terminal.finish();
    assert!(shown.success, "{}", shown.text);
    assert!(
        !names_one_of(&shown.text, names),
        "a key change shown again: {}",
        shown.text
    );
}

/// The key change on `machine` that `names` identify waits and shows once: `skald <later>` prints
/// the line; `skald key-changes` at a terminal shows it, naming one of `names`, and, once the
/// person presses Enter, counts it seen. Then `skald <later>` and `skald spaces` print no line, and
/// `skald key-changes` has nothing to show. Gives what the terminal showed.
#[track_caller]
pub fn assert_waits_and_shows_once(machine: &Machine, later: &[&str], names: &[String]) -> Shown {
    assert_run_says_waiting(machine, later);

    let shown = seen_at_terminal(machine);

    assert!(
        names_one_of(&shown.text, names),
        "none of {names:?} shown: {}",
        shown.text
    );
    assert_run_says_none(machine, later);
    assert_run_says_none(machine, &[world::SPACES]);
    assert_nothing_to_show(machine, names);
    shown
}

/// A new account of GitHub user `id` with `login` in `world`, whose first machine, on the host
/// `host`, runs with `setting` from its sign-in on.
pub fn signed_up_with(
    world: &World,
    (id, login): (u64, &str),
    host: &'static str,
    (name, value): (&str, &str),
) -> Person {
    let cloud = &world.cloud;
    let user = cloud.user(id, login);
    let machine = Machine::new().with_env(name, value).with_host_name(host);
    let ran = cloud.signed_in(&world.browser.page(), &machine, &user, &[]);
    assert!(ran.success, "{login} did not sign in: {ran:?}");
    world.person(user, machine, host)
}

/// `person`'s entry in a roster, with `role` and the master key `master`.
pub fn member_with_master(person: &Person, role: Role, master: [u8; 32]) -> Member {
    let mut member = person.member(role);
    member.master = master;
    member
}

/// A master key of no account.
pub fn other_master() -> [u8; 32] {
    SigningKey::generate().public_key()
}

/// Ada and Bob, each with a first machine, and `acme`, which Bob's machine created, at revision
/// 3, which adds Ada's account as a member and which his machine accepted with no line of key
/// changes.
pub struct Pair {
    pub world: World,
    pub ada: Person,
    pub bob: Person,
    pub third: skald_proto::Roster,
}

impl Pair {
    /// Ada is GitHub user `ids[0]` with `logins[0]`, Bob the next.
    pub fn new(ids: [u64; 2], logins: [&str; 2]) -> Self {
        let world = World::start();
        let mut people = (ids.into_iter().zip(logins))
            .zip([ADA_HOST, BOB_HOST])
            .map(|((id, login), host)| world.sign_up(id, login, host));
        let (ada, bob) = (people.next().expect("Ada"), people.next().expect("Bob"));
        drop(people);
        let third = create_with_member(&world.cloud, &bob, ACME, &ada);
        assert_says_none(&members(&bob.machine, ACME).shown());
        Pair {
            world,
            ada,
            bob,
            third,
        }
    }

    /// Ada's handle and account ID, as a key change of hers may name her.
    pub fn adas_names(&self) -> Vec<String> {
        vec![self.ada.handle.to_string(), self.ada.account.to_string()]
    }
}

/// In the talk of `agents`, Ada's machine creates `globex`, and a revision her device signs lists
/// Bob there with another master key than `acme` does; her `skald members globex` accepts it and
/// warns of Bob's two master keys: one key change waits. Before that, her `skald rules` prints no
/// line of key changes.
#[track_caller]
pub fn bobs_two_masters(agents: &Agents) {
    let talk = &agents.talk;
    let (cloud, ada) = (talk.cloud(), &talk.ada);
    assert_run_says_none(&ada.machine, &[world::RULES]);
    let globex = create_space(cloud, ada, GLOBEX);
    let bob = &talk.played[0].person;
    let listed = next_by(&latest(cloud, &ada.token, globex.id), ada, |list| {
        list.push(member_with_master(bob, Role::Member, other_master()));
    });
    store(cloud, &ada.token, &listed);
    let accepted = run(&ada.machine, &[MEMBERS, GLOBEX]);
    assert!(accepted.success, "{accepted:?}");
}

/// Bob's handle and account ID, as the key change of [`bobs_two_masters`] may name him.
pub fn bobs_names(agents: &Agents) -> Vec<String> {
    let bob = &agents.talk.played[0].person;
    vec![bob.handle.to_string(), bob.account.to_string()]
}
