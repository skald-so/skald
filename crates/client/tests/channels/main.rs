//! Creating channels in the CLI: an admin's device gets a channel ID from the server and stores the
//! space's next roster revision with the channel, its own account the channel's member with
//! `history` true (`docs/protocol/membership.md`, "Roster" and "Creating a channel"). When the
//! server refuses the revision because another came first, the device accepts the new revisions,
//! applies its change to the latest and signs again; when the latest makes the change impossible,
//! it stops and says why. The device writes the revision before sending it and resends it until the
//! server has taken it; only then does the revision count as accepted.
//!
//! Against a `skald-server` with GitHub sign-in through the GitHub stand-in, on PostgreSQL, behind
//! the recording relay, which can lose the server's answers and rewrite them; a test plays the
//! person on the verification page in headless Chrome. Each machine keeps its state in a directory
//! of its own and its secrets in the 0600 file there. The tests sign revisions of other admins with
//! the keys of their machines' devices, read from their secrets files, and store them on the server
//! through the API, as those devices would.
//!
//! The CLI, as these tests use it, beside `skald create-space <name>` and `skald members <space>`:
//! - `skald create-channel <space>`: on a device of an admin of the space named `<space>`, adds a
//!   channel to it by a revision the device signs, and says the new channel's ID; on a device of a
//!   member that is no admin, or when the latest revision makes it no admin, it fails saying why,
//!   in a message that says `admin`, and sends nothing more;
//! - `skald channels <space>`: lists the channels of the space named `<space>` as its last accepted
//!   revision gives them, one line per channel holding the channel's ID and the handles of its
//!   members;
//! - any command that uses a space first resends a revision of it that the server has not taken; a
//!   resend the server fails to store leaves the space at its last accepted revision, and the
//!   command goes on with it.

// Each test target uses a part of the shared modules.
#![allow(dead_code)]

#[path = "../../../server/tests/common/browser.rs"]
mod browser;
#[path = "../sign_in/cloud.rs"]
mod cloud;
#[path = "../roster_revisions/world.rs"]
mod world;

#[path = "../common/mod.rs"]
mod common;
#[path = "../device_keys/keys.rs"]
mod keys;
#[path = "../sign_in/machine.rs"]
mod machine;
#[path = "../sign_in/relay.rs"]
mod relay;

use std::sync::atomic::{AtomicUsize, Ordering};
use std::thread;
use std::time::{Duration, Instant};

use cloud::Cloud;
use machine::{Finished, Machine};
use relay::{Lose, strings_named};
use serde_json::{Value, json};
use skald_crypto::accept_next;
use skald_proto::{Channel, ChannelId, ChannelMember, Name, Role, Roster, SpaceId, Unknown};
use world::{
    ADMIN, MEMBER, Person, World, assert_role, create, members, next_by, next_with, plain_lines,
    post_as, revisions_request, run, set_role, space_certs, store, warnings, words,
};

/// The commands that create a channel in a space and list a space's channels.
const CREATE_CHANNEL: &str = "create-channel";
const CHANNELS: &str = "channels";

/// The space every test creates.
const ACME: &str = "acme";

/// The request that issues a channel ID for `space`, as its line starts.
fn channel_request(space: SpaceId) -> String {
    format!("POST /spaces/{space}/channels")
}

/// The request that stores a revision of `space`, as its line starts.
fn store_request(space: SpaceId) -> String {
    format!("POST /spaces/{space}/revisions")
}

/// Runs `skald create-channel <space>` on `machine`.
fn create_channel(machine: &Machine, space: &str) -> Finished {
    run(machine, &[CREATE_CHANNEL, space])
}

/// Runs `skald channels <space>` on `machine`; fails unless it succeeds.
fn channels(machine: &Machine, space: &str) -> Finished {
    let listed = run(machine, &[CHANNELS, space]);
    assert!(listed.success, "{listed:?}");
    listed
}

/// The revisions of `space` the server holds, for the session `token`.
fn server_rosters(cloud: &Cloud, token: &str, space: SpaceId) -> Vec<Roster> {
    cloud
        .revisions(token, space)
        .into_iter()
        .map(|(_, roster)| roster)
        .collect()
}

