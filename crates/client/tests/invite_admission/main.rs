//! Admitting newcomers on an admin's device (`docs/protocol/membership.md`, "Invite", "Roster" and
//! "Group changes"; `architecture.md`, "Membership"). An admin's `client` device takes the waiting
//! acceptances of its spaces on each run that connects and on a `space-notice`, as the person or as
//! an agent, one-shot or through the helper. It admits a newcomer only after checks 1–4 of
//! "Invite": the invite signed by an admin's device in the latest revision and not expired, the
//! acceptance signed with the invite's `key`, the account not a member and the handle free, and the
//! newcomer's devices checked against the acceptance's `master`. It then signs the next revision
//! with the member, its account, handle, the invite's role, that `master` and `invite`, and the
//! newcomer in the default channel with `history` true and in the invite's other channels with
//! their `history`. It writes the revision before sending it, resends it, and applies its change to
//! a revision that came first; an acceptance a revision already admitted is done. It tells its user
//! which handle it admitted to which space; an acceptance that fails a check it does not admit, and
//! warns its user with the handle and the check. As an agent, what it says of another space waits
//! for the person's run. The newcomer's devices get into the channels as a new channel member's do:
//! the signing device adds those with a valid certificate and a KeyPackage, and the others join by
//! themselves.
//!
//! As the tests of channel messages (`channel_messages/main.rs`): the CLI under test runs on Ada's
//! machine, which created the space; the devices of Carol, a second admin, of Dan, a member who is
//! no admin, and of Bob, the newcomer of another account, are played by the tests with an MLS state
//! of their own. The invites are made as Ada's device, and the acceptances as the newcomer's, with
//! `skald-crypto`, apart from the code under test, and issued, stored and accepted through the
//! server's API. A `skald-server` with GitHub sign-in through the stand-in, behind the front and
//! the recording relay of the tests of sending, which the tests use as a proxy that sees, loses,
//! holds and changes what goes through. What a compromised server gives beside that, the tests
//! write to its database: an invite's bytes replaced by another one signed with a device's key the
//! tests hold, and a device on Bob's account whose certificate is chained to another master key.
//!
//! The CLI, as these tests use it, beside the commands of the tests of spaces, channels, sending,
//! receiving and joining channels:
//! - `skald recv`, a run that connects, takes the waiting acceptances of each space in which its
//!   account is an admin, and admits each newcomer whose acceptance passes the checks, in the same
//!   run; it succeeds also when it admits no one. `skald recv --wait`, one-shot or through the
//!   helper, does so on a `space-notice` of an acceptance, with no other command.
//! - A run that admits a newcomer shows a line that is no warning holding, as words, the handle it
//!   admitted and the space's name.
//! - An acceptance it does not admit gives a warning, a line saying `warning`, that holds the handle
//!   the acceptance asks for, as a word, and names the check that failed: `signature` or `key` for
//!   an acceptance that does not verify with the invite's `key`; `expired` for an invite past its
//!   `expires`; `admin` for an invite whose signer is no admin's device; `handle` for a handle a
//!   member or a service agent owner holds; `already` or `member` for an account already a member.
//! - A device that joins a channel by itself that the members do not admit gives a warning line
//!   holding its device ID, as in the tests of joining channels.
//! - Run with `--agent <name>` in a folder bound to another space, its output names nothing of the
//!   space it admits to: not its name or ID, its channels' IDs, the handle it admits or the handle a
//!   warning holds.
//!
//! The server's API, as these tests use it, beside that of the tests of roster revisions and
//! channel groups: `POST /spaces/{space}/invites`, which issues an invite ID;
//! `PUT /spaces/{space}/invites/{invite}`, which stores an invite;
//! `PUT /invites/{invite}/acceptance`, which stores an acceptance; and
//! `GET /spaces/{space}/acceptances`, whose answer the relay changes.

// Each test target uses a part of the shared modules.
#![allow(dead_code)]

mod admitting;
mod agent;
mod devices;
mod forged;
mod once;
mod refusals;
mod reuse;
mod waiting;

// The modules the tests of the channel history share (`history_common/mod.rs`), and the runs
// through a helper of the tests of the helper.
#[path = "../history_common/mod.rs"]
mod history_common;
use history_common::*;
#[path = "../helper/helped.rs"]
mod helped;
#[path = "../invite_fixtures/signed.rs"]
mod signed_invite;

use std::collections::BTreeSet;

