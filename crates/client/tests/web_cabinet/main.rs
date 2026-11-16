//! The web app's cabinet with `skald` on the machines: the space switcher lists the account's
//! spaces, its personal space marked as the spaces list marks it, once the account's first machine
//! has signed in with `skald` and signed that space; before that, it says to sign in a machine with
//! `skald`, and the web app creates no personal space. Against a `skald-server` with GitHub sign-in
//! through the GitHub stand-in, on PostgreSQL, behind the recording relay and the front of the web
//! app's harness (`server/tests/common/spa.rs`), on which the machines reach it too; a test plays
//! the person in headless Chrome.

// Each test target uses a part of the shared modules.
#![allow(dead_code)]

#[path = "../../../server/tests/common/browser.rs"]
mod browser;
#[path = "../../../server/tests/common/spa.rs"]
mod spa;

#[path = "../sign_in/cloud.rs"]
mod cloud;
#[path = "../common/mod.rs"]
mod common;
#[path = "../sign_in/machine.rs"]
mod machine;
#[path = "../sign_in/relay.rs"]
mod relay;

use browser::{Browser, Page};
use cloud::Cloud;
use spa::{
    SIGN_IN_GITHUB, Spa, answers_without_secrets, space_options, wait_for_cabinet,
    wait_for_sign_in_page,
};

/// A host of the server's public address that the machines resolve, as Chrome does.
const HOST: &str = "localhost";

/// The request that creates the account's personal space.
const PERSONAL: &str = "/account/personal-space";

/// Whether a line of `page`'s text says to sign in a machine with `skald`.
fn says_to_sign_in_a_machine(page: &Page) -> bool {
    page.text().lines().any(|line| {
        let lower = line.to_lowercase();
        lower.contains("machine") && lower.contains("skald") && lower.contains("sign")
    })
}

/// A new account signed up on the web has no space: the switcher says to sign in a machine with
/// `skald`. Once its first machine has signed in with `skald`, the switcher lists the account's
/// personal space, marked personal. The web app never asked for the personal space, and no answer
/// it got holds a secret.
#[test]
fn switcher_lists_the_personal_space_once_the_first_machine_signs_in() {
    let browser = Browser::launch();
    let spa = Spa::start(&browser, HOST);
    let cloud = Cloud::fronted(HOST, spa.port, 120, 1);
    spa.serve(&browser, &cloud.relay.url);
    let user = cloud.user(3_220_600_001, "Pia-Cabinet-Standin");
    let username = "pia-cabinet-standin";
    let page = browser.page();
    page.record_answers();

    page.goto(&cloud.github.sign_in_url(user.id));
    page.goto(&spa.at("/"));
    wait_for_sign_in_page(&page);
    page.click_settled(SIGN_IN_GITHUB);
    wait_for_cabinet(&page, username);
    spa::wait_until(
        &page,
        "the empty state of the switcher",
        says_to_sign_in_a_machine,
    );

    let (_machine, signed_in) = cloud.first_machine(&page, &user, "pia-laptop");
    assert!(signed_in.success, "{signed_in:?}");

    page.goto(&spa.at("/"));
    wait_for_cabinet(&page, username);
    let options = space_options(&page).expect("the space switcher");
    assert_eq!(options.len(), 1, "the personal space only: {options:?}");
    let personal = options[0].to_lowercase();
    assert!(personal.contains(username), "{options:?}");
    assert!(
        personal.contains("personal"),
        "marked personal: {options:?}"
    );

    let answers = answers_without_secrets(&page);
    for answer in &answers {
        assert!(
            !(answer.method == "POST" && answer.url.ends_with(PERSONAL)),
            "the web app asked for the personal space: {answer:?}"
        );
    }
}
