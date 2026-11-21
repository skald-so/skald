//! The DNS root of trust of a space (`docs/protocol/membership.md`, "Policy" and "Account
//! fingerprint"; `docs/dev/architecture.md`, "Network").
//!
//! - A space's policy may name its organization's domain; a new space's names none. An admin's
//!   device sets or clears it by the space's next revision; a device of a member that is no admin
//!   refuses and sends nothing.
//! - TXT records at `_skald.<domain>` list accounts by ID, each with its fingerprint, one per
//!   record. A member device of a space whose policy names a domain looks them up with DNS over
//!   HTTPS [RFC 8484], at the resolver its setting [`RESOLVER`] names, through `HTTPS_PROXY` as it
//!   reaches its server. Results from another resolver are kept apart, so a run with another one,
//!   as an agent could make, changes nothing later runs show. A lookup is made again after the
//!   records' TTL; a resolver that fails leaves the members not checked and fails no command.
//! - A member the records list with its fingerprint shows as listed by the domain. One they list
//!   with another fingerprint, and an admin they do not list, are key changes: the device warns,
//!   once. Other members they do not list show nothing. The check comes on top of the roster, never
//!   instead of it.
//!
//! Against a `skald-server` with GitHub sign-in through the GitHub stand-in, on PostgreSQL, behind
//! the recording relay; a test plays the person on the verification page in headless Chrome. Bob's
//! machine creates `acme`, and a revision his device signs adds Ada as a member, as the tests of
//! roster revisions do (`roster_revisions/main.rs`); her machine follows no space of his. In most
//! tests a revision 4 his device signs names the domain [`DOMAIN`]. Bob's machine looks names up at
//! a DNS-over-HTTPS stand-in of its own (`doh.rs`) from its first run on, so no run reaches a real
//! resolver.
//!
//! What the tests chose, beside what the spec names:
//! - the setting of the resolver's URL: [`RESOLVER`], by default Cloudflare's;
//! - a record: the account's ID, as the CLI shows IDs, one space and the fingerprint as the CLI shows
//!   it, eight groups of five digits joined by one space ([`record`]);
//! - `skald set-domain <space> <domain>` and `skald clear-domain <space>`, as `skald
//!   set-history-period` sets the history period;
//! - `skald members <space>` shows the domain on a line that is no warning before the first
//!   member's line; a member the domain lists with its fingerprint has the domain on a line of its
//!   own lines, from its line to the next member's, that is no warning; no other member's lines
//!   hold it;
//! - a key change the domain shows: a line saying `warning` that holds the member's handle.

// Each test target uses a part of the shared modules.
#![allow(dead_code)]

mod agents;
mod doh;
mod failing;
mod listing;
mod proxied;
mod setting;

// The modules the tests of the channel history share (`history_common/mod.rs`).
#[path = "../history_common/mod.rs"]
mod history_common;
use history_common::*;
// The TLS endpoint and test CA of the tests of HTTPS, and the recording proxy of the tests behind
// a proxy.
#[path = "../https_proxy/proxy.rs"]
mod proxy;
#[path = "../common/tls.rs"]
mod tls;
#[path = "../https/front.rs"]
mod tls_front;

use skald_crypto::{SigningKey, account_fingerprint};
use skald_proto::{AccountId, Roster, RosterBody};

use doh::Doh;
use machine::{Finished, Machine};
use talk::{ADA_HOST, BOB_HOST};
use world::{ACME, Person, World, create_with_member, members, next_with, store, warnings, words};

/// The setting of the DNS-over-HTTPS resolver's URL.
pub const RESOLVER: &str = "SKALD_DNS_RESOLVER";

/// The commands that set and clear a space's domain.
pub const SET_DOMAIN: &str = "set-domain";
pub const CLEAR_DOMAIN: &str = "clear-domain";

/// The organization's domain, and the name of its records.
pub const DOMAIN: &str = "corp-q7k2.example";
pub const RECORDS: &str = "_skald.corp-q7k2.example";

/// TTLs of the records, in seconds: one that outlasts every test, and one a test waits out.
pub const LONG_TTL: u32 = 3600;
pub const SHORT_TTL: u32 = 1;

/// How long a test waits for [`SHORT_TTL`] to run out.
pub const PAST_SHORT_TTL: std::time::Duration = std::time::Duration::from_millis(2500);

/// An agent of Bob's machine.
pub const CLAUDE: &str = "claude";

