//! Creating an invite link in the CLI (`docs/protocol/membership.md`, "Invite";
//! `architecture.md`, "Membership" and "Security"). On a `client` device of an admin of a space,
//! the person creates an invite: the role, member unless they ask for admin; the space's default
//! channel with `history` true, then the channels they name, each with whether it reads earlier
//! history; an expiry 7 days ahead unless an option sets another, up to 30 days. The device makes
//! the secret, gets an invite ID from the server, signs the invite and stores it, resending the
//! same bytes after a lost answer within the run. Once the server holds the invite, it shows the
//! link with the role and the expiry, and that the link admits one person and goes only to them,
//! over a channel the person trusts. The secret stays in memory: no request, database row, file or
//! key storage entry holds it.
//!
//! It shows the link only when its output is a terminal, which it checks before it sends anything;
//! otherwise, and run as an agent, it refuses and sends nothing. On a device of a member who is no
//! admin it refuses and sends nothing.
//!
//! Against a `skald-server` with GitHub sign-in through the stand-in, behind the recording relay of
//! the tests of signing in, which can lose the server's answers. The CLI runs at a terminal: in a
//! pseudo-terminal on Unix and a pseudo-console on Windows (`recovery_key/terminal.rs`). Each
//! machine keeps its secrets in the file key storage of its state directory, so the files of the
//! machine hold its key storage entries. The server's web address is its public address, the
//! default, which differs from the relay's address the machines reach the server at. The tests
//! read the invite the server holds through the API and check it with `skald-crypto`, apart from
//! the code under test.
//!
//! The CLI, as these tests use it, beside the commands of the tests of spaces and channels:
//! - `skald invite <space>`: on a device of an admin of the space named `<space>`, creates an
//!   invite as above and shows its link, `<web>/invite/<invite>#<fragment>`, on a line of its own
//!   or followed by whitespace; it shows the role as a word, `member` or `admin`, the date of the
//!   expiry as `YYYY-MM-DD`, in UTC or the machine's time zone, and says that the link admits `one
//!   person`, sent over a channel the person `trust`s; it exits with success;
//! - `--admin` invites an admin; `--channel <channel>`, repeatable, adds a channel by its ID with
//!   `history` true, `--channel-without-history <channel>`, repeatable, with `history` false;
//!   `--days <days>` sets the expiry that many days ahead, at most 30;
//! - it fails, sending no request about invites, for `--days` over 30, saying `30`, and for a
//!   channel the space lacks, naming its ID;
//! - with its output not a terminal, it fails saying `terminal` and sends nothing; run with
//!   `--agent <name>`, at a terminal, it fails saying `skald invite`, `person's command` and `an
//!   agent cannot run`, and sends nothing; on a device of a member who is no admin, it fails saying
//!   `admin` and sends no request about invites.
//!
//! The server's API, as these tests use it: `GET /invites/{invite}`, which gives the invite as the
//! server holds it; the answers of `POST /spaces/{space}/invites`, which issue invite IDs.

// Each test target uses a part of the shared modules.
#![allow(dead_code)]

// The world of the tests of roster revisions, at the terminal of the tests of the recovery key.
#[path = "../recovery_key/terminal.rs"]
mod terminal;
#[path = "../roster_revisions/world.rs"]
mod world;

#[path = "../../../server/tests/common/browser.rs"]
mod browser;
#[path = "../common/mod.rs"]
mod common;
#[path = "../device_keys/keys.rs"]
mod keys;

// The cloud, machines and relay of the tests of signing in.
#[path = "../sign_in/cloud.rs"]
mod cloud;
#[path = "../sign_in/machine.rs"]
mod machine;
#[path = "../sign_in/relay.rs"]
mod relay;

mod creating;
mod lost;
mod refusals;
mod secret;
mod web_address;

use std::collections::BTreeSet;
use std::time::Instant;

use skald_crypto::InviteLink;
use skald_proto::{Cbor as _, ChannelId, Invite, InviteId, Roster, SpaceId};

use cloud::{Cloud, unb64};
use machine::Machine;
use relay::strings_named;
use terminal::{Shown, Terminal};
use world::{ACME, CREATE_CHANNEL, Person, World, create, run, words};

/// The command that creates an invite, and its options.
pub const INVITE: &str = "invite";
pub const AS_ADMIN: &str = "--admin";
pub const CHANNEL: &str = "--channel";
pub const WITHOUT_HISTORY: &str = "--channel-without-history";
pub const DAYS: &str = "--days";

/// Seconds in a day.
pub const DAY: u64 = 86_400;
/// How far a stored expiry may stray from the one the run was asked for: the clocks of the run and
/// the test, read before and after it.
pub const SLACK: u64 = 60;

/// The setting of the request timeout, and the one a run that retries after a withheld answer
/// gets.
pub const TIMEOUT_SETTING: &str = "SKALD_TIMEOUT";
pub const TIMEOUT_SECONDS: &str = "4";

/// Ada, signed in on her first machine, which created `acme`, with its default channel in revision
/// 2, and two channels in it by `skald create-channel`, in revisions 3 and 4.
pub struct Admin {
    pub world: World,
    pub ada: Person,
    pub space: SpaceId,
    /// `acme`'s default channel.
    pub default: ChannelId,
    /// The two channels Ada's machine created.
    pub channels: [ChannelId; 2],
}

