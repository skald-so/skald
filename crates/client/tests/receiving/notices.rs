//! New roster revisions in the CLI (`docs/protocol/delivery.md`, "Frames", and
//! `docs/protocol/membership.md`, "Roster"): on connecting, `skald recv` fetches and accepts the
//! revisions of each space the device follows, and on a `space-notice` those of its space; a notice
//! for a space it does not follow fetches nothing. As an agent, its output names nothing of other
//! spaces: warnings about their revisions wait for the person's next run.
//!
//! As the tests of receiving messages, but the front holds back the `batch` that ends a catch-up
//! until the server sends that connection a `space-notice`, so a revision stored while `skald recv`
//! waits reaches it as a notice; the relay rewrites the revisions the server gives, as a
//! compromised server would.
//!
//! The CLI, as these tests use it, beside the commands of the tests of roster revisions and of
//! receiving messages:
//! - `skald recv` succeeds once catch-up has ended, having accepted the revisions it fetched;
//! - a device keeps a revision it accepted: once it has, another valid revision with the same
//!   number is a forked roster, warned of on a line that says `fork`, and the device keeps its own.
//!   The tests tell that a run accepted a revision this way.

use std::io::{Read as _, Write as _};
use std::net::TcpStream;
use std::path::Path;
use std::thread;
use std::time::{Duration, Instant};

use skald_proto::{Role, Roster, SpaceId};

use crate::cloud::{Cloud, WAIT};
use crate::fixtures::{await_running, fronted};
use crate::front::Front;
use crate::machine::{Finished, Machine, Run};
use crate::world::{
    ACME, ADMIN, AGENT, CLIENT_A, CLIENT_B, Desk, FORK, GLOBEX, MEMBER, Person, assert_role,
    broken_signature, create, devices_request, members, next_by, revisions_request,
    serve_revisions, set_role, spaces, store, warnings,
};
use crate::{CLAUDE, Joining, RECV, assert_shows_once, received};

/// Starts a run with `start`, the end of its catch-up held back until a `space-notice`; once the
/// front holds it, Bob's device stores `revision`. Fails unless the run succeeds and Ada's device
/// got a notice of the revision's space; gives what the run showed and when the revision was
/// stored.
fn stored_while_held(
    front: &Front,
    cloud: &Cloud,
    (ada, bob): (&Person, &Person),
    start: impl FnOnce() -> Run,
    revision: &Roster,
) -> (Finished, Instant) {
    front.hold_end_until_notice(true);
    let mut run = start();
    await_running(&mut run, "catch-up held back", || front.held_ends() > 0);
    let stored_at = cloud.relay.mark();
    store(cloud, &bob.token, revision);
    let finished = run.finish();
    front.hold_end_until_notice(false);
    assert!(finished.success, "{finished:?}");
    let space = revision.body().space;
    assert!(
        front.notices_to(ada.device).contains(&space),
        "no notice of {space}: {finished:?}"
    );
    (finished, stored_at)
}

/// A space of Ada's machine and its revisions, as the server holds them.
struct Followed {
    revisions: Vec<Roster>,
}

impl Followed {
    fn id(&self) -> SpaceId {
        self.revisions[0].body().space
    }

    fn last(&self) -> &Roster {
        self.revisions.last().expect("a revision")
    }
}

/// Ada's machine creates the space `name`, with its default channel in revision 2, whose
/// revision 3, signed by Ada's device and stored, adds Bob as an admin; Ada's machine accepts it.
fn shared_space(cloud: &Cloud, ada: &Person, bob: &Person, name: &str) -> Followed {
    let second = create(cloud, ada, name);
    let third = next_by(&second, ada, |list| list.push(bob.member(Role::Admin)));
    store(cloud, &ada.token, &third);
    let shown = members(&ada.machine, name);
    assert_role(&shown, &bob.handle, ADMIN);
    let held = cloud.revisions(&ada.token, second.body().space);
    Followed {
        revisions: held.into_iter().map(|(_, roster)| roster).collect(),
    }
}

/// The next revision of `space`, signed by Bob's device: Ada an ordinary member.
fn ada_demoted(space: &Followed, ada: &Person, bob: &Person) -> Roster {
    next_by(space.last(), bob, |list| {
        set_role(list, ada.account, Role::Member);
    })
}

/// Fails unless Ada's machine accepted `accepted`, a revision of `space` after its last one that
/// makes Ada an ordinary member: once the relay gives another valid revision with its number, by
/// Bob's device, in which Ada stays an admin, `skald members` warns of a fork and keeps Ada a
/// member. A machine that had not accepted `accepted` would take the other revision without a word.
#[track_caller]
fn assert_accepted(cloud: &Cloud, space: &Followed, ada: &Person, bob: &Person, accepted: &Roster) {
    let other = next_by(space.last(), bob, |_| {});
    assert_eq!(other.body().revision, accepted.body().revision);
    assert_ne!(other.hash(), accepted.hash(), "the same revision");
    let mut given: Vec<&Roster> = space.revisions.iter().collect();
    given.push(&other);
    serve_revisions(cloud, space.id(), &given);

    let shown = members(&ada.machine, &space.revisions[0].body().name.to_string());

    assert!(
        warnings(&shown).iter().any(|line| line.contains(FORK)),
        "no warning of a fork: the revision was not accepted: {shown:?}"
    );
    assert_role(&shown, &ada.handle, MEMBER);
}