/// The record that lists `account` with the fingerprint of the master key `master`.
pub fn record(account: AccountId, master: &[u8; 32]) -> String {
    format!("{account} {}", account_fingerprint(account, master))
}

/// The record that lists `person`'s account with its fingerprint.
pub fn listed(person: &Person) -> String {
    record(person.account, &person.master)
}

/// A record that lists `person`'s account with the fingerprint of another master key.
pub fn listed_elsewhere(person: &Person) -> String {
    record(person.account, &SigningKey::generate().public_key())
}

/// A new account of GitHub user `id` with `login`, whose first machine, on the host `host`, runs
/// with the settings `env` beside its own from its sign-in on.
pub fn sign_up_with(
    world: &World,
    (id, login): (u64, &str),
    host: &'static str,
    env: &[(String, String)],
) -> Person {
    let user = world.cloud.user(id, login);
    let mut machine = Machine::new().with_host_name(host);
    for (name, value) in env {
        machine = machine.with_env(name, value);
    }
    let signed_in = (world.cloud).signed_in(&world.browser.page(), &machine, &user, &[]);
    assert!(signed_in.success, "{signed_in:?}");
    world.person(user, machine, host)
}

/// Bob's machine, which looks names up at [`Scene::doh`], and Ada's; `acme`, which Bob's machine
/// created, at revision 3, which his device signed and which adds Ada as a member.
pub struct Scene {
    pub world: World,
    pub bob: Person,
    pub ada: Person,
    pub third: Roster,
    pub doh: Doh,
}

impl Scene {
    /// Bob is GitHub user `ids[0]` with `logins[0]`, Ada `ids[1]` with `logins[1]`; Bob's machine
    /// looks names up at a stand-in of its own.
    pub fn new(ids: [u64; 2], logins: [&str; 2]) -> Self {
        let doh = Doh::start();
        let env = vec![(RESOLVER.to_owned(), doh.url())];
        Self::with(ids, logins, doh, &env)
    }

    /// As [`Scene::new`], Bob's machine with the settings `env` from its sign-in on, its resolver
    /// among them, and `doh` behind it.
    pub fn with(ids: [u64; 2], logins: [&str; 2], doh: Doh, env: &[(String, String)]) -> Self {
        let world = World::start();
        let bob = sign_up_with(&world, (ids[0], logins[0]), BOB_HOST, env);
        let ada = world.sign_up(ids[1], logins[1], ADA_HOST);
        let third = create_with_member(&world.cloud, &bob, ACME, &ada);
        Scene {
            world,
            bob,
            ada,
            third,
            doh,
        }
    }

    /// As [`Scene::new`], with revision 4 naming [`DOMAIN`] and no other change, and the records
    /// `records` gives for Bob and Ada, with [`LONG_TTL`].
    pub fn named(
        ids: [u64; 2],
        logins: [&str; 2],
        records: impl FnOnce(&Person, &Person) -> Vec<String>,
    ) -> Self {
        let scene = Self::new(ids, logins);
        scene.name_domain(|_| {});
        scene.records(LONG_TTL, &records(&scene.bob, &scene.ada));
        scene
    }

    /// Revision 4, signed by Bob's device, which names [`DOMAIN`] and makes the changes `change`
    /// to the body; the server stores it.
    pub fn name_domain(&self, change: impl FnOnce(&mut RosterBody)) -> Roster {
        let fourth = next_with(&self.third, (self.bob.device, &self.bob.key), |body| {
            body.policy.domain = Some(DOMAIN.to_owned());
            change(body);
        });
        store(&self.world.cloud, &self.bob.token, &fourth);
        fourth
    }

    /// From now on, the stand-in gives `texts` as the records of the domain, with the TTL `ttl`.
    pub fn records(&self, ttl: u32, texts: &[String]) {
        self.doh.set_txt(RECORDS, ttl, texts);
    }

    /// The handles of the space's members.
    pub fn handles(&self) -> [&str; 2] {
        [self.bob.handle.as_str(), self.ada.handle.as_str()]
    }

    /// Runs `skald members acme` on Bob's machine; fails unless it succeeds.
    pub fn members(&self) -> Finished {
        members(&self.bob.machine, ACME)
    }

