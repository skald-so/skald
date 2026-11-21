//! Account fingerprints and verified accounts (`docs/protocol/membership.md`, "Account fingerprint"
//! and "Device checks"). Each device shows the fingerprint of its own account, from the master key
//! it pinned, and of each member of a space it follows, from the roster. Two people compare the
//! ones their devices show for their own accounts over a channel they trust; a device marks an
//! account verified, for that master key, only on its person's yes at a terminal, never from a
//! pipe or as an agent. A verified account whose `master` changes in an accepted revision is no
//! longer verified, and the device warns. A verification stays on the device that made it.
//!
//! Against a `skald-server` with GitHub sign-in through the GitHub stand-in, behind the relay, as
//! the tests of roster revisions (`roster_revisions/main.rs`). Bob's first machine creates `acme`;
//! a fixture revision 3, which his device signs with the keys of its secrets file and stores
//! through the API, adds Ada's account as a member; her machine follows no space of his. Bob's
//! second machine is linked by hand, as in the tests of the history on new devices
//! (`history_devices/devices.rs`). A person's answers go through the pseudo-terminal of the tests of
//! the recovery key (`recovery_key/terminal.rs`); runs through a helper go through a copy of
//! `skald` of the machine's own, as in the tests of the helper (`helper/helped.rs`).
//!
//! The CLI, as these tests use it, besides what the tests of roster revisions use:
//! - `skald fingerprint`: shows the fingerprint of the device's own account as digits in groups of
//!   five, and no other run of eight or more groups of five digits.
//! - `skald fingerprint <space> <handle>`: shows the fingerprint of the account of the member with
//!   that handle in the space. As the person at a terminal, it then asks, on a line holding `?`,
//!   whether it matches what the member's own device shows, and takes `yes` or `no`; on `yes` it
//!   marks the account verified and exits with success. Through a pipe, or with `--agent <name>`
//!   in a folder bound to the space, it shows the fingerprint and marks nothing.
//! - `skald members <space>`: each member's line, holding its handle and role, and the lines under
//!   it up to the next member's, show the member's fingerprint, and say `verified` if this device
//!   verified the account for its current master key; otherwise they say `not verified`, or nothing
//!   that holds `verified`.
//! - An accepted revision that changes a verified account's `master` gives a warning line, one
//!   saying `warning`, that holds the member's handle and `verified`.

// Each test target uses a part of the shared modules.
#![allow(dead_code)]

mod changes;
mod showing;
mod verifying;

// The modules the tests of the channel history share (`history_common/mod.rs`), the second
// machines of the tests of the history on new devices and the helpers of the tests of reading the
// history, which those use.
#[path = "../history_common/mod.rs"]
mod history_common;
use history_common::*;
#[path = "../history_devices/devices.rs"]
mod devices;
#[path = "../history_reading/reading.rs"]
mod reading;

// The runs through a helper of the tests of the helper, and the terminal of the tests of the
// recovery key.
#[path = "../helper/helped.rs"]
mod helped;
#[path = "../recovery_key/terminal.rs"]
mod terminal;

use skald_proto::{Role, Roster};

use machine::{Finished, Machine};
use talk::{ADA_HOST, BOB_HOST};
use terminal::{Shown, Terminal};
use world::{
    ACME, ADD_RULE, AGENT, Person, World, create, dir_of, members, next_by, path, plain_lines, run,
    store, words,
};

/// The command that shows fingerprints.
pub const FINGERPRINT: &str = "fingerprint";

/// What the person answers to the question.
pub const YES: &str = "yes";
pub const NO: &str = "no";

/// The agent of the runs as an agent.
pub const CLAUDE: &str = "claude";

/// The fewest groups of five digits that hold 128 bits: 40 digits, 10^39 > 2^128.
const MIN_GROUPS: usize = 8;