/// Ada's `skald recv` waits for the end of its catch-up, which the front holds back, while Bob's
/// device stores a revision of `acme`: the server sends Ada's connection a `space-notice` of
/// `acme`, and the run fetches the space's revisions and accepts the new one.
#[test]
fn connected_device_gets_a_notice_of_a_new_revision_and_accepts_it() {
    let (world, front) = fronted();
    let cloud = &world.cloud;
    let ada = world.sign_up(4_122_081_001, "ada-notified-standin", "ada-workstation");
    let bob = world.sign_up(4_122_081_002, "bob-revises-standin", "bob-workstation");
    let acme = shared_space(cloud, &ada, &bob, ACME);
    let demoted = ada_demoted(&acme, &ada, &bob);

    let (finished, since) = stored_while_held(
        &front,
        cloud,
        (&ada, &bob),
        || ada.machine.start(&[RECV]),
        &demoted,
    );

    assert!(
        cloud.count_sent(since, &revisions_request(acme.id())) > 0,
        "the run fetched no revision after the notice: {finished:?}"
    );
    assert_accepted(cloud, &acme, &ada, &bob, &demoted);
}

/// Bob's device stores a revision of `acme` while Ada's device is offline, so Ada's device gets no
/// notice of it; Ada's next `skald recv` fetches the revisions of each space the device follows,
/// `acme` and `globex`, and accepts the new one, though its mailbox is empty.
#[test]
fn device_offline_when_a_revision_was_stored_accepts_it_on_connecting() {
    let (world, _front) = fronted();
    let cloud = &world.cloud;
    let ada = world.sign_up(4_122_081_101, "ada-offline-standin", "ada-workstation");
    let bob = world.sign_up(4_122_081_102, "bob-online-standin", "bob-workstation");
    let acme = shared_space(cloud, &ada, &bob, ACME);
    let globex = create(cloud, &ada, GLOBEX).body().space;
    let demoted = ada_demoted(&acme, &ada, &bob);
    store(cloud, &bob.token, &demoted);
    let since = cloud.relay.mark();

    let finished = received(&ada.machine);

    for space in [acme.id(), globex] {
        assert!(
            cloud.count_sent(since, &revisions_request(space)) > 0,
            "the run fetched no revision of {space}: {finished:?}"
        );
    }
    assert_accepted(cloud, &acme, &ada, &bob, &demoted);
}

/// Bob's machine creates `globex` and adds Ada's account, but Ada's device follows only the spaces
/// its machine created. A revision of `globex` that Bob's device stores while Ada's `skald recv`
/// waits sends Ada's connection a notice of `globex`; the run fetches nothing of `globex`, as the
/// relay shows, and the device still does not follow it.
#[test]
fn notice_of_a_space_the_device_does_not_follow_fetches_nothing() {
    let (world, front) = fronted();
    let cloud = &world.cloud;
    let ada = world.sign_up(4_122_081_201, "ada-unfollowing-standin", "ada-workstation");
    let bob = world.sign_up(4_122_081_202, "bob-elsewhere-standin", "bob-workstation");
    create(cloud, &ada, ACME);
    let second = create(cloud, &bob, GLOBEX);
    let globex = second.body().space;
    let third = next_by(&second, &bob, |list| list.push(ada.member(Role::Member)));
    store(cloud, &bob.token, &third);
    let fourth = next_by(&third, &bob, |list| {
        set_role(list, ada.account, Role::Admin);
    });

    let (finished, since) = stored_while_held(
        &front,
        cloud,
        (&ada, &bob),
        || ada.machine.start(&[RECV]),
        &fourth,
    );

    for request in [revisions_request(globex), devices_request(globex)] {
        assert_eq!(
            cloud.count_sent(since, &request),
            0,
            "the run sent {request}: {finished:?}"
        );
    }
    let listed = spaces(&ada.machine);
    assert!(
        !listed.shown().to_lowercase().contains(GLOBEX),
        "globex is listed: {listed:?}"
    );
}

/// `skald --agent <CLAUDE> recv` in `dir` on `machine`.
fn agent_run(machine: &Machine, dir: &Path) -> Run {
    machine.start_in(dir, &[AGENT, CLAUDE, RECV])
}

