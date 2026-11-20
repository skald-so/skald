//! An admin replaces a service agent's key: the new device takes the old one's place in the next
//! revision, the old device is revoked, and the old key secret and tokens are refused; for a
//! member agent, the old device leaves the groups and the new one gets in. A device offline since
//! before the change accepts the revisions that list the old device.

use skald_crypto::accept_next;
use skald_proto::{ChannelId, SpaceId};

use crate::changes::{ROUNDS, lists, new_device, replace_key, revoked, services_of};
use crate::devices::second_machine;
use crate::fixtures::{fronted, last_channel, send};
use crate::kit::{channels, latest, received};
use crate::member::{Bot, Runner, assert_not_shown, shown_once, texts_of};
use crate::scene::{Agent, Scene};
use crate::service_kit::{
    Acme, BOT, RECEIVE, SEND, assert_certified_by, assert_record, create_agent, entry_of, key_of,
    own_record, rosters, secrets_of,
};
use crate::talk::messages_sent;
use crate::world::{ACME, CREATE_CHANNEL, Person, World, run, space_certs, warnings};

/// What the agent and Ada say; none is anywhere else.
const BEFORE_SAYS: &str = "key change test: deploy-bot reports with its first key";
const OLD_SAYS: &str = "key change test: deploy-bot tries its old key";
const NEW_SAYS: &str = "key change test: deploy-bot reports with its new key";
const ADA_SAYS: &str = "key change test: ada asks the new deploy-bot";

/// The stateless agent sends with its first key, and Ada's run shows it. Ada's machine replaces its
/// key: stdout is only the new key secret and token, which no file of Ada's machine holds. The
/// space's next revision, signed by Ada's device, lists a new `service` device in the entry in
/// place of the old one, which the revision before it lists; Bob's device accepts it and finds the
/// new device's certificate valid, issued by Ada's device. The new key secret's anchor is that
/// revision. The server holds the old device's revocation, the old token is refused and the new
/// one reads its record, of the new device. A run with the old secrets sends nothing and receives
/// nothing; one with the new secrets sends, and Ada's run shows its line from the new device, and
/// receives Ada's next line.
#[test]
fn after_a_key_change_the_old_key_and_tokens_are_refused_and_the_new_ones_work() {
    let scene = Scene::new(
        [3_135_900_001, 3_135_900_002],
        ["ada-keychange-standin", "bob-keychange-standin"],
    );
    let (cloud, ada, channel) = (scene.cloud(), scene.ada(), scene.channel());
    scene.agent.sent(channel, BEFORE_SAYS);
    scene.assert_shows_the_agent(&scene.ada_receives(), BEFORE_SAYS);

    let replaced = replace_key(ada, &[channel]);

    let fresh = secrets_of(&replaced);
    for secret in [&fresh.key, &fresh.token] {
        assert_eq!(
            ada.machine.files_holding(secret),
            Vec::<std::path::PathBuf>::new(),
            "files of Ada's machine that hold a new secret"
        );
    }
    let new = new_device(cloud, ada, scene.device);
    let held = scene.revisions();
    let [.., before, after] = held.as_slice() else {
        panic!("too few revisions: {held:?}");
    };
    assert_eq!(entry_of(before, BOT), Some(scene.device), "{before:?}");
    assert_eq!(entry_of(after, BOT), Some(new), "{after:?}");
    assert_eq!(after.body().signer, ada.device, "{after:?}");
    let bob = scene.bob();
    let certs = space_certs(cloud, &bob.token, scene.space());
    accept_next(before, after, &certs, bob.account, &bob.master)
        .unwrap_or_else(|err| panic!("Bob's device refuses the revision: {err:?}"));
    assert_certified_by(ada, &certs, new);
    assert_eq!(
        key_of(&fresh.key).anchor,
        after.hash(),
        "the new key secret's anchor"
    );
    assert!(
        revoked(cloud, ada, scene.device),
        "the old device is not revoked"
    );
    assert!(
        own_record(cloud, &scene.secrets.token).is_none(),
        "the server takes the old token"
    );
    let record = own_record(cloud, &fresh.token).expect("the server refuses the new token");
    assert_record(&record, new, &[SEND, RECEIVE], &[channel]);

    let mark = scene.mark();
    let refused = scene.agent.send(channel, OLD_SAYS);
    assert!(!refused.success, "the old key sent: {refused:?}");
    scene.assert_none_sent(mark, "a run with the old secrets");

    let agent = Agent::of(&fresh);
    agent.sent(channel, NEW_SAYS);
    let got = scene.ada_receives();
    shown_once(&got, NEW_SAYS, (channel, &scene.address, new));
    assert_not_shown(&got, OLD_SAYS);

    let said = send(&ada.machine, channel, ADA_SAYS);
    assert!(said.success, "Ada's send failed: {said:?}");
    let old_run = scene.agent.recv();
    assert!(!old_run.success, "the old key received: {old_run:?}");
    assert_not_shown(&old_run, ADA_SAYS);
    scene.assert_agent_shows(&agent.received(), ADA_SAYS, ada);
}