/// A channel ID the server issues for `space` to the session `token`, through the API.
fn issue_channel(cloud: &Cloud, token: &str, space: SpaceId) -> ChannelId {
    let path = format!("/spaces/{space}/channels");
    let (status, body) = post_as(cloud, token, &path, &json!({}));
    assert_eq!(status, 201, "POST {path}: {body}");
    body["channel"]
        .as_str()
        .and_then(|id| id.parse().ok())
        .unwrap_or_else(|| panic!("POST {path}: {body}"))
}

/// The channel IDs the server issued in its answers from `since` on.
fn issued_since(cloud: &Cloud, since: Instant) -> Vec<String> {
    strings_named(&cloud.relay.answered_since(since), "channel")
}

/// A channel with `members`, each with its `history`, and no services.
fn channel(id: ChannelId, members: &[(&Person, bool)]) -> Channel {
    Channel {
        id,
        members: members
            .iter()
            .map(|(person, history)| ChannelMember {
                account: person.account,
                history: *history,
            })
            .collect(),
        services: Vec::new(),
        unknown: Unknown::default(),
    }
}

/// From now on, the first answer the proxy gives to a request for `space`'s revisions holds only
/// the first `keep` of them, as if the device fetched them before the others were stored; every
/// later answer holds them all.
fn stale_once(cloud: &Cloud, space: SpaceId, keep: usize) {
    let answers = AtomicUsize::new(0);
    cloud.relay.rewrite(&revisions_request(space), move |body| {
        if answers.fetch_add(1, Ordering::SeqCst) > 0 {
            return body.to_owned();
        }
        let mut list: Value = serde_json::from_str(body).expect("the server's revisions");
        list["revisions"]
            .as_array_mut()
            .expect("a list of revisions")
            .truncate(keep);
        list.to_string()
    });
}

/// Fails unless `next`, a revision the server holds, is the one after `previous` that `person`'s
/// device signed to add one channel: the same members, service agents and policy, every channel of
/// `previous` as it was, and one new channel whose only member is `person`'s account with `history`
/// true. The device that follows the space accepts it. Gives the new channel's ID.
fn assert_adds_a_channel(
    cloud: &Cloud,
    previous: &Roster,
    next: &Roster,
    person: &Person,
) -> ChannelId {
    let (before, after) = (previous.body(), next.body());
    assert_eq!(after.revision, before.revision + 1, "{after:?}");
    assert_eq!(
        after.prev,
        Some(previous.hash()),
        "built on another revision"
    );
    assert_eq!(after.signer, person.device, "signed by another device");
    assert_eq!(after.members, before.members, "the members changed");
    assert_eq!(after.service_agents, before.service_agents);
    assert_eq!(after.policy, before.policy, "the policy changed");
    let certs = space_certs(cloud, &person.token, after.space);
    assert_eq!(
        accept_next(previous, next, &certs, person.account, &person.master),
        Ok(()),
        "the revision with the channel is not accepted"
    );
    for kept in &before.channels {
        assert!(
            after.channels.contains(kept),
            "{:?} is not kept: {:?}",
            kept.id,
            after.channels
        );
    }
    let added: Vec<&Channel> = after
        .channels
        .iter()
        .filter(|channel| !before.channels.iter().any(|kept| kept.id == channel.id))
        .collect();
    assert_eq!(added.len(), 1, "channels added: {added:?}");
    assert_eq!(
        added[0],
        &channel(added[0].id, &[(person, true)]),
        "the new channel"
    );
    assert_eq!(after.channels.len(), before.channels.len() + 1);
    added[0].id
}

/// The lines of `listed` but its warnings that hold the channel ID `id` as a word.
fn lines_of(listed: &Finished, id: &str) -> Vec<String> {
    plain_lines(listed)
        .into_iter()
        .filter(|line| words(line).contains(&id))
        .collect()
}

/// Fails unless `listed` shows the channel `id` on one line, with the handles of `with` and none of
/// `without`.
fn assert_channel(listed: &Finished, id: ChannelId, with: &[&Name], without: &[&Name]) {
    let id = id.to_string();
    let lines = lines_of(listed, &id);
    assert_eq!(lines.len(), 1, "{id} is not listed once: {listed:?}");
    let line = words(&lines[0]);
    for handle in with {
        assert!(
            line.contains(&handle.as_str()),
            "{id} not with {handle}: {listed:?}"
        );
    }
    for handle in without {
        assert!(
            !line.contains(&handle.as_str()),
            "{id} with {handle}: {listed:?}"
        );
    }
}

