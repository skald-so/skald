//! A device follows a space another device of its account created after it was linked, from that
//! device's anchors object, and gets into its default channel; an anchors object that names an
//! earlier revision of a space it follows changes nothing.

use crate::joined::{
    Account, anchored, anchors_object, follows, give_anchors_too, next_run, warn_of,
};
use crate::kit::{assert_members, create_space, group, latest, received, rosters};
use crate::reading::{assert_history, sends, shown_history};
use crate::world::{ACME, GLOBEX, next_by, space_id, store};

/// Ada's second machine is linked; then her first machine creates `acme` and says a line in its
/// default channel. The second machine's next `skald recv` follows `acme`: `skald spaces` lists
/// it, and the second device's anchors object names `acme` at its latest revision. The default
/// channel's group then has Ada's two devices. Her first machine says another line there and runs;
/// the second machine's `skald history` of the default channel shows both lines, oldest first.
#[test]
fn device_follows_a_space_its_account_created_after_it_was_linked_and_reads_its_channel() {
    let account = Account::new(4_811_406_001, "ada-follows-on-her-laptop", &[]);
    let (cloud, ada, laptop) = (account.cloud(), &account.ada, &account.laptop);
    let acme = create_space(cloud, ada, ACME);
    let mut said = vec![sends(
        ada,
        (ACME, acme.default),
        "ada drafts the launch plan",
    )];

    let (listed, _) = next_run(&laptop.machine);

    assert!(follows(&listed, ACME), "acme is not followed: {listed:?}");
    let last = latest(cloud, &ada.token, acme.id);
    assert_eq!(
        anchored(cloud, laptop, acme.id),
        Some(last.hash()),
        "the second device's anchors object does not name acme's latest revision"
    );
    let held = group(cloud, &ada.token, acme.id, acme.default);
    assert_members(&held.members, &[ada.device, laptop.device]);
    said.push(sends(
        ada,
        (ACME, acme.default),
        "ada asks her laptop to review the plan",
    ));
    received(&ada.machine);
    let shown = shown_history(&laptop.machine, acme.default);
    assert_history(&shown, acme.default, &said, &[]);
}

/// Ada's first machine creates `acme` before her second machine is linked, so both follow its
/// revision 2, then `globex`; her third device is linked, and the second machine runs. The relay
/// gives an anchors object of the third device, signed with its key, that names `acme` at revision
/// 1, and a revision 3 of `acme`, signed as her first device, is stored. The second machine's next
/// run follows `globex`, still follows `acme`, warns of nothing about `acme`, and its anchors
/// object names `acme` at revision 3.
#[test]
fn anchors_object_naming_an_earlier_revision_of_a_followed_space_changes_nothing() {
    let account = Account::new(4_811_406_011, "ada-keeps-her-revision", &[ACME]);
    let (cloud, ada, laptop) = (account.cloud(), &account.ada, &account.laptop);
    let acme = space_id(cloud, &ada.token, ACME);
    create_space(cloud, ada, GLOBEX);
    let tablet = account.tablet();
    // The second machine warns of the new device in each of its spaces, `acme` too, at this run.
    received(&laptop.machine);
    let held = rosters(cloud, &ada.token, acme);
    assert_eq!(held.len(), 2, "acme's revisions: {held:?}");
    let earlier = anchors_object(
        ada.account,
        (tablet.device, &tablet.key),
        &[(acme, &held[0])],
    );
    give_anchors_too(cloud, &[earlier]);
    let third = next_by(&held[1], ada, |_| {});
    store(cloud, &ada.token, &third);

    let (listed, said) = next_run(&laptop.machine);

    assert!(
        follows(&listed, GLOBEX),
        "globex is not followed: {listed:?}"
    );
    assert!(
        follows(&listed, ACME),
        "acme is no longer followed: {listed:?}"
    );
    assert!(!warn_of(&said, (ACME, acme)), "a warning of acme: {said:?}");
    assert_eq!(
        anchored(cloud, laptop, acme),
        Some(third.hash()),
        "the second device's anchors object does not name revision 3 of acme"
    );
}
