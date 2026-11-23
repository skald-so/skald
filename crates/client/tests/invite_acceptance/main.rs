//! Accepting an invite in the CLI (`docs/protocol/membership.md`, "Invite"; `architecture.md`,
//! "Membership" and "Security"). A person of another account than the space's takes an invite link
//! only from a terminal, never from an argument, a pipe or the environment, and never as an agent.
//! Their `client` device takes the invite by its ID and takes the newcomer's steps of "Invite": the
//! revision with the link's hash as its anchor, the later ones accepted as roster revisions are;
//! the invite signed by an admin's device per those revisions, its `key` derived from the link's
//! secret, and `expires` not passed. Only then does it show the invite, ask its person to confirm
//! and sign the acceptance, for its account, its pinned master key and a free handle. It writes the
//! anchor and the acceptance before sending the acceptance, resends it until the server takes it,
//! follows the space from the anchor and signs anchors objects naming it. Once a revision admits
//! the newcomer, its device joins the space's default channel by itself; when the invite ends
//! first, it says so and follows the space no more. The secret reaches neither the server nor the
//! disk.
//!
//! As the tests of channel messages (`channel_messages/main.rs`): a `skald-server` with GitHub
//! sign-in through the stand-in, behind the front and the recording relay of the tests of sending,
//! which the tests use as a proxy that sees, loses, holds and changes what goes through. Ada's
//! machine signed up first and created `acme`, with its default channel; Bob's machine is the first
//! device of another account. The invites are made as Ada's device with `skald-crypto`, apart from
//! the code under test, and issued and stored through the server's API; the tests build the link
//! from the server's web address, the invite's ID, its secret and the hash of a revision of `acme`.
//! A revision that admits Bob is a fixture revision, signed with the key of Ada's device and stored
//! through the API, as in the tests of roster revisions. The CLI runs at a terminal: in a
//! pseudo-terminal on Unix and a pseudo-console on Windows (`recovery_key/terminal.rs`). Each
//! machine keeps its secrets in the file key storage of its state directory, so the files of the
//! machine hold its key storage entries.
//!
//! The CLI, as these tests use it, beside the commands of the tests of spaces, channels, sending
//! and receiving:
//! - `skald accept-invite`, at a terminal as the person, asks for the invite link on a line that
//!   says `link` and reads it from the terminal. It then shows the space's name, the role as a
//!   word, `member` or `admin`, the handle and device name of the admin whose device signed the
//!   invite and the number of the invite's channels as a number, and asks to confirm on a line
//!   holding `?`: `yes` confirms, `no` declines and signs nothing. The handle it asks for is the
//!   account's username, or the one `--handle <handle>` names; once confirmed, for a handle that is
//!   no valid name or that a member or a service agent owner holds in the latest revision, it asks
//!   for another on a line that says `handle`, until it gets a free one. It then stores the
//!   acceptance, follows the space, says that it `wait`s for an admin's device and how to bind a
//!   folder to the space (`add-rule` or `.skald.toml`), and exits with success.
//! - It refuses, saying why after the link, without asking to confirm, signing nothing and
//!   following nothing, and exits failing: a link whose revision the server does not give or whose
//!   later revisions are off its chain, saying `revision`; an invite signed by a device of a member
//!   who is no admin, saying `admin`; one signed by a device without a valid certificate, saying
//!   `admin` or `certificate`; a link whose secret does not derive the invite's `key`, saying
//!   `secret` or `key`; an expired invite, saying `expired`; an account already a member, saying
//!   `already`; an invite the server does not hold, saying `ended`. A link of another server than
//!   the device's, saying `server`, and a malformed link, saying `link`, it refuses sending
//!   nothing.
//! - A link given as an argument, and a run whose input or output is no terminal, it refuses,
//!   saying `terminal` and sending nothing; a link in the environment it never reads, and asks for
//!   one. Run with `--agent <name>`, it fails saying `skald accept-invite`, `person's command` and
//!   `an agent cannot run`, and sends nothing. On a machine that never signed in, a malformed
//!   link also fails before it sends anything.
//! - A run that connects, such as `skald recv`, sends an acceptance the device wrote that the
//!   server has not taken; it follows the space while the device waits; once a revision admits
//!   the device's account, the device joins the default channel by itself. When the server no
//!   longer holds the invite before a revision admits the account, such a run says the invite
//!   `ended`, and the device follows the space no more.
//!
//! The server's API, as these tests use it: `POST /spaces/{space}/invites`, which issues an invite
//! ID; `PUT /spaces/{space}/invites/{invite}`, which stores an invite; `GET /invites/{invite}`,
//! whose answer the relay changes; `PUT /invites/{invite}/acceptance`, whose answer the relay loses
//! or holds; `GET /spaces/{space}/acceptances`, which gives an admin's session the space's waiting
//! acceptances; and the anchors objects of the tests of anchors objects.

