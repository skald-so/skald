//! The anchors object each `client` device keeps on the server: signed on each revision it accepts,
//! resent until the server takes it, and none once the device is revoked.

use base64::Engine as _;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use serde_json::json;
use skald_crypto::sign_revocation;
use skald_proto::{Cbor as _, NonEmpty, RevocationBody, Roster, Unknown};

use crate::fixtures::{Issuer, add_device};
use crate::objects::{ANCHORS_REQUEST, assert_anchors, held, latest, personal_space};
use crate::relay::Lose;
use crate::world::{
    ACME, MEMBERS, Person, SPACES, World, create, members, next_with, now, post_as, run, spaces,
    store,
};

/// The host of the tests' machine.
const HOST: &str = "ada-workstation";

/// Ada's account signs up; its first machine runs `skald spaces`, which adds the personal space's
/// default channel, and creates `acme`. Gives Ada and `acme`'s revision 2, its latest.
fn ada_in_acme(world: &World, user: (u64, &str)) -> (Person, Roster) {
    let ada = world.sign_up(user.0, user.1, HOST);
    spaces(&ada.machine);
    let created = create(&world.cloud, &ada, ACME);
    (ada, created)
}

/// Once Ada's machine accepts a revision, the server holds its anchors object, naming the latest
/// revision it accepted of each of its spaces and its personal space: after its personal space's
/// default channel, after it creates `acme`, and after it accepts `acme`'s revision 3, which the
/// test signs with the machine's key and stores. Until the machine has run since, the object still
/// names revision 2.
#[test]
fn each_accepted_revision_leaves_an_anchors_object_of_the_latest_revisions_and_the_personal_space()
{
    let world = World::start();
    let cloud = &world.cloud;
    let ada = world.sign_up(4_172_020_101, "ada-anchors-standin", HOST);
    spaces(&ada.machine);
    let personal = personal_space(cloud, &ada);
    assert_eq!(
        cloud.revisions(&ada.token, personal).len(),
        2,
        "the personal space's default channel"
    );
    assert_anchors(cloud, &ada, &latest(cloud, &ada), personal);

    let created = create(cloud, &ada, ACME);
    let acme = created.body().space;
    let accepted = latest(cloud, &ada);
    assert_eq!(
        accepted.get(&acme),
        Some(&created.hash()),
        "acme's revision 2"
    );
    assert_anchors(cloud, &ada, &accepted, personal);

    let third = next_with(&created, (ada.device, &ada.key), |_| {});
    store(cloud, &ada.token, &third);
    assert_anchors(cloud, &ada, &accepted, personal);

    members(&ada.machine, ACME);

    let mut expected = accepted;
    expected.insert(acme, third.hash());
    assert_eq!(
        expected,
        latest(cloud, &ada),
        "acme's revision 3 is its latest"
    );
    assert_anchors(cloud, &ada, &expected, personal);
}

/// The relay answers every request that stores an anchors object as a server without them would,
/// so none reaches the server while Ada's first machine signs in and adds its personal space's
/// default channel, as with a device whose local state predates anchors objects. Once the relay
/// lets them through, the machine's next run, `skald spaces`, sends one, and the server holds the
/// machine's anchors object of the latest revision of its personal space.
#[test]
fn device_whose_anchors_never_reached_the_server_uploads_them_at_its_next_run() {
    let world = World::start();
    let cloud = &world.cloud;
    cloud.relay.refuse(ANCHORS_REQUEST, "404 Not Found");
    let user = cloud.user(4_172_020_201, "ada-old-anchors");
    // Its result does not matter.
    let (machine, _) = cloud.first_machine(&world.browser.page(), &user, HOST);
    let ada = world.person(user, machine, HOST);
    // Its result does not matter.
    let _ = run(&ada.machine, &[SPACES]);
    let personal = personal_space(cloud, &ada);
    assert!(
        held(cloud, &ada.token).is_empty(),
        "an anchors object got through"
    );
    cloud.relay.stop_refusing();
    let since = cloud.relay.mark();

    spaces(&ada.machine);

    assert!(
        cloud.count_sent(since, ANCHORS_REQUEST) >= 1,
        "no anchors object was sent"
    );
    assert_anchors(cloud, &ada, &latest(cloud, &ada), personal);
}