/// Fails if `listed` shows one of the channel IDs `ids`.
fn assert_unlisted(listed: &Finished, ids: &[String]) {
    for id in ids {
        assert_eq!(lines_of(listed, id), Vec::<String>::new(), "{id} is listed");
    }
}

/// Fails unless `finished` failed saying why: that the account is no admin.
fn assert_refused_as_no_admin(finished: &Finished) {
    assert!(!finished.success, "{finished:?}");
    assert!(
        finished.shown().to_lowercase().contains(ADMIN),
        "it does not say why: {finished:?}"
    );
}

/// From now on, the server's database refuses every new revision, as a failing database would.
fn refuse_revisions(cloud: &Cloud) {
    cloud.execute(
        "CREATE FUNCTION skald_test_refuse() RETURNS trigger LANGUAGE plpgsql AS
           $$ BEGIN RAISE EXCEPTION 'the database is down'; END $$;
         CREATE TRIGGER skald_test_refuse BEFORE INSERT ON roster_revisions
           FOR EACH ROW EXECUTE FUNCTION skald_test_refuse();",
    );
}

/// From now on, the server's database stores new revisions again.
fn store_revisions(cloud: &Cloud) {
    cloud.execute(
        "DROP TRIGGER skald_test_refuse ON roster_revisions;
         DROP FUNCTION skald_test_refuse();",
    );
}

/// Ada's machine creates `acme`, and its revision 2, signed by Ada's device and stored on the
/// server, adds the people of `added` with their roles. Gives revisions 1 and 2.
fn acme_with(cloud: &Cloud, ada: &Person, added: &[(&Person, Role)]) -> [Roster; 2] {
    let first = create(cloud, ada, ACME);
    let second = next_by(&first, ada, |members| {
        members.extend(added.iter().map(|(person, role)| person.member(*role)));
    });
    store(cloud, &ada.token, &second);
    [first, second]
}

/// The revision after `previous`, signed by Bob's device and stored on the server, which makes Ada
/// a member.
fn demote(cloud: &Cloud, ada: &Person, bob: &Person, previous: &Roster) -> Roster {
    let next = next_by(previous, bob, |members| {
        set_role(members, ada.account, Role::Member);
    });
    store(cloud, &bob.token, &next);
    next
}

/// An admin's device creates two channels, each by a revision it signed after the last one: the
/// server holds both revisions, each run says its channel's ID, and the list shows both channels,
/// each with the creator's handle.
#[test]
fn admin_device_creates_two_channels_and_lists_both() {
    let world = World::start();
    let cloud = &world.cloud;
    let ada = world.sign_up(4_120_691_001, "ada-channels", "ada-workstation");
    let first = create(cloud, &ada, ACME);
    let space = first.body().space;
    let empty = channels(&ada.machine, ACME);
    assert_eq!(warnings(&empty), Vec::<String>::new(), "{empty:?}");

    let one = create_channel(&ada.machine, ACME);
    assert!(one.success, "{one:?}");
    let two = create_channel(&ada.machine, ACME);
    assert!(two.success, "{two:?}");

    let rosters = server_rosters(cloud, &ada.token, space);
    assert_eq!(rosters.len(), 3, "the space's revisions: {rosters:?}");
    assert_eq!(rosters[0], first, "revision 1 changed");
    let general = assert_adds_a_channel(cloud, &rosters[0], &rosters[1], &ada);
    let random = assert_adds_a_channel(cloud, &rosters[1], &rosters[2], &ada);
    assert_ne!(general, random, "one channel twice");
    assert!(one.shown().contains(&general.to_string()), "{one:?}");
    assert!(two.shown().contains(&random.to_string()), "{two:?}");

    let listed = channels(&ada.machine, ACME);
    assert_eq!(warnings(&listed), Vec::<String>::new(), "{listed:?}");
    assert_channel(&listed, general, &[&ada.handle], &[]);
    assert_channel(&listed, random, &[&ada.handle], &[]);
    assert_role(&members(&ada.machine, ACME), &ada.handle, ADMIN);
}