// Each test target uses a part of the shared modules.
#![allow(dead_code)]

mod accepting;
mod ended;
mod handles;
mod links;
mod lost;
mod refusals;
mod secret;
mod terminal_only;
mod web_address;

// The modules the tests of the channel history share (`history_common/mod.rs`), the anchors
// objects of the tests of anchors, and the terminal of the tests of the recovery key.
#[path = "../history_common/mod.rs"]
mod history_common;
use history_common::*;
#[path = "../anchors/objects.rs"]
mod anchors;
#[path = "../recovery_key/terminal.rs"]
mod terminal;

use std::thread;
use std::time::{Duration, Instant};

use serde_json::{Value, json};
use skald_crypto::{InviteLink, InviteSecret, SigningKey, check_acceptance, sign_invite};
use skald_proto::{
    Acceptance, Cbor as _, ChannelMember, Invite, InviteBody, InviteId, Member, Name, NonEmpty,
    Role, Roster, SpaceId, Unknown,
};

use cloud::{Cloud, WAIT, unb64};
use fixtures::fronted;
use front::Front;
use kit::{Space, b64, create_space};
use machine::{Finished, Machine, RUN_LIMIT};
use talk::{ADA_HOST, BOB_HOST};
use terminal::{Shown, Terminal};
use world::{ACME, Person, World, holds_word, next_with, now, plain_lines, spaces};

/// The command that accepts an invite, and its option that names the handle.
pub const ACCEPT: &str = "accept-invite";
pub const HANDLE: &str = "--handle";

/// The answers to the question that confirms.
pub const YES: &str = "yes";
pub const NO: &str = "no";

/// Seconds in a day.
pub const DAY: u64 = 86_400;

/// Ada, whose machine signed up first and created `acme`, and Bob, the first device of another
/// account.
pub struct Scene {
    pub world: World,
    pub front: Front,
    pub ada: Person,
    pub bob: Person,
    pub space: Space,
}

impl Scene {
    /// Ada is GitHub user `ada.0` with the login `ada.1`, Bob `bob.0` with `bob.1`.
    pub fn new(ada: (u64, &str), bob: (u64, &str)) -> Self {
        Self::with(ada, bob, |_| {})
    }

    /// As [`Scene::new`], with `before_bob` done to the cloud once Ada created `acme`, before Bob
    /// signs up.
    pub fn with(ada: (u64, &str), bob: (u64, &str), before_bob: impl FnOnce(&Cloud)) -> Self {
        let (world, front) = fronted();
        let ada = world.sign_up(ada.0, ada.1, ADA_HOST);
        let space = create_space(&world.cloud, &ada, ACME);
        before_bob(&world.cloud);
        let bob = world.sign_up(bob.0, bob.1, BOB_HOST);
        Scene {
            world,
            front,
            ada,
            bob,
            space,
        }
    }

    pub fn cloud(&self) -> &Cloud {
        &self.world.cloud
    }

    /// The last revision of `acme` the server holds.
    pub fn latest(&self) -> Roster {
        kit::latest(self.cloud(), &self.ada.token, self.space.id)
    }

    /// An invite to `acme` for a member, to its default channel with `history` true, expiring 7
    /// days ahead, signed by Ada's device, issued and stored through the API; its link names the
    /// last revision of `acme`.
    pub fn invite(&self) -> Made {
        self.invite_with(|_| {})
    }

    /// As [`Scene::invite`], with `change` made to the invite's body before it is signed.
    pub fn invite_with(&self, change: impl FnOnce(&mut InviteBody)) -> Made {
        let cloud = self.cloud();
        let space = self.space.id;
        let id = issue(cloud, &self.ada.token, space);
        let secret = InviteSecret::generate();
        let bytes = *secret.to_bytes();
        let revision = self.latest();
        let mut body = InviteBody {
            invite: id,
            space,
            key: secret.invite_key().public_key(),
            role: Role::Member,
            channels: vec![(self.space.default, true)],
            signer: self.ada.device,
            expires: now() + 7 * DAY,
            unknown: Unknown::default(),
        };
        change(&mut body);
        let invite = sign_invite(&body, &self.ada.key).expect("an invite");
        store_invite(cloud, &self.ada.token, &invite);
        Made {
            link: link_of(&cloud.public, id, &bytes, revision.hash()),
            invite,
            secret: bytes,
            revision,
        }
    }

