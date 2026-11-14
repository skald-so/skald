//! The account's personal space in the CLI: the account's first device, once the server has taken
//! its certificate, gets the account's personal space from the server and stores its revision 1,
//! signed by the device, as `skald create-space` does with a team space; a device of an account the
//! server has no personal space of yet does so on its next run. The device pins its personal space
//! when it signs that revision 1 and never takes another from the server.
//!
//! Against a `skald-server` with GitHub sign-in through the GitHub stand-in, on PostgreSQL, behind
//! the recording relay, which can lose the server's answers, rewrite them, as a compromised server
//! would, and answer a request itself, as a server without personal spaces would; a test plays the
//! person on the verification page in headless Chrome. Each machine keeps its state in a directory
//! of its own and its secrets in a file there.
//!
//! The API and the CLI, as these tests use them, beside `skald create-space <name>` and
//! `skald members <space>`:
//! - `POST /account/personal-space`, without a body: `{"space": <ID>, "name": <name>, ...}`, the
//!   session's account's personal space, which the first request creates; `201` when it creates it,
//!   `200` after. Its name is the account's username, numbered when another space has that name;
//! - `skald sign-in` of the account's first device gets the personal space after the server has
//!   taken the device's certificate; any later run, such as `skald spaces`, finishes what an earlier
//!   one left undone: it gets the personal space and stores or resends its revision 1;
//! - `skald spaces` lists the spaces the device follows; the line of its pinned personal space also
//!   holds the word `personal`, and no other line does.

// Each test target uses a part of the shared modules.
#![allow(dead_code)]

#[path = "../sign_in/relay.rs"]
mod relay;
#[path = "../roster_revisions/world.rs"]
mod world;

#[path = "../../../server/tests/common/browser.rs"]
mod browser;
#[path = "../sign_in/cloud.rs"]
mod cloud;
#[path = "../common/mod.rs"]
mod common;
#[path = "../device_keys/keys.rs"]
mod keys;
#[path = "../sign_in/machine.rs"]
mod machine;

use std::collections::BTreeMap;
use std::thread;
use std::time::Duration;

use browser::APPROVE;
use cloud::{Cloud, GIVE_MASTER, shown_grant};
use machine::{FIRST_DEVICE, Machine, NO, SERVER, SIGN_IN, YES};
use relay::Lose;
use skald_crypto::{check_device, check_roster};
use skald_proto::{Cbor as _, ROSTER_LABEL, RosterBody, SpaceId};
use world::{
    ACME, ADMIN, PERSONAL_REQUEST, Person, STORE, World, assert_role, create, give_as_personal,
    mark_as_personal, members, plain_lines, run, space_certs, spaces, words,
};

/// The word that marks the personal space in the space list.
const PERSONAL: &str = "personal";

/// The spaces the account of the session `token` gets, straight from the server: each one's name
/// by its ID.
fn server_spaces(cloud: &Cloud, token: &str) -> BTreeMap<SpaceId, String> {
    cloud.spaces(token).into_iter().collect()
}

/// Fails unless the server gives `person`'s account one space named `name`, holding one revision:
/// a new space's revision 1, signed by `person`'s machine, with the account the only member and
/// admin, with its username as handle and its master key, which passes checks 3 and 4. Gives the
/// space and the revision's bytes.
fn assert_personal(cloud: &Cloud, person: &Person, name: &str) -> (SpaceId, Vec<u8>) {
    let spaces = server_spaces(cloud, &person.token);
    let named: Vec<SpaceId> = spaces
        .iter()
        .filter_map(|(id, listed)| (listed == name).then_some(*id))
        .collect();
    assert_eq!(named.len(), 1, "one space {name}: {spaces:?}");
    let space = named[0];
    let mut revisions = cloud.revisions(&person.token, space);
    assert_eq!(revisions.len(), 1, "{name}'s revisions: {revisions:?}");
    let (bytes, roster) = revisions.remove(0);
    assert_eq!(roster.to_cbor(), bytes, "the revision re-encodes otherwise");
    let expected = RosterBody::first(
        space,
        name.parse().expect("a valid name"),
        person.account,
        person.handle.clone(),
        person.master,
        person.device,
        roster.body().timestamp,
    );
    assert_eq!(roster.body(), &expected, "not the machine's revision 1");
    let certs = space_certs(cloud, &person.token, space);
    let signer = check_device(&person.master, &certs, person.device)
        .unwrap_or_else(|err| panic!("the machine has no valid certificate: {err}"));
    let signed = [roster.body_bytes()];
    skald_crypto::verify(
        &signer.body().signing,
        ROSTER_LABEL,
        &signed,
        roster.signature(),
    )
    .expect("the machine's signature");
    check_roster(&roster, &certs).expect("revision 1 passes checks 3 and 4");
    (space, bytes)
}

