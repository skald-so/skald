//! A key change a domain's records show counts in the runs of the resolver that showed it, as the
//! client that starts each run has it, also through a helper another resolver's run started.

use skald_crypto::account_fingerprint;

use crate::doh::Doh;
use crate::helped::Helped;
use crate::world::{ACME, SPACES, World, create_with_member, members, next_with, store};
use crate::{
    ADA_HOST, BOB_HOST, assert_says_none, assert_says_waiting, other_master, seen_at_terminal,
    signed_up_with,
};

/// The setting of the DNS-over-HTTPS resolver's URL.
const RESOLVER: &str = "SKALD_DNS_RESOLVER";

/// A resolver no record is looked up at in these tests.
const ELSEWHERE: &str = "http://127.0.0.1:9/dns-query";

/// Bob's machine looks names up at a stand-in of its own, and `acme`, which it created with Ada
/// as a member, names a domain whose records list Ada with another fingerprint: his `skald
/// members acme` warns of a key change there. A run of `skald spaces` with another resolver starts
/// a helper and prints no line; through that helper, his next `skald spaces`, with his resolver,
/// prints the line, and `skald key-changes` at a terminal shows it, after which that helper's runs
/// print no line.
#[test]
fn a_helper_another_resolvers_run_started_counts_each_runs_own_resolver() {
    let doh = Doh::start();
    let world = World::start();
    let resolver = doh.url();
    let bob = signed_up_with(
        &world,
        (4_188_961_001, "bob-reads-his-own-resolver"),
        BOB_HOST,
        (RESOLVER, &resolver),
    );
    let ada = world.sign_up(4_188_961_002, "ada-listed-at-his-resolver", ADA_HOST);
    let third = create_with_member(&world.cloud, &bob, ACME, &ada);
    let named = next_with(&third, (bob.device, &bob.key), |body| {
        body.policy.domain = Some("corp-r5t2.example".to_owned());
    });
    store(&world.cloud, &bob.token, &named);
    let texts = [(bob.account, bob.master), (ada.account, other_master())]
        .map(|(account, master)| format!("{account} {}", account_fingerprint(account, &master)));
    doh.set_txt("_skald.corp-r5t2.example", 3600, &texts);
    assert!(members(&bob.machine, ACME).success);
    let helped = Helped::new(&bob.machine);

    let elsewhere = helped.run_with(&[(RESOLVER, ELSEWHERE)], &[SPACES]);
    let own = helped.run(&[SPACES]);

    assert!(elsewhere.success, "{elsewhere:?}");
    assert_says_none(&elsewhere.shown());
    assert!(own.success, "{own:?}");
    assert_says_waiting(&own.shown(), Some(1));
    helped.one_helper();
    seen_at_terminal(&bob.machine);
    assert_says_none(&helped.run(&[SPACES]).shown());
}