    /// The acceptances of `acme` the server holds waiting, as Ada's session gets them.
    pub fn waiting(&self) -> Vec<Acceptance> {
        waiting(self.cloud(), &self.ada.token, self.space.id)
    }

    /// The hash of the revision of `acme` that the anchors object the server holds of Bob's device
    /// names, if one names `acme`.
    pub fn bob_anchors(&self) -> Option<[u8; 32]> {
        anchors::anchored(self.cloud(), &self.bob, self.space.id)
    }

    /// The revision after the last one of `acme`, signed by Ada's device and stored through the
    /// API, that admits Bob with `made`'s invite, `handle` and `made`'s role, his pinned master key,
    /// and adds him to the default channel with `history` true.
    pub fn admit_bob(&self, made: &Made, handle: &str) -> Roster {
        let previous = self.latest();
        let bob = &self.bob;
        let admitting = next_with(&previous, (self.ada.device, &self.ada.key), |body| {
            let mut members = body.members.clone().into_vec();
            members.push(Member {
                account: bob.account,
                handle: handle.parse().expect("a handle"),
                role: made.invite.body().role,
                master: bob.master,
                invite: Some(made.id()),
                excluded: None,
                unknown: Unknown::default(),
            });
            body.members = NonEmpty::from_vec(members).expect("a member");
            body.channels[0].members.push(ChannelMember {
                account: bob.account,
                history: true,
            });
        });
        world::store(self.cloud(), &self.ada.token, &admitting);
        admitting
    }

    /// Makes an invite, which Bob accepts at a terminal, confirming, with his username; fails
    /// unless the run succeeds and the server holds his acceptance. Gives the invite.
    #[track_caller]
    pub fn bob_accepts(&self) -> Made {
        let made = self.invite();
        let accepted = accept_at_terminal(&self.bob.machine, &made.link, &[], (YES, &[]));
        assert!(accepted.shown.success, "{}", accepted.text());
        self.assert_bob_accepted(&made, self.bob.handle.as_str());
        made
    }

    /// Fails unless the server holds one waiting acceptance of `acme`: Bob's of `made`, naming his
    /// account, his pinned master key and `handle`, signed by `made`'s invite key.
    #[track_caller]
    pub fn assert_bob_accepted(&self, made: &Made, handle: &str) {
        let held = self.waiting();
        let [acceptance] = held.as_slice() else {
            panic!("not one waiting acceptance: {held:?}");
        };
        let body = acceptance.body();
        assert_eq!(body.invite, made.id(), "{body:?}");
        assert_eq!(body.account, self.bob.account, "{body:?}");
        assert_eq!(body.master, self.bob.master, "not his pinned master key");
        assert_eq!(body.handle.as_str(), handle, "{body:?}");
        check_acceptance(acceptance, &made.invite.body().key)
            .unwrap_or_else(|err| panic!("the acceptance's signature: {err}"));
    }

    /// Fails unless Bob's machine accepted nothing from `since` on: it sent no acceptance of
    /// `made`, the server holds no waiting acceptance of `acme`, no anchors object of Bob's names
    /// `acme`, and `skald spaces` on his machine does not list it.
    #[track_caller]
    pub fn assert_nothing_accepted(&self, made: &Made, since: Instant) {
        let sent = self
            .cloud()
            .count_sent(since, &acceptance_request(made.id()));
        assert_eq!(sent, 0, "an acceptance was sent");
        assert_eq!(self.waiting(), Vec::new(), "acceptances held");
        assert_eq!(self.bob_anchors(), None, "Bob's anchors object names acme");
        assert!(
            !follows(&spaces(&self.bob.machine), ACME),
            "Bob's machine follows acme"
        );
    }

    /// Fails unless Bob's machine sent no request at all from `since` on.
    #[track_caller]
    pub fn assert_nothing_sent(&self, since: Instant) {
        world::assert_nothing_sent(self.cloud(), since);
    }
}

/// An invite the tests made, its secret, the revision its link names, and its link.
pub struct Made {
    pub invite: Invite,
    pub secret: [u8; 32],
    pub revision: Roster,
    pub link: String,
}

impl Made {
    pub fn id(&self) -> InviteId {
        self.invite.body().invite
    }
}

