//! A creation that races another revision signs its entry on the new one.

use skald_crypto::accept_next;
use skald_proto::Role;

use crate::fixtures::fronted;
use crate::keys;
use crate::kit::{BOT, create_agent, entry_of, rosters, secrets_of, service_of};
use crate::world::{ACME, create, next_by, space_certs, stale_once, store, store_request};

/// Ada's machine knows `acme` at revision 2 when revision 3, by her device elsewhere, adds Bob.
/// Its creation of `ci/deploy-bot` is refused on revision 2 and signs again on revision 3: revision
/// 4 follows it, keeps Bob and adds the entry, and the key secret pins revision 4.
#[test]
fn creation_racing_another_revision_signs_on_the_new_one() {
    let (world, _front) = fronted();
    let cloud = &world.cloud;
    let ada = world.sign_up(3_135_313_001, "ada-race-standin", "ada-workstation");
    let bob = world.sign_up(3_135_313_002, "bob-race-standin", "bob-laptop");
    let second = create(cloud, &ada, ACME);
    let space = second.body().space;
    let third = next_by(&second, &ada, |members| {
        members.push(bob.member(Role::Member))
    });
    store(cloud, &ada.token, &third);
    stale_once(cloud, space, 2);
    let since = cloud.relay.mark();

    let created = create_agent(
        &ada.machine,
        ACME,
        BOT,
        &[second.body().channels[0].id],
        &[],
    );

    let secrets = secrets_of(&created);
    assert!(
        cloud.count_sent(since, &store_request(space)) >= 2,
        "the device did not sign again after the server refused its revision: {:?}",
        cloud.relay.sent_since(since)
    );
    let held = rosters(cloud, &ada.token, space);
    assert_eq!(held.len(), 4, "the space's revisions: {held:?}");
    assert_eq!(held[2], third, "the racing revision is not kept");
    let fourth = &held[3];
    assert_eq!(fourth.body().prev, Some(third.hash()), "{fourth:?}");
    let certs = space_certs(cloud, &ada.token, space);
    accept_next(&third, fourth, &certs, ada.account, &ada.master)
        .unwrap_or_else(|err| panic!("the revision does not follow the racing one: {err:?}"));
    assert!(
        fourth
            .body()
            .members
            .iter()
            .any(|member| member.account == bob.account),
        "Bob is dropped: {fourth:?}"
    );
    let (device, _) = service_of(cloud, &ada);
    assert_eq!(entry_of(fourth, BOT), Some(device), "{fourth:?}");
    assert!(
        keys::holds(secrets.key.as_bytes(), &fourth.hash()),
        "the key secret does not pin the revision with the entry"
    );
}
