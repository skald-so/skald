//! What Bob's device shows of the members of `acme`, whose policy names the domain, against the
//! domain's records: a member listed with its fingerprint shows as listed; one listed with another
//! fingerprint, and an admin not listed, warn once; another member not listed shows nothing. The
//! records change nothing the roster gives.

use skald_crypto::account_fingerprint;
use skald_proto::{AccountId, NonEmpty, Role};

use crate::world::set_role;
use crate::{LONG_TTL, RECORDS, SHORT_TTL, Scene, listed, listed_elsewhere, member_lines, record};

/// The records list Bob and Ada with their fingerprints: `skald members acme` shows the domain and
/// each of them listed by it, warns of neither, and looked the records up at Bob's resolver; a
/// later run shows the same.
#[test]
fn members_listed_with_their_fingerprints_show_as_listed_by_the_domain() {
    let scene = Scene::named(
        [4_187_201_001, 4_187_201_002],
        ["bob-listed-by-the-domain", "ada-listed-by-the-domain"],
        |bob, ada| vec![listed(bob), listed(ada)],
    );

    for _ in 0..2 {
        scene.assert_both_listed(&scene.members());
    }
    assert!(
        scene.doh.txt_queries(RECORDS) > 0,
        "no TXT lookup of {RECORDS}: {:?}",
        scene.doh.queries()
    );
}

/// The records list Bob with his fingerprint and Ada with another, with a TTL that runs out
/// between two runs of `skald members acme`. The first warns once, naming Ada, and the second does
/// not; neither shows her listed, and each shows her with the fingerprint the roster gives.
#[test]
fn a_member_listed_with_another_fingerprint_warns_once() {
    let scene = Scene::new(
        [4_187_202_001, 4_187_202_002],
        ["bob-sees-a-wrong-fingerprint", "ada-listed-with-another"],
    );
    let (bob, ada) = (&scene.bob, &scene.ada);
    scene.name_domain(|_| {});
    scene.records(SHORT_TTL, &[listed(bob), listed_elsewhere(ada)]);
    let hers = account_fingerprint(ada.account, &ada.master);

    let looks = scene.ada_warned_once();

    for shown in &looks {
        let lines = member_lines(shown, ada.handle.as_str(), &scene.handles());
        assert!(
            lines.iter().any(|line| line.contains(&hers)),
            "Ada is not shown with the roster's fingerprint {hers:?}: {shown:?}"
        );
    }
}

/// Revision 4 also makes Ada an admin; the records list only Bob, with a TTL that runs out between
/// two runs of `skald members acme`. The first warns once, naming Ada, and the second does not;
/// neither shows her listed.
#[test]
fn an_admin_the_domain_does_not_list_warns_once() {
    let scene = Scene::new(
        [4_187_203_001, 4_187_203_002],
        ["bob-sees-an-unlisted-admin", "ada-admin-not-listed"],
    );
    let (bob, ada) = (&scene.bob, &scene.ada);
    scene.name_domain(|body| {
        let mut list = body.members.clone().into_vec();
        set_role(&mut list, ada.account, Role::Admin);
        body.members = NonEmpty::from_vec(list).expect("a member");
    });
    scene.records(SHORT_TTL, &[listed(bob)]);

    scene.ada_warned_once();
}

/// The records list only Bob, an admin; Ada is a member that is no admin: `skald members acme`
/// shows Bob listed and Ada not, and warns of nothing naming her.
#[test]
fn a_member_the_domain_does_not_list_shows_nothing() {
    let scene = Scene::new(
        [4_187_204_001, 4_187_204_002],
        ["bob-beside-an-unlisted-member", "ada-member-not-listed"],
    );
    let (bob, ada) = (&scene.bob, &scene.ada);
    scene.name_domain(|_| {});
    scene.records(LONG_TTL, &[listed(bob)]);

    let shown = scene.members();

    scene.assert_listed(&shown, bob, true);
    scene.assert_listed(&shown, ada, false);
    scene.assert_warned(&shown, ada, 0);
}

/// The records list Bob, Ada and an account the roster does not hold: `skald members acme` shows
/// Bob and Ada listed and neither that account's ID nor its fingerprint.
#[test]
fn an_account_the_roster_does_not_hold_is_shown_nowhere() {
    let scene = Scene::new(
        [4_187_205_001, 4_187_205_002],
        ["bob-beside-a-stranger", "ada-beside-a-stranger"],
    );
    let (bob, ada) = (&scene.bob, &scene.ada);
    scene.name_domain(|_| {});
    let stranger = AccountId::generate();
    let master = skald_crypto::SigningKey::generate().public_key();
    scene.records(
        LONG_TTL,
        &[listed(bob), listed(ada), record(stranger, &master)],
    );

    let shown = scene.members();

    scene.assert_listed(&shown, bob, true);
    scene.assert_listed(&shown, ada, true);
    let text = shown.shown();
    assert!(
        !text.contains(&stranger.to_string()),
        "the stranger's account is shown: {shown:?}"
    );
    assert!(
        !text.contains(&account_fingerprint(stranger, &master)),
        "the stranger's fingerprint is shown: {shown:?}"
    );
}