/// The link `<web>/invite/<invite>#<fragment>` of `secret` and the revision hash `revision`.
pub fn link_of(web: &str, invite: InviteId, secret: &[u8; 32], revision: [u8; 32]) -> String {
    let link = InviteLink {
        web: web.to_owned(),
        invite,
        secret: InviteSecret::from_bytes(secret),
        revision,
    };
    link.shown().to_string()
}

/// `made`'s invite with `change` made to its body, signed with `key`.
pub fn resigned(made: &Made, key: &SigningKey, change: impl FnOnce(&mut InviteBody)) -> Invite {
    let mut body = made.invite.body().clone();
    change(&mut body);
    sign_invite(&body, key).expect("an invite")
}

/// The request that stores an acceptance of the invite `invite`, as its line starts.
pub fn acceptance_request(invite: InviteId) -> String {
    format!("PUT /invites/{invite}/acceptance")
}

/// The request for the invite `invite`, as its line starts.
pub fn invite_request(invite: InviteId) -> String {
    format!("GET /invites/{invite} ")
}

/// From now on the relay gives the answer to a request for the invite `invite` as `change` makes
/// it, as a compromised server would.
pub fn change_invite_answer(
    cloud: &Cloud,
    invite: InviteId,
    change: impl Fn(&mut Value) + Send + Sync + 'static,
) {
    cloud.relay.rewrite(&invite_request(invite), move |answer| {
        let mut given: Value = serde_json::from_str(answer).expect("the server's invite");
        change(&mut given);
        given.to_string()
    });
}

/// From now on the relay gives `invite` in place of the invite the server holds under its ID.
pub fn give_invite(cloud: &Cloud, invite: &Invite) {
    let bytes = b64(&invite.to_cbor());
    change_invite_answer(cloud, invite.body().invite, move |given| {
        given["invite"] = json!(bytes);
    });
}

/// From now on the relay gives `revisions` as the space's revisions with the invite `invite`.
pub fn give_revisions(cloud: &Cloud, invite: InviteId, revisions: &[&Roster]) {
    let list: Vec<String> = (revisions.iter())
        .map(|roster| b64(&roster.to_cbor()))
        .collect();
    change_invite_answer(cloud, invite, move |given| {
        given["revisions"] = json!(list);
    });
}

/// An invite ID the server issues for `space` to the session `token`, through the API.
#[track_caller]
pub fn issue(cloud: &Cloud, token: &str, space: SpaceId) -> InviteId {
    let path = format!("/spaces/{space}/invites");
    let (status, body) = world::post_as(cloud, token, &path, None);
    match (status, body["invite"].as_str().map(str::parse::<InviteId>)) {
        (201, Some(Ok(id))) => id,
        _ => panic!("no invite ID issued for {space}: {status} {body}"),
    }
}

/// Stores `invite` as the session `token`, through the API; fails unless the server stores it.
#[track_caller]
pub fn store_invite(cloud: &Cloud, token: &str, invite: &Invite) {
    let body = invite.body();
    let path = format!("/spaces/{}/invites/{}", body.space, body.invite);
    let request = json!({ "invite": b64(&invite.to_cbor()) });
    let (status, answer) = world::put_as(cloud, token, &path, &request);
    assert_eq!(status, 201, "the server did not store the invite: {answer}");
}

/// The waiting acceptances of `space` the server gives the session `token`, straight from the
/// server.
pub fn waiting(cloud: &Cloud, token: &str, space: SpaceId) -> Vec<Acceptance> {
    let path = format!("/spaces/{space}/acceptances");
    let (status, body) = cloud.get_as(&path, token);
    assert_eq!(status, 200, "GET {path}: {body}");
    body["acceptances"]
        .as_array()
        .unwrap_or_else(|| panic!("GET {path}: {body}"))
        .iter()
        .map(|entry| {
            let bytes = unb64(entry["acceptance"].as_str().expect("an acceptance"));
            Acceptance::from_cbor(&bytes).expect("an acceptance that decodes")
        })
        .collect()
}

/// Whether `listed`, what `skald spaces` showed, lists the space `name`.
pub fn follows(listed: &Finished, name: &str) -> bool {
    holds_word(&plain_lines(listed), name)
}

/// How many times `text` says `word`, in any case.
pub fn mentions(text: &str, word: &str) -> usize {
    text.to_lowercase().matches(&word.to_lowercase()).count()
}