/// The member agent `ci/deploy-bot` is in the channel's group. Ada's machine replaces its key: the
/// next revision lists the new device in the channel's `services` in place of the old one. With
/// the new secrets and a new state directory, the agent's runs, Ada's and Bob's device bring the
/// group in line: Bob's device holds the new device in it and not the old one. The new agent then
/// gets Ada's line and says one, which Bob's device reads from the new device. A run with the old
/// secrets and state sends nothing.
#[test]
fn after_a_member_agent_s_key_change_the_old_device_leaves_the_groups_and_the_new_one_gets_in() {
    let mut bot = Bot::created(
        [3_135_901_001, 3_135_901_002],
        ["ada-memberkey-standin", "bob-memberkey-standin"],
    );
    bot.brought_in();
    let (old_device, channel) = (bot.device, bot.channel());

    let replaced = replace_key(&bot.talk.ada, &[channel]);

    let fresh = secrets_of(&replaced);
    let cloud = bot.talk.cloud();
    let new = new_device(cloud, &bot.talk.ada, old_device);
    let after = latest(cloud, &bot.talk.ada.token, bot.talk.space.id);
    assert_eq!(entry_of(&after, BOT), Some(new), "{after:?}");
    assert_eq!(services_of(&after, channel), [new], "{after:?}");
    assert!(!lists(&after, old_device), "{after:?}");

    let old_runner = std::mem::replace(&mut bot.runner, Runner::of(&fresh));
    bot.device = new;
    bot.brought_in();
    for _ in 0..ROUNDS {
        if !bot.talk.played[0].members(channel).contains(&old_device) {
            break;
        }
        received(&bot.talk.ada.machine);
        bot.bob_takes_in();
    }
    let members = bot.talk.played[0].members(channel);
    assert!(
        members.contains(&new) && !members.contains(&old_device),
        "the channel's group as Bob's device holds it: {members:?}; old {old_device}, new {new}"
    );

    bot.ada_reaches_the_agent(ADA_SAYS);
    bot.says(NEW_SAYS);
    bot.bob_takes_in();
    let read: Vec<Vec<String>> = (bot.bob_read_from_the_agent().iter())
        .map(|(_, content)| texts_of(content))
        .collect();
    assert_eq!(
        read,
        [[NEW_SAYS]],
        "what Bob's device read from the new device"
    );

    let mark = bot.talk.front.sent().len();
    let refused = old_runner.send(channel, OLD_SAYS);
    assert!(!refused.success, "the old key sent: {refused:?}");
    let sent = messages_sent(&bot.talk.front, channel, mark);
    assert!(sent.is_empty(), "the old key's run sent {sent:?}");
}

/// Ada's second machine, linked when `acme` had only its default channel, stays offline. Ada's
/// first machine creates `ci/deploy-bot`, adds a channel, replaces the agent's key, which revokes
/// its old device, and adds another channel. The second machine then accepts every revision, those
/// that list the old device included: it warns of no revision, and lists both new channels.
#[test]
fn a_device_offline_since_before_the_key_change_accepts_the_revisions_that_list_the_old_device() {
    let (world, _front) = fronted();
    let cloud = &world.cloud;
    let ada = world.sign_up(3_135_902_001, "ada-offline-standin", "ada-workstation");
    let acme = Acme::create(cloud, &ada);
    let laptop = second_machine(&world, &ada, "ada-laptop");
    secrets_of(&create_agent(&ada.machine, ACME, BOT, &[acme.default], &[]));
    let old = entry_of(&latest(cloud, &ada.token, acme.id), BOT).expect("the agent's entry");
    let between = add_channel(&world, &ada, acme.id);

    secrets_of(&replace_key(&ada, &[acme.default]));
    assert!(revoked(cloud, &ada, old), "the old device is not revoked");
    let later = add_channel(&world, &ada, acme.id);
    let held = rosters(cloud, &ada.token, acme.id);
    assert!(
        held.iter().filter(|roster| lists(roster, old)).count() >= 2,
        "the revisions that list the old device: {held:?}"
    );

    let listed = channels(&laptop.machine, ACME);

    let of_revisions: Vec<String> = (warnings(&listed).into_iter())
        .filter(|line| line.contains("revision"))
        .collect();
    assert_eq!(of_revisions, Vec::<String>::new(), "{listed:?}");
    for channel in [between, later] {
        assert!(
            listed.stdout.contains(&channel.to_string()),
            "the second machine does not list {channel}: {listed:?}"
        );
    }
}

/// Runs `skald create-channel acme` on `person`'s machine, which must succeed; gives the new
/// channel's ID.
#[track_caller]
fn add_channel(world: &World, person: &Person, space: SpaceId) -> ChannelId {
    let created = run(&person.machine, &[CREATE_CHANNEL, ACME]);
    assert!(created.success, "{created:?}");
    last_channel(&world.cloud, person, space).1
}