/// Fails unless `machine`'s space list marks one line as its personal space: the line of `name`,
/// with the role `admin` and `person`'s handle.
fn assert_marked(machine: &Machine, name: &str, person: &Person) {
    let listed = spaces(machine);
    let marked: Vec<String> = plain_lines(&listed)
        .into_iter()
        .filter(|line| words(line).contains(&PERSONAL))
        .collect();
    assert_eq!(marked.len(), 1, "one personal space: {listed:?}");
    let marked = words(&marked[0]);
    for word in [name, ADMIN, person.handle.as_str()] {
        assert!(marked.contains(&word), "no {word} in the mark: {listed:?}");
    }
}

/// The account's first machine, at its sign-in, once the server has taken its certificate, gets
/// its personal space, named its username, and signs its revision 1 as its admin. It lists the
/// space as personal, and its members show the machine among the account's devices. The account's
/// next machine, not linked, signs nothing and marks no space as personal.
#[test]
fn new_accounts_first_machine_is_in_its_personal_space_as_its_admin() {
    let world = World::start();
    let cloud = &world.cloud;
    let ada = world.sign_up(4_120_701_001, "ada-personal-standin", "ada-workstation");

    // The sign-in's requests: it asked for the space after the server took the certificate.
    let sent: Vec<String> = cloud.relay.sent().into_iter().map(|one| one.head).collect();
    let certified = sent
        .iter()
        .rposition(|head| head.starts_with(&GIVE_MASTER.to_lowercase()));
    let asked = sent
        .iter()
        .position(|head| head.starts_with(&PERSONAL_REQUEST.to_lowercase()));
    assert!(asked.is_some(), "the sign-in asked for no personal space");
    assert!(asked > certified, "asked before the certificate was taken");
    let name = ada.handle.as_str();
    let (space, _) = assert_personal(cloud, &ada, name);
    assert_eq!(server_spaces(cloud, &ada.token).len(), 1, "one space");

    assert_marked(&ada.machine, name, &ada);
    let shown = members(&ada.machine, name);
    assert_role(&shown, &ada.handle, ADMIN);
    assert!(
        shown.stdout.contains(&ada.device.to_string()),
        "the machine is not among the account's devices: {shown:?}"
    );

    let laptop = Machine::new().with_host_name("ada-second-laptop");
    let page = world.browser.page();
    let waiting = cloud.signed_in(&page, &laptop, &ada.user, &[FIRST_DEVICE, NO]);
    assert!(waiting.success, "the second machine: {waiting:?}");
    let listed = spaces(&laptop);
    assert!(
        !plain_lines(&listed)
            .iter()
            .any(|line| words(line).contains(&PERSONAL)),
        "{listed:?}"
    );
    assert_eq!(server_spaces(cloud, &ada.token).len(), 1, "one space");
    assert_eq!(cloud.revisions(&ada.token, space).len(), 1, "one revision");
}

/// The relay loses the server's answer to `request` during the first machine's sign-in, as `how`
/// says: it closes the connection, or it withholds the answer and the test kills the CLI. The next
/// run, `skald spaces`, leaves one personal space, the one the server created first, with one
/// revision 1: if the server took one, that one, byte for byte; and lists it as personal.
fn recovers_from_a_lost_answer(request: &str, how: Lose, user: (u64, &str)) {
    let world = World::start();
    let cloud = &world.cloud;
    let ada = cloud.user(user.0, user.1);
    let machine = Machine::new().with_host_name("ada-flaky-network");
    cloud.relay.lose_answer(request, how);

    let mut signing_in = machine.start(&[SIGN_IN, SERVER, cloud.url.as_str(), FIRST_DEVICE, YES]);
    let grant = shown_grant(cloud, &mut signing_in, 0);
    cloud.decide(&world.browser.page(), &ada, &grant, APPROVE);
    match how {
        // Its result does not matter.
        Lose::Close => drop(signing_in.finish()),
        Lose::Withhold => cloud.kill_once_lost(signing_in, request),
    }
    assert_eq!(cloud.relay.lost(), 1, "the server never answered {request}");
    let ada = world.person(ada, machine, "ada-flaky-network");
    let created: Vec<SpaceId> = server_spaces(cloud, &ada.token).into_keys().collect();
    assert_eq!(created.len(), 1, "the server created no personal space");
    let taken = cloud.revisions(&ada.token, created[0]);
    if request == STORE {
        assert_eq!(taken.len(), 1, "the server took no revision 1");
    }
    // A revision signed again now has another timestamp, and so other bytes.
    thread::sleep(Duration::from_millis(1100));

    spaces(&ada.machine);

    let since = cloud.relay.sent().first().expect("requests").at;
    assert!(
        cloud.count_sent(since, request) >= 2,
        "{request} was not sent again"
    );
    let (space, bytes) = assert_personal(cloud, &ada, ada.handle.as_str());
    assert_eq!(space, created[0], "another space");
    assert_eq!(server_spaces(cloud, &ada.token).len(), 1, "one space");
    if let Some((first, _)) = taken.first() {
        assert_eq!(&bytes, first, "another revision 1 than the one taken");
    }
    assert_marked(&ada.machine, ada.handle.as_str(), &ada);
}