/// Whether `text`, what a run showed in all, says `word`, in any case, more often than `before`,
/// what it showed earlier: so it says `word` after that.
pub fn says_after(before: &str, text: &str, word: &str) -> bool {
    mentions(text, word) > mentions(before, word)
}

/// How many questions `text` asks: its `?`.
pub fn questions(text: &str) -> usize {
    text.matches('?').count()
}

/// Starts `skald accept-invite <extra>` on `machine` at a terminal.
pub fn start(machine: &Machine, extra: &[&str]) -> Terminal {
    Terminal::start(machine, &[&[ACCEPT][..], extra].concat())
}

/// Waits until `terminal` asks for the invite link; gives what it showed then.
pub fn asked_for_link(terminal: &mut Terminal) -> String {
    terminal.wait_for("a question for the invite link", |shown| {
        shown.to_lowercase().contains("link")
    })
}

/// A run of `skald accept-invite` at a terminal: what it showed when it asked for the link, when it
/// asked to confirm, and in all, the link taken out, and how it exited.
pub struct Dialog {
    pub asked: String,
    pub summary: String,
    pub shown: Shown,
}

impl Dialog {
    /// What the run showed in all, the link taken out.
    pub fn text(&self) -> &str {
        &self.shown.text
    }
}

/// Runs `skald accept-invite <extra>` on `machine` at a terminal: types `link` when asked, waits
/// for the invite and the question to confirm, answers `confirm`, then answers each of `handles`
/// once it asks for another handle; waits for it to exit.
pub fn accept_at_terminal(
    machine: &Machine,
    link: &str,
    extra: &[&str],
    (confirm, handles): (&str, &[&str]),
) -> Dialog {
    let mut terminal = start(machine, extra);
    let asked = asked_for_link(&mut terminal);
    let before = questions(&terminal.shown());
    terminal.type_line(link);
    let summary = terminal.wait_for("the invite and a question to confirm it", |shown| {
        questions(&shown.replace(link, "")) > before
    });
    let mut asked_handle = mentions(&terminal.shown(), "handle");
    terminal.type_line(confirm);
    for handle in handles {
        terminal.wait_for("a question for another handle", |shown| {
            mentions(shown, "handle") > asked_handle
        });
        asked_handle = mentions(&terminal.shown(), "handle");
        terminal.type_line(handle);
    }
    let mut shown = terminal.finish();
    shown.text = shown.text.replace(link, "");
    Dialog {
        asked,
        summary: summary.replace(link, ""),
        shown,
    }
}

/// Runs `skald accept-invite` on `machine` at a terminal, types `link` when asked and waits for it
/// to exit; should it ask a question after the link, the person ends the terminal's input. Gives
/// what it showed when it asked for the link, and in all, the link taken out.
pub fn refused_at_terminal(machine: &Machine, link: &str) -> Dialog {
    let mut terminal = start(machine, &[]);
    let asked = asked_for_link(&mut terminal);
    let before = questions(&terminal.shown());
    terminal.type_line(link);
    let started = Instant::now();
    let mut ended = false;
    while !terminal.exited() {
        if !ended && questions(&terminal.shown().replace(link, "")) > before {
            terminal.end_input();
            ended = true;
        }
        assert!(
            started.elapsed() < RUN_LIMIT,
            "skald did not exit: {}",
            terminal.shown()
        );
        thread::sleep(Duration::from_millis(50));
    }
    let mut shown = terminal.finish();
    shown.text = shown.text.replace(link, "");
    Dialog {
        asked,
        summary: String::new(),
        shown,
    }
}

/// Fails unless `refused`, a run of `skald accept-invite`, failed and said one of `words` after it
/// asked for the link.
#[track_caller]
pub fn assert_refused_saying(refused: &Dialog, words: &[&str]) {
    assert!(!refused.shown.success, "{}", refused.text());
    assert!(
        (words.iter()).any(|word| says_after(&refused.asked, refused.text(), word)),
        "it does not say why, with one of {words:?}: {}",
        refused.text()
    );
}

/// Waits until `seen` holds, for up to [`WAIT`]; fails then.
#[track_caller]
pub fn wait_until(what: &str, seen: impl Fn() -> bool) {
    let deadline = Instant::now() + WAIT;
    while !seen() {
        assert!(Instant::now() < deadline, "{what} did not happen");
        thread::sleep(Duration::from_millis(50));
    }
}

/// `name` as a handle.
pub fn handle(name: &str) -> Name {
    name.parse().expect("a valid name")
}