/// Ada's machine accepts `acme`'s revision 3, which the test signs with its key and stores, and the
/// relay loses the server's answer to the anchors object it sends then, as `how` says: it closes
/// the connection, or it withholds the answer and the test kills the CLI. The next run, `skald
/// spaces`, sends it again, and the server holds the machine's anchors object of `acme`'s revision
/// 3 and of the personal space's latest.
fn recovers_from_a_lost_answer(how: Lose, user: (u64, &str)) {
    let world = World::start();
    let cloud = &world.cloud;
    let (ada, created) = ada_in_acme(&world, user);
    let personal = personal_space(cloud, &ada);
    let third = next_with(&created, (ada.device, &ada.key), |_| {});
    store(cloud, &ada.token, &third);
    cloud.relay.lose_answer(ANCHORS_REQUEST, how);
    let since = cloud.relay.mark();

    let accepting = ada.machine.start(&[MEMBERS, ACME]);
    match how {
        // Its result does not matter.
        Lose::Close => drop(accepting.finish()),
        Lose::Withhold => cloud.kill_once_lost(accepting, ANCHORS_REQUEST),
    }
    assert_eq!(
        cloud.relay.lost(),
        1,
        "the server never answered an anchors object"
    );

    spaces(&ada.machine);

    assert!(
        cloud.count_sent(since, ANCHORS_REQUEST) >= 2,
        "the anchors object was not sent again after its answer was lost"
    );
    let expected = latest(cloud, &ada);
    assert_eq!(expected.get(&created.body().space), Some(&third.hash()));
    assert_anchors(cloud, &ada, &expected, personal);
}

#[test]
fn anchors_answer_dropped_by_the_proxy_is_recovered_by_the_next_run() {
    recovers_from_a_lost_answer(Lose::Close, (4_172_020_301, "ada-lost-anchors"));
}

#[test]
fn anchors_answer_lost_with_the_cli_killed_is_recovered_by_the_next_run() {
    recovers_from_a_lost_answer(Lose::Withhold, (4_172_020_302, "ada-killed-anchors"));
}

/// Another `client` device of Ada's account, which her machine certified, written to the server's
/// database, signs `acme`'s revision 3, and her machine accepts it: the server holds the machine's
/// anchors object of it. That device then revokes the machine's device, keeping itself, and signs
/// revision 4. The machine's next runs send no anchors object, and the server still holds its last
/// one.
#[test]
fn revoked_device_uploads_no_anchors_object() {
    let world = World::start();
    let cloud = &world.cloud;
    let (ada, created) = ada_in_acme(&world, (4_172_020_401, "ada-revoked-anchors"));
    let personal = personal_space(cloud, &ada);
    let other = add_device(cloud, ada.account, "ada-old-laptop", &Issuer::Device(&ada));
    let by_other = (other.id, &other.signing);
    let third = next_with(&created, by_other, |_| {});
    store(cloud, &ada.token, &third);
    members(&ada.machine, ACME);
    let accepted = latest(cloud, &ada);
    assert_eq!(accepted.get(&created.body().space), Some(&third.hash()));
    assert_anchors(cloud, &ada, &accepted, personal);

    let revocation = sign_revocation(
        &RevocationBody {
            account: ada.account,
            device: ada.device,
            keep: Some(NonEmpty::new(other.id)),
            signer: Some(other.id),
            timestamp: now(),
            also: None,
            unknown: Unknown::default(),
        },
        &other.signing,
    )
    .expect("a revocation");
    let request = json!({ "revocation": URL_SAFE_NO_PAD.encode(revocation.to_cbor()) });
    let (status, answer) = post_as(cloud, &ada.token, "/account/revocations", Some(&request));
    assert_eq!(status, 201, "the revocation: {answer}");
    store(cloud, &ada.token, &next_with(&third, by_other, |_| {}));
    let since = cloud.relay.mark();

    // Their results do not matter: the machine's device is revoked.
    let _ = run(&ada.machine, &[MEMBERS, ACME]);
    let _ = run(&ada.machine, &[SPACES]);

    assert_eq!(
        cloud.count_sent(since, ANCHORS_REQUEST),
        0,
        "a revoked device sent an anchors object"
    );
    assert_anchors(cloud, &ada, &accepted, personal);
}