#[test]
fn personal_space_answer_dropped_by_the_proxy_is_recovered_by_the_next_run() {
    recovers_from_a_lost_answer(
        PERSONAL_REQUEST,
        Lose::Close,
        (4_120_701_101, "ada-lost-personal"),
    );
}

#[test]
fn personal_space_answer_lost_with_the_cli_killed_is_recovered_by_the_next_run() {
    recovers_from_a_lost_answer(
        PERSONAL_REQUEST,
        Lose::Withhold,
        (4_120_701_102, "ada-killed-personal"),
    );
}

#[test]
fn personal_revision_1_answer_dropped_by_the_proxy_is_recovered_by_the_next_run() {
    recovers_from_a_lost_answer(STORE, Lose::Close, (4_120_701_103, "ada-lost-first"));
}

#[test]
fn personal_revision_1_answer_lost_with_the_cli_killed_is_recovered_by_the_next_run() {
    recovers_from_a_lost_answer(STORE, Lose::Withhold, (4_120_701_104, "ada-killed-first"));
}

/// An account whose first machine signed in while the server had no personal spaces, as one
/// created before them: the server answers the request for the personal space as a server without
/// it would, so the account has none. The machine's next run gets the personal space, signs its
/// revision 1 and lists it as personal.
#[test]
fn existing_accounts_machine_signs_revision_1_of_its_personal_space_on_its_next_run() {
    let world = World::start();
    let cloud = &world.cloud;
    cloud.relay.refuse(PERSONAL_REQUEST, "404 Not Found");
    let user = cloud.user(4_120_701_201, "ada-existing-standin");
    let host = "ada-old-workstation";
    // Its result does not matter.
    let (machine, _) = cloud.first_machine(&world.browser.page(), &user, host);
    let ada = world.person(user, machine, host);
    assert_eq!(server_spaces(cloud, &ada.token), BTreeMap::new());
    cloud.relay.stop_refusing();
    let since = cloud.relay.mark();

    spaces(&ada.machine);

    assert!(
        cloud.count_sent(since, PERSONAL_REQUEST) >= 1,
        "no personal space was asked for"
    );
    let name = ada.handle.as_str();
    assert_personal(cloud, &ada, name);
    assert_eq!(server_spaces(cloud, &ada.token).len(), 1, "one space");
    assert_marked(&ada.machine, name, &ada);
}

/// What a run says when it could not set up the account's personal space.
const NOT_SET_UP: &str = "the account's personal space is not set up yet";

/// The first machine's sign-in says it signed revision 1 of the account's personal space, with the
/// space's name and ID and the machine's handle.
#[test]
fn first_machines_sign_in_says_it_signed_revision_1_of_the_personal_space() {
    let world = World::start();
    let cloud = &world.cloud;
    let user = cloud.user(4_120_701_501, "ada-said-standin");
    let host = "ada-workstation";
    let (machine, signed_in) = cloud.first_machine(&world.browser.page(), &user, host);
    assert!(signed_in.success, "{signed_in:?}");
    let ada = world.person(user, machine, host);
    let (space, _) = assert_personal(cloud, &ada, ada.handle.as_str());

    let said = format!(
        "This device signed revision 1 of the account's personal space {} ({space}) as its admin, \
         with the handle {}.",
        ada.handle.as_str(),
        ada.handle.as_str()
    );
    assert!(signed_in.stdout.contains(&said), "{signed_in:?}");
    assert!(!signed_in.stderr.contains(NOT_SET_UP), "{signed_in:?}");
}

