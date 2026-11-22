//! Domains: the domain a space's policy names lists a member with another fingerprint, or does not
//! list an admin.

use skald_crypto::account_fingerprint;
use skald_proto::{AccountId, Role};

use crate::doh::Doh;
use crate::world::{
    ACME, MEMBERS, Person, World, create_with_member, members, next_by, next_with, set_role, store,
};
use crate::{
    ADA_HOST, BOB_HOST, assert_says_none, assert_waits_and_shows_once, other_master, signed_up_with,
};

/// The setting of the DNS-over-HTTPS resolver's URL.
const RESOLVER: &str = "SKALD_DNS_RESOLVER";

/// The organization's domain, and the name of its records.
const DOMAIN: &str = "corp-k8m4.example";
const RECORDS: &str = "_skald.corp-k8m4.example";

/// A TTL of the records that outlasts every test, in seconds.
const TTL: u32 = 3600;

/// The record that lists `account` with the fingerprint of the master key `master`.
fn listing(account: AccountId, master: &[u8; 32]) -> String {
    format!("{account} {}", account_fingerprint(account, master))
}

/// Bob's machine looks names up at a DNS-over-HTTPS stand-in of its own from its sign-in on; it
/// creates `acme` and adds Ada as a member. If `ada_admin`, a revision signed by his device makes
/// Ada an admin; the next names [`DOMAIN`]. The stand-in gives the records `records` makes of Bob and
/// Ada, and Bob's `skald members acme` looks them up.
fn named_domain(
    (ids, logins): ([u64; 2], [&str; 2]),
    ada_admin: bool,
    records: impl FnOnce(&Person, &Person) -> Vec<String>,
) -> Named {
    let doh = Doh::start();
    let world = World::start();
    let cloud = &world.cloud;
    let bob = signed_up_with(
        &world,
        (ids[0], logins[0]),
        BOB_HOST,
        (RESOLVER, &doh.url()),
    );
    let ada = world.sign_up(ids[1], logins[1], ADA_HOST);
    let mut latest = create_with_member(cloud, &bob, ACME, &ada);
    assert_says_none(&members(&bob.machine, ACME).shown());
    if ada_admin {
        latest = next_by(&latest, &bob, |list| {
            set_role(list, ada.account, Role::Admin)
        });
        store(cloud, &bob.token, &latest);
    }
    let fourth = next_with(&latest, (bob.device, &bob.key), |body| {
        body.policy.domain = Some(DOMAIN.to_owned());
    });
    store(cloud, &bob.token, &fourth);
    doh.set_txt(RECORDS, TTL, &records(&bob, &ada));

    members(&bob.machine, ACME);

    Named {
        _world: world,
        _doh: doh,
        bob,
        ada,
    }
}

/// What [`named_domain`] sets up: Bob and Ada, and the server and stand-in their runs reach.
struct Named {
    _world: World,
    _doh: Doh,
    bob: Person,
    ada: Person,
}

impl Named {
    /// The key change of Ada waits on Bob's machine and shows once, naming her.
    #[track_caller]
    fn assert_adas_waits_and_shows_once(&self) {
        let names = [self.ada.handle.to_string(), self.ada.account.to_string()];
        assert_waits_and_shows_once(&self.bob.machine, &[MEMBERS, ACME], &names);
    }
}

/// The domain lists Bob with his fingerprint and Ada with that of another key: a key change waits
/// on Bob's machine and shows once, naming Ada.
#[test]
fn a_member_the_domain_lists_with_another_fingerprint_waits_until_seen() {
    let named = named_domain(
        (
            [4_188_601_001, 4_188_601_002],
            ["bob-reads-the-domain", "ada-listed-elsewhere"],
        ),
        false,
        |bob, ada| {
            vec![
                listing(bob.account, &bob.master),
                listing(ada.account, &other_master()),
            ]
        },
    );

    named.assert_adas_waits_and_shows_once();
}

/// Ada is an admin the domain does not list; it lists Bob with his fingerprint: a key change waits
/// on Bob's machine and shows once, naming Ada.
#[test]
fn an_admin_the_domain_does_not_list_waits_until_seen() {
    let named = named_domain(
        (
            [4_188_602_001, 4_188_602_002],
            ["bob-reads-the-domain-again", "ada-admin-unlisted"],
        ),
        true,
        |bob, _| vec![listing(bob.account, &bob.master)],
    );

    named.assert_adas_waits_and_shows_once();
}
