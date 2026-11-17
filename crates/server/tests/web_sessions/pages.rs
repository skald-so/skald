//! The Access section and signing out in the web app, in the browser.

use crate::access::{
    REVOKE, THIS_SESSION, WHEN_REVOKED, ask_to_sign_out_everywhere_else, confirm_sign_out,
    open_access, revoke_the_other, rows, rows_holding, sign_out,
};
use crate::common::browser::Page;
use crate::common::device_flow::{INTERVAL, Machine, text};
use crate::common::sign_in::SESSION;
use crate::common::spa::{wait_for_cabinet, wait_for_sign_in_page, wait_until};
use crate::kit::{Providers, Web, assert_answers};
use crate::sessions::cookie_of;

/// The web app on a server whose codes poll every second.
fn web() -> Web {
    Web::launch(Providers::Both, &[INTERVAL, "1"])
}

/// A new browser of `web`'s person signed in to the web app with GitHub as `user`, in the cabinet
/// of `username`.
fn signed_in<'a>(web: &'a Web, user: &skald_github_stand_in::User, username: &str) -> Page<'a> {
    let page = web.page();
    web.sign_in_with_github(&page, user, "/");
    wait_for_cabinet(&page, username);
    page
}

/// Whether `web`'s server takes the session that `headers` carry, straight.
fn takes(web: &Web, headers: &[(&str, &str)]) -> bool {
    let response =
        Machine::at(web.stand.server.base()).request("get", SESSION, headers, None, true);
    match response.status {
        200 => true,
        401 => false,
        _ => panic!("GET /session: {}", text(&response)),
    }
}

/// Ada signed in in two browsers: in the second, Access lists both web sessions, its own marked and
/// without Revoke; she revokes the first there. The first browser gets the sign-in page at its
/// next request, and the second still has its cabinet.
#[test]
fn web_session_revoked_from_another_browser_gets_the_sign_in_page_at_its_next_request() {
    let web = web();
    let user = web.github_user(3_222_600_001, "Ada-Revokes-Standin");
    let first = signed_in(&web, &user, "ada-revokes-standin");
    let second = signed_in(&web, &user, "ada-revokes-standin");

    open_access(&second);
    let listed = rows(&second);
    assert_eq!(listed.len(), 2, "{listed:?}");
    revoke_the_other(&second);
    wait_until(&second, "the revoked session gone", |page| {
        rows(page).len() == 1
    });
    assert_eq!(rows_holding(&second, THIS_SESSION).len(), 1);

    first.goto(&web.spa.at("/"));
    wait_for_sign_in_page(&first);
    second.goto(&web.spa.at("/"));
    wait_for_cabinet(&second, "ada-revokes-standin");
    assert_answers(&second);
}

/// Ada's web sessions in two browsers and a machine she approved: Access in the first lists the
/// three, the machine's CLI session expiring "when revoked" and the web sessions not, and Revoke on
/// each but the page's own. "Sign out everywhere else" asks first, saying the machines are signed
/// out too, and ends nothing until confirmed; then only the page's own session is left: the
/// machine's session is refused, and the second browser gets the sign-in page.
#[test]
fn sign_out_everywhere_else_leaves_only_the_current_session() {
    let web = web();
    let user = web.github_user(3_222_600_011, "Ada-Everywhere-Standin");
    let username = "ada-everywhere-standin";
    let first = signed_in(&web, &user, username);
    let second = signed_in(&web, &user, username);
    let machine = Machine::at(web.stand.server.base());
    let grant = machine.authorize();
    let token = machine.approve(&first, &grant);
    let bearer = format!("Bearer {token}");
    first.goto(&web.spa.at("/"));
    wait_for_cabinet(&first, username);

    open_access(&first);
    let listed = rows(&first);
    assert_eq!(listed.len(), 3, "{listed:?}");
    let revoked = rows_holding(&first, WHEN_REVOKED);
    assert_eq!(revoked.len(), 1, "the CLI session: {listed:?}");
    assert!(
        !revoked[0].to_lowercase().contains(THIS_SESSION),
        "{listed:?}"
    );
    assert_eq!(first.count(REVOKE), 2, "{listed:?}");

    let asked = ask_to_sign_out_everywhere_else(&first);
    assert!(asked.to_lowercase().contains("machine"), "{asked:?}");
    assert!(takes(&web, &[("authorization", &bearer)]), "ended unasked");
    confirm_sign_out(&first);

    wait_until(&first, "only this session", |page| {
        let left = rows(page);
        left.len() == 1 && left[0].to_lowercase().contains(THIS_SESSION)
    });
    assert!(!takes(&web, &[("authorization", &bearer)]), "the machine");
    second.goto(&web.spa.at("/"));
    wait_for_sign_in_page(&second);
    first.goto(&web.spa.at("/"));
    wait_for_cabinet(&first, username);
    assert_answers(&first);
}

/// Sign out in the account menu ends the page's web session and shows the sign-in page: the server
/// refuses its cookie from then on, and Ada's other browser keeps its cabinet.
#[test]
fn sign_out_in_the_account_menu_ends_this_web_session() {
    let web = web();
    let user = web.github_user(3_222_600_021, "Ada-Menu-Standin");
    let username = "ada-menu-standin";
    let page = signed_in(&web, &user, username);
    let other = signed_in(&web, &user, username);
    let cookie = cookie_of(&page);

    sign_out(&page);

    wait_for_sign_in_page(&page);
    assert!(
        !takes(&web, &[("cookie", &cookie)]),
        "the signed-out session"
    );
    page.goto(&web.spa.at("/"));
    wait_for_sign_in_page(&page);
    other.goto(&web.spa.at("/"));
    wait_for_cabinet(&other, username);
    assert!(takes(&web, &[("cookie", &cookie_of(&other))]));
    assert_answers(&page);
}
