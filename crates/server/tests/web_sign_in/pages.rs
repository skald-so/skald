//! The web app in the browser: its sign-in page and the cabinet it signs a person in to.

use crate::common::browser::Visit;
use crate::common::device_flow::Machine;
use crate::common::google::HOST;
use crate::common::sign_in::{github_session, is_uuid_v7, param};
use crate::common::spa::{
    CODE_FIELD, CONTINUE, SIGN_IN_GITHUB, sign_in_buttons, sign_up_note, wait_for_cabinet,
    wait_for_sign_in_page,
};
use crate::kit::{Providers, Web, assert_answers};

/// Checks that `visit`, a sign-in's, ended on the web app's page `expected`.
#[track_caller]
fn assert_back_at(visit: &Visit, expected: &str) {
    assert_eq!(visit.error, None, "{visit:?}");
    assert_eq!(visit.url, expected, "back on the page: {visit:?}");
}

/// A new GitHub user signs up in one click: the sign-in page's GitHub button brings them back to
/// the web app in the cabinet of their new account, its footer showing their username, their
/// GitHub login and GitHub as the method; the session names GitHub as its method. No answer the
/// web app got holds a secret, and each conforms to the spec.
#[test]
fn new_github_user_signs_up_in_one_click_and_lands_in_the_cabinet() {
    let web = Web::start();
    let user = web.github_user(3_220_200_001, "Ada-Web-Standin");
    let page = web.page();

    let visit = web.sign_in_with_github(&page, &user, "/");
    assert_back_at(&visit, &web.spa.at("/"));
    wait_for_cabinet(&page, "ada-web-standin");
    let text = page.text();
    assert!(text.contains("Ada-Web-Standin"), "the login: {text:?}");
    assert!(text.contains("GitHub"), "the method: {text:?}");
    assert_answers(&page);

    let session = web.session(&page);
    assert!(
        is_uuid_v7(session["account"].as_str().unwrap_or_default()),
        "{session}"
    );
    assert_eq!(session["username"], "ada-web-standin", "{session}");
    assert_eq!(session["github_login"], "Ada-Web-Standin", "{session}");
    assert_eq!(session["sign_in_method"], "github", "{session}");
}

/// A new Google user signs up in one click and lands in the cabinet of their new account, its
/// footer showing their username and Google as the method, and no GitHub login.
#[test]
fn new_google_user_signs_up_in_one_click_and_lands_in_the_cabinet() {
    let web = Web::start();
    let person = web.stand.person("220200101", "grace-web@example.test");
    let page = web.page();

    let visit = web.sign_in_with_google(&page, &person, "/");
    assert_back_at(&visit, &web.spa.at("/"));
    wait_for_cabinet(&page, "grace-web");
    let text = page.text();
    assert!(text.contains("Google"), "the method: {text:?}");
    assert!(!text.contains("GitHub"), "no GitHub login: {text:?}");
    assert_answers(&page);

    let session = web.session(&page);
    assert_eq!(session["username"], "grace-web", "{session}");
    assert_eq!(session.get("github_login"), None, "{session}");
    assert_eq!(session["sign_in_method"], "google", "{session}");
}

/// A second sign-in, in another browser, reaches the account the first one created, with GitHub as
/// with Google.
#[test]
fn second_sign_in_reaches_the_same_account() {
    let web = Web::start();
    let user = web.github_user(3_220_200_011, "Lin-Web-Standin");
    let person = web.stand.person("220200111", "mae-web@example.test");

    let mut accounts = Vec::new();
    for _ in 0..2 {
        let page = web.page();
        web.sign_in_with_github(&page, &user, "/");
        wait_for_cabinet(&page, "lin-web-standin");
        let github = web.session(&page)["account"].clone();
        let page = web.page();
        web.sign_in_with_google(&page, &person, "/");
        wait_for_cabinet(&page, "mae-web");
        let google = web.session(&page)["account"].clone();
        assert_ne!(github, google, "two people, two accounts");
        accounts.push((github, google));
    }
    assert_eq!(
        accounts[0], accounts[1],
        "the second sign-ins reach the same accounts"
    );
}

/// A page of the web app opened without a session shows the sign-in page, and the sign-in returns
/// to that page, its query kept, in the cabinet.
#[test]
fn page_opened_without_a_session_opens_again_after_sign_in() {
    let web = Web::start();
    let user = web.github_user(3_220_200_021, "Kay-Web-Standin");
    let page = web.page();
    let opened = "/?from=a-link&tab=2";

    let visit = web.sign_in_with_github(&page, &user, opened);
    assert_back_at(&visit, &web.spa.at(opened));
    wait_for_cabinet(&page, "kay-web-standin");
}

/// The sign-in page offers a button for each provider the server has settings for, and only those.
#[test]
fn sign_in_page_offers_only_the_providers_the_server_has() {
    for (providers, buttons) in [
        (Providers::Google, (0, 1)),
        (Providers::GitHub, (1, 0)),
        (Providers::Both, (1, 1)),
    ] {
        let web = Web::launch(providers, &[]);
        let page = web.page();
        page.goto(&web.spa.at("/"));
        wait_for_sign_in_page(&page);
        assert_eq!(
            sign_in_buttons(&page),
            buttons,
            "{providers:?}: (GitHub, Google)"
        );
        assert_answers(&page);
    }
}

/// The sign-in page says that a first sign-in creates an account; while sign-up is closed, that it
/// does so only for the people the server lists.
#[test]
fn sign_in_page_says_whether_sign_up_is_open() {
    for (args, closed) in [
        (vec![], false),
        (vec!["--sign-up", "open"], false),
        (
            vec![
                "--sign-up",
                "closed",
                "--sign-up-github-logins",
                "listed-web-standin",
            ],
            true,
        ),
    ] {
        let web = Web::launch(Providers::Both, &args);
        let page = web.page();
        page.goto(&web.spa.at("/"));
        wait_for_sign_in_page(&page);
        let note = sign_up_note(&page);
        assert_eq!(
            note.to_lowercase().contains("only"),
            closed,
            "{args:?}: {note:?}"
        );
        assert!(
            !page.text().contains("listed-web-standin"),
            "the page names no list"
        );
    }
}

/// The terminal box of the sign-in page opens the verification page with the code typed in it,
/// which, once the person signs in there, has the code filled in.
#[test]
fn terminal_box_opens_the_verification_page_with_its_code() {
    let web = Web::start();
    let user = web.github_user(3_220_200_031, "Ian-Web-Standin");
    let grant = Machine::at(web.stand.server.base()).authorize();
    let page = web.page();
    github_session(&page, web.stand.github(), &user);

    page.goto(&web.spa.at("/"));
    wait_for_sign_in_page(&page);
    page.fill(CODE_FIELD, &grant.user_code);
    let visit = page.click(CONTINUE);
    assert_eq!(visit.status, Some(200), "{visit:?}");
    let opened = url::Url::parse(&visit.url).expect("a URL");
    assert_eq!(opened.host_str(), Some(HOST), "{visit:?}");
    assert_eq!(opened.path(), "/device", "{visit:?}");
    assert_eq!(
        param(&visit.url, "user_code"),
        Some(grant.user_code.clone())
    );

    page.click_settled(SIGN_IN_GITHUB);
    assert_eq!(
        page.values(CODE_FIELD),
        vec![grant.user_code.clone()],
        "the code filled in: {}",
        page.text()
    );
}