    /// Fails unless `shown`, a run of `skald members acme`, shows the domain and shows `person`
    /// listed by it if `is_listed`, otherwise not.
    #[track_caller]
    pub fn assert_listed(&self, shown: &Finished, person: &Person, is_listed: bool) {
        assert_shows_domain(shown, &self.handles());
        let lines = member_lines(shown, person.handle.as_str(), &self.handles());
        assert_eq!(
            lines.iter().any(|line| holds_domain(line)),
            is_listed,
            "{} listed by {DOMAIN}: {lines:?}, in {shown:?}",
            person.handle.as_str()
        );
    }

    /// Fails unless `shown`, a run of `skald members acme`, has `count` warnings naming `person`.
    #[track_caller]
    pub fn assert_warned(&self, shown: &Finished, person: &Person, count: usize) {
        let handle = person.handle.as_str();
        assert_eq!(
            warnings_naming(shown, handle).len(),
            count,
            "warnings naming {handle}: {shown:?}"
        );
    }

    /// Fails unless `shown`, a run of `skald members acme`, shows Bob and Ada listed and warns of
    /// neither.
    #[track_caller]
    pub fn assert_both_listed(&self, shown: &Finished) {
        for person in [&self.bob, &self.ada] {
            self.assert_listed(shown, person, true);
            self.assert_warned(shown, person, 0);
        }
    }

    /// Two runs of `skald members acme`, [`SHORT_TTL`] run out between them; fails unless the
    /// first warns once naming Ada and the second does not, and each shows Bob listed and Ada not.
    #[track_caller]
    pub fn ada_warned_once(&self) -> [Finished; 2] {
        let first = self.members();
        std::thread::sleep(PAST_SHORT_TTL);
        let second = self.members();
        self.assert_warned(&first, &self.ada, 1);
        self.assert_warned(&second, &self.ada, 0);
        for shown in [&first, &second] {
            self.assert_listed(shown, &self.ada, false);
            self.assert_listed(shown, &self.bob, true);
        }
        [first, second]
    }
}

/// Whether `line` holds [`DOMAIN`] as a word: whitespace around it, or punctuation that ends a
/// clause.
pub fn holds_domain(line: &str) -> bool {
    line.split_whitespace().any(|word| {
        let word = word.trim_matches(|char: char| "()[]{}<>,;:'\"`".contains(char));
        word.trim_end_matches('.').eq_ignore_ascii_case(DOMAIN)
    })
}

/// The lines of standard output of `shown`, a run of `skald members`, that are no warnings.
fn listing_lines(shown: &Finished) -> Vec<&str> {
    (shown.stdout.lines())
        .filter(|line| !line.to_lowercase().contains(world::WARNING))
        .collect()
}

/// Whether `line` holds one of `handles` as a word.
fn names_one_of(line: &str, handles: &[&str]) -> bool {
    let line_words = words(line);
    handles.iter().any(|handle| line_words.contains(handle))
}

/// The lines of `shown`, a run of `skald members`, that are no warnings, of the member `handle`:
/// from its line to the line of the next of `handles`.
pub fn member_lines(shown: &Finished, handle: &str, handles: &[&str]) -> Vec<String> {
    let lines = listing_lines(shown);
    let start = (lines.iter())
        .position(|line| words(line).contains(&handle))
        .unwrap_or_else(|| panic!("{handle} is not shown: {shown:?}"));
    let others: Vec<&str> = handles
        .iter()
        .copied()
        .filter(|one| *one != handle)
        .collect();
    let mut block = vec![lines[start].to_owned()];
    for line in &lines[start + 1..] {
        if names_one_of(line, &others) {
            break;
        }
        block.push((*line).to_owned());
    }
    block
}

/// Fails unless `shown`, a run of `skald members`, shows [`DOMAIN`] on a line that is no warning
/// before the first line of one of `handles`.
#[track_caller]
pub fn assert_shows_domain(shown: &Finished, handles: &[&str]) {
    let lines = listing_lines(shown);
    let header: Vec<&&str> = (lines.iter())
        .take_while(|line| !names_one_of(line, handles))
        .collect();
    assert!(
        header.iter().any(|line| holds_domain(line)),
        "{DOMAIN} is not shown before the members: {shown:?}"
    );
}

/// The warning lines of `shown` that hold `handle` as a word.
pub fn warnings_naming(shown: &Finished, handle: &str) -> Vec<String> {
    let mut lines = warnings(shown);
    lines.retain(|line| words(line).contains(&handle));
    lines
}