use serde_json::{Value, json};
use skald_crypto::{InviteSecret, SigningKey, accept_next, sign_acceptance, sign_invite};
use skald_proto::{
    Acceptance, AcceptanceBody, Cbor as _, ChannelId, ChannelMember, DeviceId, Invite, InviteBody,
    InviteId, Member, NonEmpty, Role, Roster, RosterBody, SpaceId, Unknown,
};

use cloud::{Cloud, unb64};
use edits::in_order;
use fixtures::{fronted, hex};
use front::Front;
use kit::{Space, b64, create_space, received, rosters, with_channel};
use machine::Finished;
use played::Played;
use talk::{ADA_HOST, BOB_HOST, CAROL_HOST};
use world::{Person, World, plain_lines, space_certs, warnings, words};

/// The host name of Dan's machine, its certificate's `name`.
pub const DAN_HOST: &str = "dan-desktop";

/// Ada, whose machine runs the CLI and created the space; Carol, a second admin, and Dan, if the
/// scene has him, a member who is no admin, both in the default channel and in a second channel
/// with Ada, by revision 3; and Bob, the newcomer, the first device of another account, in no
/// space. Carol's, Dan's and Bob's devices are played, each with KeyPackages on the server; Ada's
/// machine has added Carol's and Dan's devices to both channels' groups.
pub struct Scene {
    pub world: World,
    pub front: Front,
    pub ada: Person,
    pub carol: Played,
    pub dan: Option<Played>,
    pub bob: Played,
    pub space: Space,
    /// The channel beside the default one.
    pub channel: ChannelId,
}

impl Scene {
    /// The scene in a space named `name`: Ada is GitHub user `people[0]`, Carol `people[1]`, Bob
    /// `people[2]` and Dan, if given, `people[3]`, each `(id, login)`.
    pub fn new(name: &str, people: &[(u64, &str)]) -> Self {
        let (world, front) = fronted();
        let cloud = &world.cloud;
        let ada = world.sign_up(people[0].0, people[0].1, ADA_HOST);
        let played =
            |(id, login): (u64, &str), host| Played::new(cloud, world.sign_up(id, login, host));
        let mut carol = played(people[1], CAROL_HOST);
        let bob = played(people[2], BOB_HOST);
        let mut dan = people.get(3).map(|person| played(*person, DAN_HOST));
        let space = create_space(cloud, &ada, name);
        let mut added = vec![(&carol.person, Role::Admin)];
        added.extend(dan.iter().map(|dan| (&dan.person, Role::Member)));
        let channel = with_channel(cloud, space.id, &ada, &added);
        received(&ada.machine);
        for one in std::iter::once(&mut carol).chain(dan.iter_mut()) {
            one.take_in(cloud);
            for group in [space.default, channel] {
                assert!(
                    !one.joined_to(group).is_empty(),
                    "{} is not in {group}",
                    one.person.handle
                );
            }
        }
        Scene {
            world,
            front,
            ada,
            carol,
            dan,
            bob,
            space,
            channel,
        }
    }

    pub fn cloud(&self) -> &Cloud {
        &self.world.cloud
    }

    /// `(space, channel)` of each of the space's two channels: the default one first.
    pub fn both(&self) -> [(SpaceId, ChannelId); 2] {
        [
            (self.space.id, self.space.default),
            (self.space.id, self.channel),
        ]
    }

    /// The revisions of the space the server holds, as Ada's session gets them.
    pub fn rosters(&self) -> Vec<Roster> {
        rosters(self.cloud(), &self.ada.token, self.space.id)
    }

    /// The last revision of the space the server holds.
    pub fn latest(&self) -> Roster {
        self.rosters().pop().expect("a revision")
    }

    /// An invite for a member to the default channel with `history` true and to the second channel
    /// with `history` false, expiring 7 days ahead, signed by Ada's device, issued and stored
    /// through the API.
    pub fn invite(&self) -> Made {
        self.invite_with(|_| {})
    }

    /// As [`Scene::invite`], with `change` made to the invite's body before it is signed.
    pub fn invite_with(&self, change: impl FnOnce(&mut InviteBody)) -> Made {
        let (cloud, ada) = (self.cloud(), &self.ada);
        let secret = InviteSecret::generate();
        let id = signed_invite::issue(cloud, &ada.token, self.space.id);
        let invite = signed_invite::sign_and_store(
            cloud,
            ada,
            (self.space.id, id),
            &secret,
            vec![(self.space.default, true), (self.channel, false)],
            change,
        );
        Made { invite, secret }
    }