/// Ada's device follows `acme` to revision 2, which made Bob an admin and Carol a member; Bob's
/// device then stores revision 3 with a channel of Bob and Carol. Ada's device, which first sees the
/// revisions as they were before Bob's, signs a revision 3 with its channel, which the server
/// refuses; it accepts Bob's revision and adds its channel by a revision 4 on top of it. The list
/// shows both channels, each with its members' handles.
#[test]
fn revision_another_admin_stored_first_is_accepted_and_the_channel_added_on_top() {
    let world = World::start();
    let cloud = &world.cloud;
    let ada = world.sign_up(4_120_691_101, "ada-on-top", "ada-workstation");
    let bob = world.sign_up(4_120_691_102, "bob-first", "bob-workstation");
    let carol = world.sign_up(4_120_691_103, "carol-invited", "carol-laptop");
    let [first, second] = acme_with(cloud, &ada, &[(&bob, Role::Admin), (&carol, Role::Member)]);
    let space = first.body().space;
    assert_role(&members(&ada.machine, ACME), &bob.handle, ADMIN);
    let bobs = issue_channel(cloud, &bob.token, space);
    let third = next_with(&second, (bob.device, &bob.key), |body| {
        body.channels
            .push(channel(bobs, &[(&bob, true), (&carol, false)]));
    });
    store(cloud, &bob.token, &third);
    stale_once(cloud, space, 2);
    let since = cloud.relay.mark();

    let created = create_channel(&ada.machine, ACME);

    assert!(created.success, "{created:?}");
    assert!(
        !warnings(&created).iter().any(|line| line.contains("fork")),
        "its own refused revision is taken for a fork: {created:?}"
    );
    assert!(
        cloud.count_sent(since, &store_request(space)) >= 2,
        "the device did not sign again after the server refused its revision: {:?}",
        cloud.relay.sent_since(since)
    );
    let rosters = server_rosters(cloud, &ada.token, space);
    assert_eq!(rosters.len(), 4, "the space's revisions: {rosters:?}");
    assert_eq!(rosters[2], third, "Bob's revision 3 is not kept");
    let adas = assert_adds_a_channel(cloud, &third, &rosters[3], &ada);
    assert!(created.shown().contains(&adas.to_string()), "{created:?}");

    let listed = channels(&ada.machine, ACME);
    assert_eq!(warnings(&listed), Vec::<String>::new(), "{listed:?}");
    assert_channel(&listed, bobs, &[&bob.handle, &carol.handle], &[&ada.handle]);
    assert_channel(&listed, adas, &[&ada.handle], &[&bob.handle, &carol.handle]);
}

/// Ada's device follows `acme` to revision 2, which made Bob an admin, and signs a revision 3 with a
/// channel, which the server fails to store; Bob's device then stores revision 3, which makes Ada a
/// member. The next `skald create-channel` on Ada's device sends its revision 3 again, which the
/// server refuses; it accepts Bob's revision, in which its account is no admin, and stops saying
/// why. The server keeps Bob's revision as the latest, the device sends the refused revision no
/// more, and lists no channel.
#[test]
fn latest_revision_that_makes_the_account_no_admin_stops_the_creation() {
    let world = World::start();
    let cloud = &world.cloud;
    let ada = world.sign_up(4_120_691_201, "ada-demoted", "ada-workstation");
    let bob = world.sign_up(4_120_691_202, "bob-demotes", "bob-workstation");
    let [first, second] = acme_with(cloud, &ada, &[(&bob, Role::Admin)]);
    let space = first.body().space;
    assert_role(&members(&ada.machine, ACME), &bob.handle, ADMIN);
    // The server issues a channel ID only to an admin in the latest revision, so Ada's device gets
    // its own before Bob's revision is stored; the server's database refuses Ada's revision.
    refuse_revisions(cloud);
    let issuing = cloud.relay.mark();
    let failed = create_channel(&ada.machine, ACME);
    assert!(!failed.success, "{failed:?}");
    store_revisions(cloud);
    let third = demote(cloud, &ada, &bob, &second);
    let since = cloud.relay.mark();

    let refused = create_channel(&ada.machine, ACME);

    assert_refused_as_no_admin(&refused);
    assert_eq!(
        cloud.count_sent(since, &store_request(space)),
        1,
        "the device did not send its revision once, or signed again: {:?}",
        cloud.relay.sent_since(since)
    );
    let issued = issued_since(cloud, issuing);
    assert_eq!(issued.len(), 1, "the channel IDs issued: {issued:?}");
    let rosters = server_rosters(cloud, &ada.token, space);
    assert_eq!(rosters, [first, second, third], "the space's revisions");

    let later = cloud.relay.mark();
    let listed = channels(&ada.machine, ACME);
    assert_unlisted(&listed, &issued);
    assert_eq!(
        cloud.count_sent(later, &store_request(space)),
        0,
        "the refused revision was sent again"
    );
    assert_role(&members(&ada.machine, ACME), &ada.handle, MEMBER);
}

