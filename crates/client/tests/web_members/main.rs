//! The web app's Members section with `skald` on the machines: the members of the space the
//! switcher chose, with their roles, as the space's latest revision gives them. Against a
//! `skald-server` with GitHub sign-in through the GitHub stand-in, on PostgreSQL, behind the
//! recording relay and the front of the web app's harness (`server/tests/common/spa.rs`), on which
//! the machines reach it too; a test plays the person in headless Chrome. The fixture revision that
//! adds a second account is signed with the key of the admin's device, read from its secrets file,
//! and stored through the API, as in the client's tests of roster revisions.
//!
//! The web app, as these tests use it, beside `common/spa.rs`: the side menu's link "Members"
//! opens the section for the space the switcher chose, which lists the space's members in a table,
//! a row per member that holds its handle and says its role, "Admin" or "Member". Choosing another
//! space in the switcher, an option naming it, shows that space's members. It gets them from
//! `GET /spaces/{space}/members`.

// Each test target uses a part of the shared modules.
#![allow(dead_code)]

// The person's browser, the web app's harness and the rows of its tables.
#[path = "../../../server/tests/web_sessions/access.rs"]
mod access;
#[path = "../../../server/tests/common/browser.rs"]
mod browser;
#[path = "../../../server/tests/common/spa.rs"]
mod spa;

// The server, the relay, the machines, their keys and the revisions they sign, of the tests of
// roster revisions.
#[path = "../sign_in/cloud.rs"]
mod cloud;
#[path = "../common/mod.rs"]
mod common;
#[path = "../device_keys/keys.rs"]
mod keys;
#[path = "../sign_in/machine.rs"]
mod machine;
#[path = "../sign_in/relay.rs"]
mod relay;
#[path = "../roster_revisions/world.rs"]
mod world;

// The only-admin warning.
mod only_admin;

use access::{press, rows_holding};
use browser::{Browser, Page};
use cloud::Cloud;
use skald_proto::Role;
use spa::{Spa, answers_without_secrets, space_options, wait_for_cabinet, wait_until};
use world::{ACME, ADMIN, MEMBER, World, create, next_by, space_id, store};

/// A host of the server's public address that the machines resolve, as Chrome does.
const HOST: &str = "localhost";
/// How long a machine's code works, in seconds.
const CODE_LIFETIME: u64 = 300;

/// The roles a row of `page` that holds `handle` says, in any case; empty unless one row holds it.
fn roles_of(page: &Page, handle: &str) -> Vec<&'static str> {
    let rows = rows_holding(page, handle);
    if rows.len() != 1 {
        return Vec::new();
    }
    let row = rows[0].to_lowercase();
    [ADMIN, MEMBER]
        .into_iter()
        .filter(|role| row.contains(role))
        .collect()
}

/// Chooses the space `name` in the switcher of the cabinet in `page`.
fn choose(page: &Page, name: &str) {
    let options = space_options(page).expect("the space switcher");
    assert!(
        options.iter().any(|option| option.contains(name)),
        "{name} in the switcher: {options:?}"
    );
    press(page, None, "[role=option]", &format!("\\b{name}\\b"));
}

/// Waits until the Members section in `page` shows each of `shown`, a handle and its role, in a row
/// of its own, and no row holds a handle of `absent`.
fn wait_for_members(page: &Page, space: &str, shown: &[(&str, &str)], absent: &[&str]) {
    let what = format!("Members of {space} listing {shown:?} and not {absent:?}");
    wait_until(page, &what, |page| {
        shown
            .iter()
            .all(|(handle, role)| roles_of(page, handle) == [*role])
            && absent
                .iter()
                .all(|handle| rows_holding(page, handle).is_empty())
    });
}

/// Ida's laptop signs in with `skald` as her account's first device, approved in the browser she
/// then opens the web app in, and signs her personal space: in Members, that space lists her as its
/// admin. Her laptop then creates `acme`, and a revision it signs adds Bob, who signed up with a
/// machine of his own, as a member: choosing `acme` lists Ida as its admin and Bob as a member;
/// choosing her personal space again lists her alone, and `acme` again both. The web app got each
/// space's members from the server, and no answer it got holds a secret.
#[test]
fn members_lists_the_chosen_spaces_members_with_their_roles() {
    let browser = Browser::launch();
    let page = browser.page();
    page.record_answers();
    let spa = Spa::start(&browser, HOST);
    let world = World {
        cloud: Cloud::fronted(HOST, spa.port, CODE_LIFETIME, 1),
        browser: Browser::launch(),
    };
    let cloud = &world.cloud;
    spa.serve(&browser, &cloud.relay.url);

    let user = cloud.user(4_225_500_001, "Ida-Roles-Standin");
    let (laptop, signed_in) = cloud.first_machine(&page, &user, "ida-laptop");
    assert!(signed_in.success, "{signed_in:?}");
    let ida = world.person(user, laptop, "ida-laptop");
    let personal = ida.handle.to_string();
    let open = |space: &str| {
        page.goto(&spa.at("/"));
        wait_for_cabinet(&page, &personal);
        press(&page, None, "a, [role=link]", "^members\\b");
        choose(&page, space);
    };
    open(&personal);
    wait_for_members(&page, &personal, &[(&personal, ADMIN)], &[]);

    let second = create(cloud, &ida, ACME);
    let bob = world.sign_up(4_225_500_002, "Bob-Roles-Standin", "bob-laptop");
    let third = next_by(&second, &ida, |members| {
        members.push(bob.member(Role::Member));
    });
    store(cloud, &ida.token, &third);
    let bob_handle = bob.handle.to_string();
    let both = [(personal.as_str(), ADMIN), (bob_handle.as_str(), MEMBER)];

    open(ACME);
    wait_for_members(&page, ACME, &both, &[]);
    choose(&page, &personal);
    wait_for_members(&page, &personal, &[(&personal, ADMIN)], &[&bob_handle]);
    choose(&page, ACME);
    wait_for_members(&page, ACME, &both, &[]);

    // The paths of the web app's `GET` requests that the server answered with `200`.
    let answered: Vec<String> = answers_without_secrets(&page)
        .into_iter()
        .filter(|answer| (answer.method.as_str(), answer.status) == ("GET", 200))
        .map(|answer| answer.url.split('?').next().unwrap_or_default().to_owned())
        .collect();
    for name in [personal.as_str(), ACME] {
        let members = format!("/spaces/{}/members", space_id(cloud, &ida.token, name));
        let got = answered.iter().any(|url| url.ends_with(&members));
        assert!(got, "{name}: no answer to GET {members}: {answered:?}");
    }
}