    /// Bob accepts `made` with his username as his handle, through the API.
    #[track_caller]
    pub fn bob_accepts(&self, made: &Made) {
        made.accepted_by(
            self.cloud(),
            &self.bob.person,
            self.bob.person.handle.as_str(),
        );
    }

    /// Bob's device takes in its mailbox; fails unless it then holds a Welcome from Ada's device to
    /// each of the space's two channels.
    #[track_caller]
    pub fn assert_bob_welcomed(&mut self) {
        self.bob.take_in(&self.world.cloud);
        for (_, channel) in self.both() {
            let welcomes = self.bob.joined_to(channel);
            assert!(
                (welcomes.iter()).any(|joining| joining.signer.device == self.ada.device),
                "Bob's device got no Welcome to {channel} from Ada's: {welcomes:?}"
            );
        }
    }

    /// Fails unless the last revision the server holds is the one after `previous` that the device
    /// `signer` signed to admit Bob with `made` and his username, as [`assert_admits`] says.
    #[track_caller]
    pub fn assert_bob_admitted(&self, previous: &Roster, signer: DeviceId, made: &Made) {
        let all = self.rosters();
        assert_eq!(
            all.len(),
            previous.body().revision as usize + 1,
            "not one more revision"
        );
        let bob = &self.bob.person;
        let newcomer = Newcomer {
            person: bob,
            handle: bob.handle.as_str(),
        };
        assert_admits(
            previous,
            all.last().expect("a revision"),
            (signer, made),
            &newcomer,
        );
        let certs = space_certs(self.cloud(), &self.ada.token, self.space.id);
        let last = all.last().expect("a revision");
        accept_next(previous, last, &certs, self.ada.account, &self.ada.master)
            .unwrap_or_else(|err| panic!("Ada's device does not accept the revision: {err}"));
    }
}

/// An invite the tests made, and its secret.
pub struct Made {
    pub invite: Invite,
    pub secret: InviteSecret,
}

impl Made {
    pub fn id(&self) -> InviteId {
        self.invite.body().invite
    }

    /// The acceptance of the invite for `person`'s account, its master key and `handle`, signed
    /// with `key`.
    pub fn acceptance(&self, person: &Person, handle: &str, key: &SigningKey) -> Acceptance {
        let body = AcceptanceBody {
            invite: self.id(),
            account: person.account,
            master: person.master,
            handle: handle.parse().expect("a valid name"),
            unknown: Unknown::default(),
        };
        sign_acceptance(&body, key).expect("an acceptance")
    }

    /// `person` accepts the invite for its account, its master key and `handle`, signed with the
    /// invite key, through the API as its session; fails unless the server stores it.
    #[track_caller]
    pub fn accepted_by(&self, cloud: &Cloud, person: &Person, handle: &str) {
        let acceptance = self.acceptance(person, handle, &self.secret.invite_key());
        let request = json!({ "acceptance": b64(&acceptance.to_cbor()) });
        let path = format!("/invites/{}/acceptance", self.id());
        let (status, _) = world::put_as(cloud, &person.token, &path, &request);
        assert_eq!(status, 201, "the acceptance is not stored");
    }

    /// The server's database holds, in place of the invite, the one `change` makes of its body,
    /// signed with `key`, as a compromised server could give it.
    pub fn replaced_in_database(
        &self,
        cloud: &Cloud,
        key: &SigningKey,
        change: impl FnOnce(&mut InviteBody),
    ) {
        let mut body = self.invite.body().clone();
        change(&mut body);
        let replaced = sign_invite(&body, key).expect("an invite");
        cloud.execute(&format!(
            "UPDATE invites SET invite = '\\x{}'::bytea WHERE id = '{}'::uuid",
            hex(&replaced.to_cbor()),
            self.id()
        ));
    }
}

/// A newcomer as a revision admits it: its person and the handle it asked for.
pub struct Newcomer<'a> {
    pub person: &'a Person,
    pub handle: &'a str,
}