/// A device of a member that is no admin creates nothing: Ada's device accepted Bob's revision that
/// made Ada a member, and `skald create-channel` fails saying why and sends no request that changes
/// anything; the space keeps its revisions.
#[test]
fn device_of_a_member_that_is_no_admin_creates_nothing() {
    let world = World::start();
    let cloud = &world.cloud;
    let ada = world.sign_up(4_120_691_301, "ada-member", "ada-workstation");
    let bob = world.sign_up(4_120_691_302, "bob-admin", "bob-workstation");
    let [first, second] = acme_with(cloud, &ada, &[(&bob, Role::Admin)]);
    let space = first.body().space;
    let third = demote(cloud, &ada, &bob, &second);
    assert_role(&members(&ada.machine, ACME), &ada.handle, MEMBER);
    let since = cloud.relay.mark();

    let refused = create_channel(&ada.machine, ACME);

    assert_refused_as_no_admin(&refused);
    assert_eq!(
        cloud.count_sent(since, "POST "),
        0,
        "the device sent a request: {:?}",
        cloud.relay.sent_since(since)
    );
    let rosters = server_rosters(cloud, &ada.token, space);
    assert_eq!(rosters, [first, second, third], "the space's revisions");
    assert_unlisted(&channels(&ada.machine, ACME), &issued_since(cloud, since));
}

/// While the server fails to store the revision with the channel, the space stays at the last
/// accepted revision: the run fails, and a later run lists no channel. Once the server stores
/// revisions again, a later run resends the revision, and the channel, the one whose ID the server
/// issued for it, is listed.
#[test]
fn until_the_server_takes_the_revision_the_space_stays_at_the_last_accepted_one() {
    let world = World::start();
    let cloud = &world.cloud;
    let ada = world.sign_up(4_120_691_401, "ada-pending", "ada-workstation");
    let first = create(cloud, &ada, ACME);
    let space = first.body().space;
    // The server's database refuses every new revision, as a failing database would.
    cloud.execute(
        "CREATE FUNCTION skald_test_refuse() RETURNS trigger LANGUAGE plpgsql AS
           $$ BEGIN RAISE EXCEPTION 'the database is down'; END $$;
         CREATE TRIGGER skald_test_refuse BEFORE INSERT ON roster_revisions
           FOR EACH ROW EXECUTE FUNCTION skald_test_refuse();",
    );
    let since = cloud.relay.mark();

    let failed = create_channel(&ada.machine, ACME);

    assert!(!failed.success, "{failed:?}");
    assert!(
        cloud.count_sent(since, &store_request(space)) >= 1,
        "no revision was sent"
    );
    let issued = issued_since(cloud, since);
    assert_eq!(issued.len(), 1, "the channel IDs issued: {issued:?}");
    assert_eq!(
        server_rosters(cloud, &ada.token, space),
        std::slice::from_ref(&first)
    );
    let pending = channels(&ada.machine, ACME);
    assert_unlisted(&pending, &issued);
    assert_role(&members(&ada.machine, ACME), &ada.handle, ADMIN);

    cloud.execute(
        "DROP TRIGGER skald_test_refuse ON roster_revisions;
         DROP FUNCTION skald_test_refuse();",
    );
    // A revision signed again now has another timestamp, and so other bytes.
    thread::sleep(Duration::from_millis(1100));
    let listed = channels(&ada.machine, ACME);

    let rosters = server_rosters(cloud, &ada.token, space);
    assert_eq!(rosters.len(), 2, "the space's revisions: {rosters:?}");
    let added = assert_adds_a_channel(cloud, &first, &rosters[1], &ada);
    assert_eq!(
        added.to_string(),
        issued[0],
        "another channel than the one issued"
    );
    assert_eq!(
        issued_since(cloud, since),
        issued,
        "another channel ID was issued"
    );
    assert_channel(&listed, added, &[&ada.handle], &[]);
}