/// Ada and Bob, each with a first machine; `acme`, which Bob's machine created, at revision 3,
/// which adds Ada's account as a member and which his machine accepted; and Ada's fingerprint, as
/// her machine shows it.
pub struct Pair {
    pub world: World,
    pub ada: Person,
    pub bob: Person,
    pub third: Roster,
    pub adas: String,
}

impl Pair {
    /// Ada is GitHub user `ids[0]` with `logins[0]`, Bob `ids[1]` with `logins[1]`.
    pub fn new(ids: [u64; 2], logins: [&str; 2]) -> Self {
        let world = World::start();
        let ada = world.sign_up(ids[0], logins[0], ADA_HOST);
        let bob = world.sign_up(ids[1], logins[1], BOB_HOST);
        let cloud = &world.cloud;
        let second = create(cloud, &bob, ACME);
        let third = next_by(&second, &bob, |members| {
            members.push(ada.member(Role::Member));
        });
        store(cloud, &bob.token, &third);
        let accepted = members(&bob.machine, ACME);
        assert!(
            words(&accepted.stdout).contains(&ada.handle.as_str()),
            "Bob's device does not show Ada as a member: {accepted:?}"
        );
        let adas = own_fingerprint(&ada);
        Pair {
            world,
            ada,
            bob,
            third,
            adas,
        }
    }

    /// Ada's handle.
    pub fn ada(&self) -> &str {
        self.ada.handle.as_str()
    }

    /// The handles of the space's members.
    pub fn handles(&self) -> [&str; 2] {
        [self.ada.handle.as_str(), self.bob.handle.as_str()]
    }

    /// A folder of Bob's first machine bound to `acme`, where his agents run.
    pub fn bobs_acme_folder(&self) -> std::path::PathBuf {
        let folder = dir_of(&self.bob.machine, "work/acme");
        let bound = run(&self.bob.machine, &[ADD_RULE, path(&folder), ACME]);
        assert!(bound.success, "not bound: {bound:?}");
        folder
    }

    /// Fails unless `text` shows Ada's fingerprint.
    #[track_caller]
    pub fn assert_shows_adas(&self, text: &str) {
        assert!(
            fingerprints_in(text).contains(&self.adas),
            "Ada's fingerprint {:?} is not shown: {text}",
            self.adas
        );
    }

    /// Fails unless `shown`, a run of `skald members acme`, shows Ada with her fingerprint, marked
    /// verified if `verified`, otherwise not.
    #[track_caller]
    pub fn assert_ada(&self, shown: &Finished, verified: bool) {
        assert_member(shown, (self.ada(), &self.handles()), &self.adas, verified);
    }

    /// Runs `skald members acme` on `machine`; fails unless it shows Ada as [`Pair::assert_ada`]
    /// says.
    #[track_caller]
    pub fn assert_ada_on(&self, machine: &Machine, verified: bool) {
        self.assert_ada(&members(machine, ACME), verified);
    }

    /// Bob answers yes for Ada at a terminal on his first machine, as [`verified_at_terminal`].
    #[track_caller]
    pub fn bob_verifies_ada(&self) {
        verified_at_terminal(&self.bob.machine, self.ada(), &self.adas);
    }

    /// Fails unless Bob's first machine shows Ada not verified, and verified once he answers yes
    /// for her at a terminal: a run before marked nothing, though a yes could.
    #[track_caller]
    pub fn assert_unmarked_until_bobs_yes(&self) {
        self.assert_ada_on(&self.bob.machine, false);
        self.bob_verifies_ada();
        self.assert_ada_on(&self.bob.machine, true);
    }
}

/// The fingerprints in `text`: each run of eight or more words of five ASCII digits, words split
/// at whitespace, joined by one space.
pub fn fingerprints_in(text: &str) -> Vec<String> {
    let mut found = Vec::new();
    let mut run: Vec<&str> = Vec::new();
    let words = text.split_whitespace().chain(std::iter::once(""));
    for word in words {
        if word.len() == 5 && word.bytes().all(|byte| byte.is_ascii_digit()) {
            run.push(word);
            continue;
        }
        if run.len() >= MIN_GROUPS {
            found.push(run.join(" "));
        }
        run.clear();
    }
    found
}