/// Fails unless `next` is the revision after `previous`, signed by the device `signer`, whose one
/// change is [`admit`] of the newcomer with `made`.
#[track_caller]
pub fn assert_admits(
    previous: &Roster,
    next: &Roster,
    (signer, made): (DeviceId, &Made),
    newcomer: &Newcomer,
) {
    let mut expected = previous.body().clone();
    admit(&mut expected, made, newcomer);
    expected.revision += 1;
    expected.prev = Some(previous.hash());
    expected.signer = signer;
    expected.timestamp = next.body().timestamp;
    assert_eq!(
        in_order(next.body()),
        in_order(&expected),
        "not the next revision, signed by {signer}, that admits the newcomer alone as the invite says"
    );
}

/// Adds to `body` the newcomer of `made`: a member with its account, `handle`, the invite's role,
/// its master key and the invite; in the first channel, the default one, with `history` true; and
/// in each other channel of the invite that `body` has, with the invite's `history`.
pub fn admit(body: &mut RosterBody, made: &Made, newcomer: &Newcomer) {
    let invite = made.invite.body();
    let account = newcomer.person.account;
    let mut members = body.members.clone().into_vec();
    members.push(Member {
        account,
        handle: newcomer.handle.parse().expect("a valid name"),
        role: invite.role,
        master: newcomer.person.master,
        invite: Some(invite.invite),
        excluded: None,
        unknown: Unknown::default(),
    });
    body.members = NonEmpty::from_vec(members).expect("a member");
    for (at, channel) in body.channels.iter_mut().enumerate() {
        let named = (invite.channels.iter()).find(|(id, _)| *id == channel.id);
        let history = if at == 0 {
            Some(true)
        } else {
            named.map(|(_, history)| *history)
        };
        if let Some(history) = history {
            channel.members.push(ChannelMember { account, history });
        }
    }
}

/// The revision after `previous`, signed by `signer`'s device, that admits the newcomer with
/// `made` as [`admit`] does.
pub fn admitting(previous: &Roster, signer: &Person, made: &Made, newcomer: &Newcomer) -> Roster {
    world::next_with(previous, (signer.device, &signer.key), |body| {
        admit(body, made, newcomer);
    })
}

/// Whether `roster` lists `person`'s account as a member.
pub fn lists(roster: &Roster, person: &Person) -> bool {
    roster.body().member(person.account).is_some()
}

/// The request for `space`'s waiting acceptances, as its line starts.
pub fn acceptances_request(space: SpaceId) -> String {
    format!("GET /spaces/{space}/acceptances")
}

/// From now on the relay gives each waiting acceptance of `space` signed with `key` in place of the
/// one the server holds, as a compromised server would.
pub fn resign_acceptances(cloud: &Cloud, space: SpaceId, key: SigningKey) {
    cloud
        .relay
        .rewrite(&acceptances_request(space), move |answer| {
            let mut given: Value = serde_json::from_str(answer).unwrap_or(Value::Null);
            for entry in given["acceptances"].as_array_mut().into_iter().flatten() {
                let Some(held) = entry["acceptance"].as_str() else {
                    continue;
                };
                let held = Acceptance::from_cbor(&unb64(held)).expect("an acceptance");
                let forged = sign_acceptance(held.body(), &key).expect("an acceptance");
                entry["acceptance"] = json!(b64(&forged.to_cbor()));
            }
            given.to_string()
        });
}

/// Fails unless `finished` shows a line that is no warning holding the words `handle` and `space`.
#[track_caller]
pub fn assert_told(finished: &Finished, handle: &str, space: &str) {
    let told = plain_lines(finished).into_iter().any(|line| {
        let held = words(&line);
        held.contains(&handle) && held.contains(&space)
    });
    assert!(
        told,
        "it does not say it admitted {handle} to {space}: {finished:?}"
    );
}

/// Fails unless `finished` shows a warning holding the word `handle` and one of `checks`.
#[track_caller]
pub fn assert_warned(finished: &Finished, handle: &str, checks: &[&str]) {
    let handle = handle.to_lowercase();
    let warned = warnings(finished).into_iter().any(|line| {
        words(&line).contains(&handle.as_str()) && checks.iter().any(|check| line.contains(check))
    });
    assert!(
        warned,
        "no warning holding {handle} and one of {checks:?}: {finished:?}"
    );
}

/// The member devices of `at`'s channel's group, as the server's GroupInfo shows it to `token`.
pub fn group_members(
    cloud: &Cloud,
    token: &str,
    (space, channel): (SpaceId, ChannelId),
) -> BTreeSet<DeviceId> {
    let group = kit::group(cloud, token, space, channel);
    group.members.into_iter().collect()
}