/// `skald create-channel` first sends a revision with a channel the server has not taken: while the
/// server fails to store it, the run fails and gets no other channel ID; once the server stores
/// revisions again, the run sends it, then adds a second channel on top of it.
#[test]
fn create_channel_sends_a_waiting_revision_first() {
    let world = World::start();
    let cloud = &world.cloud;
    let ada = world.sign_up(4_120_691_601, "ada-waiting", "ada-workstation");
    let first = create(cloud, &ada, ACME);
    let space = first.body().space;
    refuse_revisions(cloud);
    let since = cloud.relay.mark();
    let failed = create_channel(&ada.machine, ACME);
    assert!(!failed.success, "{failed:?}");

    let again = create_channel(&ada.machine, ACME);

    assert!(!again.success, "{again:?}");
    let issued = issued_since(cloud, since);
    assert_eq!(issued.len(), 1, "the channel IDs issued: {issued:?}");
    assert_eq!(
        cloud.count_sent(since, &store_request(space)),
        2,
        "the waiting revision was not sent again, or another was sent"
    );
    store_revisions(cloud);

    let created = create_channel(&ada.machine, ACME);

    assert!(created.success, "{created:?}");
    let rosters = server_rosters(cloud, &ada.token, space);
    assert_eq!(rosters.len(), 3, "the space's revisions: {rosters:?}");
    let general = assert_adds_a_channel(cloud, &first, &rosters[1], &ada);
    assert_eq!(
        general.to_string(),
        issued[0],
        "another channel than the one issued"
    );
    let random = assert_adds_a_channel(cloud, &rosters[1], &rosters[2], &ada);
    assert!(created.shown().contains(&random.to_string()), "{created:?}");
    let listed = channels(&ada.machine, ACME);
    assert_channel(&listed, general, &[&ada.handle], &[]);
    assert_channel(&listed, random, &[&ada.handle], &[]);
}

/// Ada's device adds a channel by revision 3, then signs a revision 4 with another, which the server
/// fails to store; Bob's device then stores revision 4 with a channel of its own. While Ada's
/// revision waits, as when the answer to sending it again is lost, Ada's device keeps `acme` at
/// revision 3 and lists only its first channel. The next run that sends it again accepts Bob's
/// revision and adds Ada's channel on top of it.
#[test]
fn waiting_revision_keeps_the_space_until_it_goes_on_top_of_another_admins() {
    let world = World::start();
    let cloud = &world.cloud;
    let ada = world.sign_up(4_120_691_701, "ada-waits", "ada-workstation");
    let bob = world.sign_up(4_120_691_702, "bob-goes-first", "bob-workstation");
    let [first, _] = acme_with(cloud, &ada, &[(&bob, Role::Admin)]);
    let space = first.body().space;
    assert!(create_channel(&ada.machine, ACME).success);
    let third = server_rosters(cloud, &ada.token, space).remove(2);
    let general = third.body().channels[0].id;
    refuse_revisions(cloud);
    let since = cloud.relay.mark();
    let failed = create_channel(&ada.machine, ACME);
    assert!(!failed.success, "{failed:?}");
    let issued = issued_since(cloud, since);
    store_revisions(cloud);
    let bobs = issue_channel(cloud, &bob.token, space);
    let fourth = next_with(&third, (bob.device, &bob.key), |body| {
        body.channels.push(channel(bobs, &[(&bob, true)]));
    });
    store(cloud, &bob.token, &fourth);
    cloud.relay.lose_answer(&store_request(space), Lose::Close);

    let waiting = channels(&ada.machine, ACME);

    assert_eq!(
        cloud.relay.lost(),
        1,
        "the waiting revision was not sent again"
    );
    assert_channel(&waiting, general, &[&ada.handle], &[]);
    assert_unlisted(&waiting, &[bobs.to_string(), issued[0].clone()]);
    assert_eq!(server_rosters(cloud, &ada.token, space).len(), 4);

    let listed = channels(&ada.machine, ACME);

    let rosters = server_rosters(cloud, &ada.token, space);
    assert_eq!(rosters.len(), 5, "the space's revisions: {rosters:?}");
    assert_eq!(rosters[3], fourth, "Bob's revision 4 is not kept");
    let adas = assert_adds_a_channel(cloud, &fourth, &rosters[4], &ada);
    assert_eq!(
        adas.to_string(),
        issued[0],
        "another channel than the one issued"
    );
    assert_channel(&listed, general, &[&ada.handle], &[&bob.handle]);
    assert_channel(&listed, bobs, &[&bob.handle], &[&ada.handle]);
    assert_channel(&listed, adas, &[&ada.handle], &[&bob.handle]);
}

