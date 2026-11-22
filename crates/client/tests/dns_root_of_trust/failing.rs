//! A resolver that fails leaves the members not checked and fails no command, and a later run
//! checks them; a lookup is made again after the records' TTL.

use std::thread;

use crate::doh::Fault;
use crate::{PAST_SHORT_TTL, RECORDS, SHORT_TTL, Scene, listed, listed_elsewhere};
use crate::{holds_domain, warnings_naming};

/// The records list Bob with his fingerprint and Ada with another.
fn ada_listed_elsewhere(ids: [u64; 2], logins: [&str; 2]) -> Scene {
    Scene::named(ids, logins, |bob, ada| {
        vec![listed(bob), listed_elsewhere(ada)]
    })
}

/// Fails unless `skald members acme`, run while the resolver fails, succeeds, shows the domain,
/// shows neither Bob nor Ada listed and warns of nothing naming either or the domain.
#[track_caller]
fn assert_not_checked(scene: &Scene) {
    let shown = scene.members();
    scene.assert_listed(&shown, &scene.bob, false);
    scene.assert_listed(&shown, &scene.ada, false);
    scene.assert_warned(&shown, &scene.ada, 0);
    scene.assert_warned(&shown, &scene.bob, 0);
    let about_domain: Vec<String> = (crate::world::warnings(&shown).into_iter())
        .filter(|line| holds_domain(line))
        .collect();
    assert!(
        about_domain.is_empty(),
        "a warning about the domain: {shown:?}"
    );
}

/// Fails unless `skald members acme` now checks the members: it shows Bob listed and warns once,
/// naming Ada.
#[track_caller]
fn assert_checked(scene: &Scene) {
    let shown = scene.members();
    scene.assert_listed(&shown, &scene.bob, true);
    scene.assert_listed(&shown, &scene.ada, false);
    assert_eq!(
        warnings_naming(&shown, scene.ada.handle.as_str()).len(),
        1,
        "no warning naming Ada once the resolver answers: {shown:?}"
    );
}

/// With every connection to Bob's resolver dropped, `skald members acme` succeeds and checks
/// nobody, twice; once the resolver answers, the next run checks the members.
#[test]
fn with_the_resolver_unreachable_nothing_warns_and_a_later_run_checks() {
    let scene = ada_listed_elsewhere(
        [4_187_301_001, 4_187_301_002],
        ["bob-without-a-resolver", "ada-unchecked-unreachable"],
    );
    scene.doh.fail(Fault::Unreachable);

    assert_not_checked(&scene);
    assert_not_checked(&scene);

    scene.doh.recover();
    assert_checked(&scene);
}

/// With Bob's resolver answering every request with `503`, then every query with SERVFAIL,
/// `skald members acme` succeeds and checks nobody each time; once the resolver answers, the next
/// run checks the members.
#[test]
fn with_the_resolver_answering_errors_nothing_warns_and_a_later_run_checks() {
    let scene = ada_listed_elsewhere(
        [4_187_302_001, 4_187_302_002],
        ["bob-with-a-failing-resolver", "ada-unchecked-on-errors"],
    );

    scene.doh.fail(Fault::Status(503));
    assert_not_checked(&scene);
    scene.doh.fail(Fault::ServFail);
    assert_not_checked(&scene);

    scene.doh.recover();
    assert_checked(&scene);
}

/// The records list Bob and Ada with their fingerprints, with a short TTL: `skald members acme`
/// shows Ada listed. The records then list her with another fingerprint; once the TTL has run out,
/// the next run looks them up again, warns once naming her and shows her not listed.
#[test]
fn after_the_records_ttl_a_later_run_looks_them_up_again() {
    let scene = Scene::new(
        [4_187_303_001, 4_187_303_002],
        ["bob-looks-up-again", "ada-whose-record-changes"],
    );
    let (bob, ada) = (&scene.bob, &scene.ada);
    scene.name_domain(|_| {});
    scene.records(SHORT_TTL, &[listed(bob), listed(ada)]);
    let before = scene.members();
    scene.assert_listed(&before, ada, true);
    let looked_up = scene.doh.txt_queries(RECORDS);

    scene.records(SHORT_TTL, &[listed(bob), listed_elsewhere(ada)]);
    thread::sleep(PAST_SHORT_TTL);
    let after = scene.members();

    assert!(
        scene.doh.txt_queries(RECORDS) > looked_up,
        "not looked up again after the TTL: {:?}",
        scene.doh.queries()
    );
    scene.assert_listed(&after, ada, false);
    scene.assert_warned(&after, ada, 1);
}
