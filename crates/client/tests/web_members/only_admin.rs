//! The only-admin warning in Members: the only admin of a space with another member is warned and
//! told the command that makes a member an admin; a member is not, nor is an admin once a second
//! admin is made. Each person uses a page of their own; the second admin is made with `skald
//! make-admin <space> <handle>` on the admin's machine.
//!
//! The web app, as these tests use it, beside the tests above: when the session's account is the
//! only admin of the chosen space and the space has another member, Members shows an alert,
//! `[role=alert]`, that holds `skald make-admin`, `only admin` and `recovery key`, in any case.
//! Otherwise no text of the page holds `make-admin`.

use serde_json::Value;
use skald_proto::Role;

use crate::access::press;
use crate::browser::{Browser, Page};
use crate::cloud::Cloud;
use crate::spa::{Spa, wait_for_cabinet, wait_until};
use crate::world::{ACME, ADMIN, MEMBER, World, create, next_by, run, store};
use crate::{CODE_LIFETIME, HOST, choose, wait_for_members};

/// The command that makes a member an admin, as the warning names it.
const MAKE_ADMIN: &str = "make-admin";
/// What the warning says, the command among it.
const WARNING_SAYS: [&str; 3] = ["skald make-admin", "only admin", "recovery key"];

/// The text of each alert of `page`, in lower case.
fn alerts(page: &Page) -> Vec<String> {
    let found = page.evaluate(
        r#"[...document.querySelectorAll("[role=alert]")].map((alert) => alert.innerText)"#,
    );
    let Value::Array(texts) = found else {
        panic!("the alerts' text: {found}");
    };
    (texts.iter())
        .map(|text| text.as_str().unwrap_or_default().to_lowercase())
        .collect()
}

/// Whether an alert of `page` holds all that the warning says.
fn warns(page: &Page) -> bool {
    alerts(page)
        .iter()
        .any(|alert| WARNING_SAYS.iter().all(|part| alert.contains(part)))
}

/// Opens Members of the space `name` in `page`, the web app of `username`.
fn open_members(page: &Page, spa: &Spa, username: &str, name: &str) {
    page.goto(&spa.at("/"));
    wait_for_cabinet(page, username);
    press(page, None, "a, [role=link]", "^members\\b");
    choose(page, name);
}

/// Fails if any text of `page` names the command that makes a member an admin; `who` names the
/// page's person.
#[track_caller]
fn assert_no_warning(page: &Page, who: &str) {
    let text = page.text();
    assert!(
        !text.to_lowercase().contains(MAKE_ADMIN),
        "{who}: an only-admin warning: {text:?}; alerts {:?}",
        alerts(page)
    );
}

/// Ida's laptop signs in with `skald` as her account's first device, approved in her browser, and
/// creates `acme`; Bob signs up with a laptop of his own, approved in a browser of his. A revision
/// Ida's laptop signs adds Bob to `acme` as a member: Ida's Members of `acme` lists her as its admin
/// and Bob as a member and warns her, naming `skald make-admin`; Bob's lists the same and does not.
/// Once `skald make-admin acme <bob>` on Ida's laptop makes Bob an admin, Ida's Members of `acme`
/// lists both as admins and does not warn.
#[test]
fn members_warns_the_only_admin_until_a_second_admin_is_made() {
    let browser = Browser::launch();
    let spa = Spa::start(&browser, HOST);
    let world = World {
        browser: Browser::launch(),
        cloud: Cloud::fronted(HOST, spa.port, CODE_LIFETIME, 1),
    };
    let (ida_page, bob_page) = (browser.page(), browser.page());
    let cloud = &world.cloud;
    spa.serve(&browser, &cloud.relay.url);

    let mut people = Vec::new();
    for (page, id, login, host) in [
        (
            &ida_page,
            4_302_200_001,
            "Ida-Only-Admin-Standin",
            "ida-laptop",
        ),
        (
            &bob_page,
            4_302_200_002,
            "Bob-Joins-Ida-Standin",
            "bob-laptop",
        ),
    ] {
        let user = cloud.user(id, login);
        let (laptop, signed_in) = cloud.first_machine(page, &user, host);
        assert!(signed_in.success, "{login}: {signed_in:?}");
        people.push(world.person(user, laptop, host));
    }
    let [ida, bob] = <[_; 2]>::try_from(people).ok().expect("two people");
    let (ida_handle, bob_handle) = (ida.handle.to_string(), bob.handle.to_string());

    let second = create(cloud, &ida, ACME);
    let third = next_by(&second, &ida, |members| {
        members.push(bob.member(Role::Member));
    });
    store(cloud, &ida.token, &third);
    let before = [(ida_handle.as_str(), ADMIN), (bob_handle.as_str(), MEMBER)];

    open_members(&ida_page, &spa, &ida_handle, ACME);
    wait_for_members(&ida_page, ACME, &before, &[]);
    let what = "Ida's Members of acme warning that she is its only admin";
    wait_until(&ida_page, what, warns);

    open_members(&bob_page, &spa, &bob_handle, ACME);
    wait_for_members(&bob_page, ACME, &before, &[]);
    assert_no_warning(&bob_page, "Bob, a member");

    let made = run(&ida.machine, &[MAKE_ADMIN, ACME, &bob_handle]);
    assert!(made.success, "{MAKE_ADMIN} {ACME} {bob_handle}: {made:?}");

    open_members(&ida_page, &spa, &ida_handle, ACME);
    let after = [(ida_handle.as_str(), ADMIN), (bob_handle.as_str(), ADMIN)];
    wait_for_members(&ida_page, ACME, &after, &[]);
    assert_no_warning(&ida_page, "Ida, with Bob a second admin");
}