/// Which answer of the server the relay loses.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Lost {
    /// The answer that issues the channel ID.
    ChannelId,
    /// The answer to the revision with the channel.
    Revision,
}

/// The relay loses the server's answer that `lost` names as `how` says: it closes the connection,
/// or it withholds the answer and the test kills the CLI. A later run lists the space's channels;
/// if no revision with a channel was sent, which only a lost channel ID allows, `skald
/// create-channel` runs again. The space then has one revision 2 with one channel, which the list
/// shows, and no other channel ID the server issued is listed.
fn one_channel_after_a_lost_answer(lost: Lost, how: Lose, (id, login): (u64, &str)) {
    let world = World::start();
    let cloud = &world.cloud;
    let ada = world.sign_up(id, login, "ada-flaky-network");
    let first = create(cloud, &ada, ACME);
    let space = first.body().space;
    let request = match lost {
        Lost::ChannelId => channel_request(space),
        Lost::Revision => store_request(space),
    };
    let since = cloud.relay.mark();
    cloud.relay.lose_answer(&request, how);

    let started = ada.machine.start(&[CREATE_CHANNEL, ACME]);
    let done = match how {
        // It may try again itself and finish.
        Lose::Close => started.finish().success,
        Lose::Withhold => {
            cloud.kill_once_lost(started, &request);
            false
        }
    };
    assert_eq!(cloud.relay.lost(), 1, "the server never answered {request}");
    if lost == Lost::Revision {
        assert_eq!(
            server_rosters(cloud, &ada.token, space).len(),
            2,
            "the server took no revision 2"
        );
    }
    // A revision signed again now has another timestamp, and so other bytes.
    thread::sleep(Duration::from_millis(1100));

    let mut listed = channels(&ada.machine, ACME);
    if server_rosters(cloud, &ada.token, space).len() == 1 {
        assert!(
            lost == Lost::ChannelId && !done,
            "no revision with the channel after a lost {request}"
        );
        let again = create_channel(&ada.machine, ACME);
        assert!(again.success, "{again:?}");
        listed = channels(&ada.machine, ACME);
    }

    let rosters = server_rosters(cloud, &ada.token, space);
    assert_eq!(rosters.len(), 2, "the space's revisions: {rosters:?}");
    let added = assert_adds_a_channel(cloud, &first, &rosters[1], &ada);
    assert_channel(&listed, added, &[&ada.handle], &[]);
    let others: Vec<String> = issued_since(cloud, since)
        .into_iter()
        .filter(|issued| *issued != added.to_string())
        .collect();
    assert_unlisted(&listed, &others);
}

#[test]
fn channel_id_answer_dropped_by_the_proxy_leaves_one_channel() {
    one_channel_after_a_lost_answer(
        Lost::ChannelId,
        Lose::Close,
        (4_120_691_501, "ada-lost-channel"),
    );
}

#[test]
fn channel_id_answer_lost_with_the_cli_killed_leaves_one_channel() {
    one_channel_after_a_lost_answer(
        Lost::ChannelId,
        Lose::Withhold,
        (4_120_691_502, "ada-killed-channel"),
    );
}

#[test]
fn revision_answer_dropped_by_the_proxy_leaves_one_channel() {
    one_channel_after_a_lost_answer(
        Lost::Revision,
        Lose::Close,
        (4_120_691_503, "ada-lost-revision"),
    );
}

#[test]
fn revision_answer_lost_with_the_cli_killed_leaves_one_channel() {
    one_channel_after_a_lost_answer(
        Lost::Revision,
        Lose::Withhold,
        (4_120_691_504, "ada-killed-revision"),
    );
}