/// A machine of an existing account, without a personal space, creates a team space while the
/// server answers the request for the personal space as a server without it would: both runs warn
/// that the personal space is not set up. Its next run gets the personal space, signs its
/// revision 1 and lists it as personal, and leaves the team space unmarked.
#[test]
fn machine_following_a_team_space_signs_revision_1_of_its_personal_space_on_its_next_run() {
    let world = World::start();
    let cloud = &world.cloud;
    cloud.relay.refuse(PERSONAL_REQUEST, "404 Not Found");
    let user = cloud.user(4_120_701_601, "ada-team-standin");
    let host = "ada-old-workstation";
    let (machine, signed_in) = cloud.first_machine(&world.browser.page(), &user, host);
    assert!(signed_in.stderr.contains(NOT_SET_UP), "{signed_in:?}");
    let ada = world.person(user, machine, host);
    let created = run(&ada.machine, &[world::CREATE_SPACE, ACME]);
    assert!(created.success, "{created:?}");
    assert!(created.stderr.contains(NOT_SET_UP), "{created:?}");
    let team: Vec<String> = server_spaces(cloud, &ada.token).into_values().collect();
    assert_eq!(team, vec![ACME.to_owned()]);
    cloud.relay.stop_refusing();

    spaces(&ada.machine);

    let name = ada.handle.as_str();
    assert_personal(cloud, &ada, name);
    assert_eq!(server_spaces(cloud, &ada.token).len(), 2, "two spaces");
    assert_marked(&ada.machine, name, &ada);
    let listed = spaces(&ada.machine);
    let lines = plain_lines(&listed);
    let team: Vec<Vec<&str>> = lines
        .iter()
        .map(|line| words(line))
        .filter(|words| words.contains(&ACME))
        .collect();
    assert_eq!(team.len(), 1, "{listed:?}");
    assert!(!team[0].contains(&PERSONAL), "{listed:?}");
}

/// After the machine pinned its personal space, the proxy gives another space for it, one of the
/// account's or a made-up one, and marks another space as personal in the account's spaces, as a
/// compromised server would: the list's mark stays on the pinned space, and the machine signs no
/// revision for the space the proxy gives.
#[test]
fn proxy_giving_another_personal_space_does_not_move_the_lists_mark() {
    let world = World::start();
    let cloud = &world.cloud;
    let ada = world.sign_up(4_120_701_301, "ada-pinned-standin", "ada-workstation");
    let name = ada.handle.as_str();
    let (personal, first) = assert_personal(cloud, &ada, name);
    let acme = create(cloud, &ada, ACME).body().space;
    let made_up = SpaceId::generate();

    for (other, other_name) in [(acme, ACME), (made_up, "globex")] {
        give_as_personal(cloud, other, other_name);
        mark_as_personal(cloud, other);
        let since = cloud.relay.mark();

        assert_marked(&ada.machine, name, &ada);
        let listed = spaces(&ada.machine);
        let line = plain_lines(&listed)
            .into_iter()
            .find(|line| words(line).contains(&ACME));
        let line = line.unwrap_or_else(|| panic!("{ACME} is not listed: {listed:?}"));
        assert!(!words(&line).contains(&PERSONAL), "{other_name}: {line:?}");
        let store = format!("POST /spaces/{other}/revisions");
        assert_eq!(cloud.count_sent(since, &store), 0, "{other_name}: sent");
    }
    assert_eq!(cloud.revisions(&ada.token, personal)[0].0, first);
    assert_eq!(cloud.revisions(&ada.token, personal).len(), 1);
    assert_eq!(cloud.revisions(&ada.token, acme).len(), 1);
}

/// When another account's space has the name the account's username gives, the personal space has
/// the numbered name the server gives it, and the machine lists it as personal under that name.
#[test]
fn personal_space_takes_the_numbered_name_when_another_space_has_the_username() {
    let world = World::start();
    let cloud = &world.cloud;
    let bob = world.sign_up(4_120_701_401, "bob-owner-standin", "bob-workstation");
    create(cloud, &bob, "ada-taken-standin");

    let ada = world.sign_up(4_120_701_402, "ada-taken-standin", "ada-workstation");

    assert_eq!(ada.handle.as_str(), "ada-taken-standin");
    let name = "ada-taken-standin-2";
    assert_personal(cloud, &ada, name);
    assert_marked(&ada.machine, name, &ada);
}