impl Admin {
    /// Ada is GitHub user `id` with `login`.
    pub fn new((id, login): (u64, &str)) -> Self {
        let world = World::start();
        let ada = world.sign_up(id, login, "ada-workstation");
        let second = create(&world.cloud, &ada, ACME);
        let space = second.body().space;
        let default = second.body().channels[0].id;
        let mut made = Vec::new();
        for _ in 0..2 {
            let created = run(&ada.machine, &[CREATE_CHANNEL, ACME]);
            assert!(created.success, "a channel: {created:?}");
            let latest = latest(&world.cloud, &ada.token, space);
            made.push(latest.body().channels.last().expect("a channel").id);
        }
        Admin {
            world,
            ada,
            space,
            default,
            channels: made.try_into().expect("two channels"),
        }
    }

    pub fn cloud(&self) -> &Cloud {
        &self.world.cloud
    }

    /// The last revision of `acme` the server holds.
    pub fn latest(&self) -> Roster {
        latest(self.cloud(), &self.ada.token, self.space)
    }
}

/// The last revision of `space` the server holds, for the session `token`.
pub fn latest(cloud: &Cloud, token: &str, space: SpaceId) -> Roster {
    let (_, roster) = cloud.revisions(token, space).pop().expect("a revision");
    roster
}

/// `skald invite acme <extra>`.
pub fn invite_args<'a>(extra: &[&'a str]) -> Vec<&'a str> {
    [&[INVITE, ACME][..], extra].concat()
}

/// Runs `skald invite acme <extra>` on `machine` at a terminal, until it exits.
pub fn invite_at_terminal(machine: &Machine, extra: &[&str]) -> Shown {
    Terminal::start(machine, &invite_args(extra)).finish()
}

/// The invite link `text` shows, under the server's web address `web`, as text and read back with
/// `skald-crypto`. Fails unless it shows exactly one link and it reads back.
#[track_caller]
pub fn shown_link(text: &str, web: &str) -> (String, InviteLink) {
    let head = format!("{web}/invite/");
    let links: BTreeSet<&str> = text
        .match_indices(&head)
        .map(|(at, _)| {
            let rest = &text[at..];
            let end = rest.find(char::is_whitespace).unwrap_or(rest.len());
            // A sentence may end right after the link.
            rest[..end].trim_end_matches(|c: char| !(c.is_ascii_alphanumeric() || "-_".contains(c)))
        })
        .collect();
    assert_eq!(links.len(), 1, "not one link under {web}: {text}");
    let link = links.first().expect("a link");
    let read = InviteLink::parse(link)
        .unwrap_or_else(|err| panic!("the link {link} does not read back: {err}: {text}"));
    ((*link).to_owned(), read)
}

/// Whether `text` shows anything like an invite link.
pub fn shows_a_link(text: &str) -> bool {
    text.contains("/invite/")
}

/// The invite the server holds under `id`, through the API as the session `token`; `None` if it
/// holds none.
pub fn server_invite(cloud: &Cloud, token: &str, id: InviteId) -> Option<Invite> {
    let path = format!("/invites/{id}");
    let (status, body) = cloud.get_as(&path, token);
    match status {
        200 => {
            let bytes = unb64(body["invite"].as_str().unwrap_or_else(|| panic!("{body}")));
            Some(Invite::from_cbor(&bytes).expect("an invite"))
        }
        404 => None,
        _ => panic!("GET {path}: {status} {body}"),
    }
}

/// The invite IDs the server issued in its answers from `since` on.
pub fn issued_since(cloud: &Cloud, since: Instant) -> Vec<InviteId> {
    strings_named(&cloud.relay.answered_since(since), "invite")
        .into_iter()
        .filter_map(|id| id.parse().ok())
        .collect()
}

/// The invites the server holds under the IDs it issued from `since` on.
pub fn stored_since(cloud: &Cloud, token: &str, since: Instant) -> Vec<Invite> {
    issued_since(cloud, since)
        .into_iter()
        .filter_map(|id| server_invite(cloud, token, id))
        .collect()
}

/// The heads of the requests about invites the machines sent from `since` on.
pub fn invite_requests(cloud: &Cloud, since: Instant) -> Vec<String> {
    (cloud.relay.sent_since(since).into_iter())
        .map(|sent| sent.head)
        .filter(|head| {
            head.lines()
                .next()
                .is_some_and(|line| line.contains("/invites"))
        })
        .collect()
}

/// Fails unless `text` holds `word` as a word.
#[track_caller]
pub fn assert_word(text: &str, word: &str) {
    assert!(
        text.lines()
            .any(|line| words(&line.to_lowercase()).contains(&word)),
        "it does not show {word:?}: {text}"
    );
}

/// The date of `expires`, seconds since the Unix epoch, as `YYYY-MM-DD` in UTC, and those of the
/// days before and after it, as a time zone may show it.
pub fn dates_around(expires: u64) -> [String; 3] {
    [expires - DAY, expires, expires + DAY].map(|second| {
        let second = i64::try_from(second).expect("a time");
        jiff::Timestamp::from_second(second)
            .expect("a time")
            .strftime("%Y-%m-%d")
            .to_string()
    })
}