/// The fingerprint of `person`'s own account, as `skald fingerprint` shows it on their machine;
/// fails unless it shows one.
#[track_caller]
pub fn own_fingerprint(person: &Person) -> String {
    let shown = run(&person.machine, &[FINGERPRINT]);
    assert!(shown.success, "{FINGERPRINT}: {shown:?}");
    let found = fingerprints_in(&shown.shown());
    let [one] = found.as_slice() else {
        panic!("not one fingerprint: {shown:?}");
    };
    one.clone()
}

/// `skald fingerprint acme <handle>` on `machine`, its input empty, as no person types.
pub fn shown_for(machine: &Machine, handle: &str) -> Finished {
    run(machine, &[FINGERPRINT, ACME, handle])
}

/// `skald fingerprint acme <handle>` on `machine` at a terminal: once it shows a fingerprint and a
/// `?`, the person answers `answer`. Gives what it showed.
pub fn answered_at_terminal(machine: &Machine, handle: &str, answer: &str) -> Shown {
    let mut terminal = Terminal::start(machine, &[FINGERPRINT, ACME, handle]);
    terminal.wait_for("a fingerprint and a question", |shown| {
        !fingerprints_in(shown).is_empty() && shown.contains('?')
    });
    terminal.type_line(answer);
    terminal.finish()
}

/// The person on `machine` answers `yes` at a terminal for the member `handle`; fails unless the
/// run shows `expected`, the member's fingerprint, and succeeds.
#[track_caller]
pub fn verified_at_terminal(machine: &Machine, handle: &str, expected: &str) -> Shown {
    let shown = answered_at_terminal(machine, handle, YES);
    assert!(
        fingerprints_in(&shown.text)
            .iter()
            .any(|one| one == expected),
        "{handle}'s fingerprint {expected:?} is not shown: {}",
        shown.text
    );
    assert!(shown.success, "{}", shown.text);
    shown
}

/// The lines of `shown`, a run of `skald members`, that are no warnings, of the member `handle`:
/// from its line to the line of the next of `handles`.
pub fn member_block(shown: &Finished, handle: &str, handles: &[&str]) -> String {
    let lines = plain_lines(shown);
    let start = lines
        .iter()
        .position(|line| words(line).contains(&handle))
        .unwrap_or_else(|| panic!("{handle} is not shown: {shown:?}"));
    let mut block = vec![lines[start].clone()];
    for line in &lines[start + 1..] {
        let line_words = words(line);
        if handles
            .iter()
            .any(|other| *other != handle && line_words.contains(other))
        {
            break;
        }
        block.push(line.clone());
    }
    block.join("\n")
}

/// Whether `text` says `verified`, but not as `not verified` or `unverified`.
pub fn says_verified(text: &str) -> bool {
    let lower = text.to_lowercase();
    lower
        .replace("not verified", "")
        .replace("unverified", "")
        .contains("verified")
}

/// Fails unless `shown`, a run of `skald members`, shows the member `handle` with the fingerprint
/// `expected`, marked verified if `verified`, otherwise not.
#[track_caller]
pub fn assert_member(
    shown: &Finished,
    (handle, handles): (&str, &[&str]),
    expected: &str,
    verified: bool,
) {
    let block = member_block(shown, handle, handles);
    assert!(
        fingerprints_in(&block).iter().any(|one| one == expected),
        "{handle} is not shown with the fingerprint {expected:?}: {shown:?}"
    );
    // The handles, such as `ada-verified-one-shot`, may hold `verified` themselves.
    let unnamed = (handles.iter()).fold(block.clone(), |text, one| text.replace(one, ""));
    assert_eq!(
        says_verified(&unnamed),
        verified,
        "{handle} verified: {block:?}, in {shown:?}"
    );
}