/// As an agent in `~/work/clientA`, `skald recv` waits for the end of its catch-up while Bob's
/// device stores revision 4 of `clientb`, whose revision 3 the device has not accepted yet either;
/// the relay gives revision 4 with its signature broken. The agent's output names nothing of
/// `clientb` and holds no warning; the person's next `skald recv` warns of the broken revision.
#[test]
fn agent_output_names_nothing_of_another_spaces_revisions() {
    let (world, front) = fronted();
    let desk = Desk::sign_up(world, 4_122_081_301, "ada-agent-standin");
    let (cloud, ada) = (&desk.world.cloud, &desk.ada);
    let bob = desk
        .world
        .sign_up(4_122_081_302, "bob-partner-standin", "bob-workstation");
    let client_b = desk.client_b.body().space;
    let third = next_by(&desk.client_b, ada, |list| {
        list.push(bob.member(Role::Admin));
    });
    store(cloud, &ada.token, &third);
    let fourth = next_by(&third, &bob, |list| {
        set_role(list, ada.account, Role::Member);
    });
    let broken = broken_signature(&fourth);
    let (_, first) = cloud.revisions(&ada.token, client_b).remove(0);
    serve_revisions(cloud, client_b, &[&first, &desk.client_b, &third, &broken]);
    let folder = desk.dir("work/clientA");
    desk.add_rule(&folder, CLIENT_A);

    let (agent, _) = stored_while_held(
        &front,
        cloud,
        (ada, &bob),
        || agent_run(&ada.machine, &folder),
        &fourth,
    );

    let shown = agent.shown().to_lowercase();
    for named in [CLIENT_B.to_owned(), client_b.to_string()] {
        assert!(
            !shown.contains(&named.to_lowercase()),
            "the agent's output names {named}: {agent:?}"
        );
    }
    assert_eq!(warnings(&agent), Vec::<String>::new(), "{agent:?}");

    let person = received(&ada.machine);

    assert!(
        !warnings(&person).is_empty(),
        "no warning of the broken revision: {person:?}"
    );
}

/// While the front holds back the `auth` of Ada's `skald recv`, so the server has not taken its
/// connection yet, Ada's device stores a revision that adds Carol's account to the channel, and
/// Carol's device sends Ada's a message there: the run accepts the revision once catch-up has ended
/// and shows the message.
#[test]
fn revision_and_message_stored_while_the_run_connects_reach_it() {
    let (world, front) = fronted();
    let joining = Joining::new(
        &world,
        [4_122_081_401, 4_122_081_402, 4_122_081_403],
        [
            "ada-connecting-standin",
            "bob-present-standin",
            "carol-joining-standin",
        ],
    );
    let ada = &joining.ada;

    front.hold_auth_of(Some(ada.device));
    let mut run = ada.machine.start(&[RECV]);
    await_running(&mut run, "auth held back", || front.held_auths() > 0);
    joining.store(&world.cloud);
    front.hold_auth_of(None);
    let finished = run.finish();

    assert!(finished.success, "{finished:?}");
    assert_shows_once(&finished.shown(), &joining.shown());
}

/// Sends `request`, such as `GET /spaces/<id>/devices`, through the relay without a session, so
/// that a request the relay holds until a second one comes goes on.
fn through_relay(cloud: &Cloud, request: &str) {
    let address = cloud.relay.address();
    let mut stream = TcpStream::connect(address).expect("reach the relay");
    write!(
        stream,
        "{request} HTTP/1.1\r\nhost: {address}\r\nconnection: close\r\n\r\n"
    )
    .expect("send a request to the relay");
    let mut answer = Vec::new();
    let _ = stream.read_to_end(&mut answer);
}

/// Once catch-up has ended, Ada's `skald recv` fetches the revisions of `acme`, then the devices of
/// its accounts, which the relay holds back; meanwhile Bob's device stores a revision, whose
/// `space-notice` reaches the connection. The run fetched the revisions before that one was
/// stored, so it accepts that revision only by its notice.
#[test]
fn revision_stored_after_the_fetch_at_the_end_of_catch_up_is_accepted_by_its_notice() {
    let (world, front) = fronted();
    let cloud = &world.cloud;
    let ada = world.sign_up(4_122_081_501, "ada-late-notice-standin", "ada-workstation");
    let bob = world.sign_up(
        4_122_081_502,
        "bob-late-revision-standin",
        "bob-workstation",
    );
    let acme = shared_space(cloud, &ada, &bob, ACME);
    let demoted = ada_demoted(&acme, &ada, &bob);
    let devices = devices_request(acme.id());
    cloud.relay.hold(&devices, 2, WAIT);

    let mut run = ada.machine.start(&[RECV]);
    await_running(&mut run, "devices request held back", || {
        cloud.relay.held() > 0
    });
    store(cloud, &bob.token, &demoted);
    await_running(&mut run, "notice", || {
        front.notices_to(ada.device).contains(&acme.id())
    });
    // The notice reaches the machine's socket while its request waits.
    thread::sleep(Duration::from_millis(100));
    through_relay(cloud, &devices);
    let finished = run.finish();

    assert!(finished.success, "{finished:?}");
    assert_accepted(cloud, &acme, &ada, &bob, &demoted);
}
