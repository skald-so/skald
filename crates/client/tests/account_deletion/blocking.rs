//! The only admin of a space with other members deletes nothing: the command names the space with
//! the command that hands over admin rights, and stops; once the rights are handed over, it deletes.

use crate::deletion::{
    AtGrant, HAND_OVER, assert_deleted, assert_kept, assert_sent_nothing, delete_at_terminal,
    deleted_at_terminal, hand_over, marked_deleted, shared_with,
};
use crate::fixtures::fronted;
use crate::linking::lines_holding;
use crate::talk::{ADA_HOST, BOB_HOST};
use crate::world::ACME;

/// Ada's machine created `acme`, with Bob a member, so Ada is its only admin. Her `skald
/// delete-account` at a terminal names `acme` on a line with `hand-over-admin` and exits failing
/// by itself: it sends no request that deletes the account and starts no device
/// authorization, and her account and her machine's state stay. Once her machine hands her rights
/// to Bob, the command deletes her account, and Bob's session gets her marked deleted in `acme`.
#[test]
fn as_the_only_admin_of_a_space_with_others_it_deletes_nothing_until_the_rights_are_handed_over() {
    let (world, _front) = fronted();
    let cloud = &world.cloud;
    let ada = world.sign_up(6_120_004_001, "ada-holds-acme-alone", ADA_HOST);
    let bob = world.sign_up(6_120_004_002, "bob-gets-acme", BOB_HOST);
    let space = shared_with(cloud, &ada, &bob);
    let since = cloud.relay.mark();

    let blocked = delete_at_terminal(&world, &ada.machine, ada.handle.as_str(), AtGrant::Refuse);

    let text = &blocked.shown.text;
    assert!(!blocked.shown.success, "{text}");
    let named: Vec<String> = (lines_holding(text, ACME).into_iter())
        .filter(|line| line.contains(HAND_OVER))
        .collect();
    assert!(
        !named.is_empty(),
        "{ACME} is not named with {HAND_OVER}: {text}"
    );
    assert_sent_nothing(cloud, since, "while a space blocks it");
    assert_kept(cloud, &ada);
    assert!(
        !marked_deleted(cloud, &bob, space, &ada),
        "Ada is marked deleted"
    );

    hand_over(&ada, &bob);
    deleted_at_terminal(&world, &ada);

    assert_deleted(cloud, &ada);
    assert!(
        marked_deleted(cloud, &bob, space, &ada),
        "Bob's session does not get Ada marked deleted"
    );
}
