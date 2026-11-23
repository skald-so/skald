//! KeyPackages: one the server hands out as a member's that fails the checks before an Add.

use skald_proto::Role;

use crate::fixtures::fronted;
use crate::kit::{HAND_OUT, RECV, create_space, received, swap_key_packages, with_channel};
use crate::played::{Played, stranger};
use crate::world::{ACME, RULES, now};
use crate::{ADA_HOST, BOB_HOST, CAROL_HOST, assert_run_says_none, assert_waits_and_shows_once};

/// Ada's machine creates `acme`, and a revision her device signs adds a channel with Bob and Carol,
/// whose devices the tests play. In place of each KeyPackage of Carol's device the server hands
/// out, the relay gives one of a device of no account. Ada's `skald recv` forms the channel's
/// group: a key change waits and shows once, naming Carol's device or Carol, also once the relay
/// hands out her KeyPackages as the server does.
#[test]
fn a_key_package_that_fails_its_checks_waits_until_seen() {
    let (world, _front) = fronted();
    let cloud = &world.cloud;
    let ada = world.sign_up(4_188_401_001, "ada-handed-a-bad-package", ADA_HOST);
    let [bob, carol] = [
        (4_188_401_002, "bob-in-the-new-channel", BOB_HOST),
        (4_188_401_003, "carol-package-swapped", CAROL_HOST),
    ]
    .map(|(id, login, host)| Played::new(cloud, world.sign_up(id, login, host)));
    let space = create_space(cloud, &ada, ACME);
    let added = [(&bob.person, Role::Member), (&carol.person, Role::Member)];
    with_channel(cloud, space.id, &ada, &added);
    assert_run_says_none(&ada.machine, &[RULES]);
    let package = stranger().key_package(now()).expect("a KeyPackage");
    swap_key_packages(cloud, carol.device(), package.as_ref());

    received(&ada.machine);
    cloud.relay.rewrite(HAND_OUT, str::to_owned);

    let names = [carol.device().to_string(), carol.person.handle.to_string()];
    assert_waits_and_shows_once(&ada.machine, &[RECV], &names);
}
