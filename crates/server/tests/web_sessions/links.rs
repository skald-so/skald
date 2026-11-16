//! A link of a sign-in method in progress ends with its web session: ended from another web
//! session, or expired.

use skald_github_stand_in::User;

use crate::common::browser::{Browser, Page};
use crate::common::google::{Google, Setup};
use crate::common::sign_in::{github_authorize_link, github_rows, page_get};
use crate::sessions::{Api, Kind, cookie_of, token_of};

/// The page of the account's sign-in methods, its button that starts a GitHub link, and the
/// account's methods in the API.
const METHODS_PAGE: &str = "/sign-in-methods";
const LINK_GITHUB: &str = "role=button[name=/link.*github/i]";
const METHODS: &str = "/account/sign-in-methods";

/// How a link's session ends before GitHub comes back.
#[derive(Clone, Copy, Debug)]
enum End {
    /// Another web session of the account ends it.
    Ended,
    /// Its sign-in is moved back 30 days.
    Expired,
}

/// Starts a link of the GitHub user `user` from the sign-in methods page in `page`'s browser, up to
/// GitHub's page that asks to authorize the app; gives the callback URL it leads to, not opened.
fn held_link(google: &Google, page: &Page, user: &User) -> String {
    let asked = page.goto(&google.github().asking_sign_in_url(user.id));
    assert_eq!(asked.status, Some(200), "{asked:?}");
    page.goto(&format!("{}{METHODS_PAGE}", google.public));
    let started = page.click_settled(LINK_GITHUB);
    github_authorize_link(google.github(), &google.public, page, &started)
}

/// Ada, signed in with Google in one browser, starts linking Bob's GitHub; before GitHub comes
/// back, her session ends as `end` says. GitHub's callback then links nothing: her account keeps
/// Google alone, and Bob's GitHub user has no account.
fn link_of_an_ending_session_links_nothing(end: End, subject: &str, github: u64) {
    let browser = Browser::launch();
    let google = Google::launch(Setup {
        github: true,
        ..Setup::default()
    });
    let api = Api::of_google(&google);
    let ada = google.person(subject, "ada.link@example.test");
    let bob = google.github_user(github, "bob-link-ends-standin", "bob@example.test");
    let linking = browser.page();
    google.sign_in(&linking, &ada);
    let callback = held_link(&google, &linking, &bob);
    let other = browser.page();
    google.sign_in(&other, &ada);
    let cookie = cookie_of(&other);
    let ending = cookie_of(&linking);

    match end {
        End::Ended => {
            let listed = api.list(&cookie);
            let found: Vec<_> = listed
                .iter()
                .filter(|session| session.kind == Kind::Web && !session.current)
                .collect();
            assert_eq!(found.len(), 1, "{listed:#?}");
            assert_eq!(api.end(&cookie, &found[0].id).status, 204);
        }
        End::Expired => api.move_sign_in_back(token_of(&ending), "30 days"),
    }
    assert!(!api.web_works(&ending), "{end:?}");
    linking.goto_settled(&callback);

    let (status, methods) = page_get(&other, &google.public, METHODS);
    assert_eq!(status, 200, "{methods}");
    let providers: Vec<_> = methods["methods"]
        .as_array()
        .unwrap_or_else(|| panic!("{methods}"))
        .iter()
        .map(|method| method["provider"].clone())
        .collect();
    assert_eq!(providers, vec!["google"], "{end:?}: {methods}");
    assert_eq!(
        github_rows(&google.db, &bob),
        Vec::<String>::new(),
        "{end:?}: Bob has no account"
    );
}

#[test]
fn link_in_progress_of_an_ended_session_links_nothing() {
    link_of_an_ending_session_links_nothing(End::Ended, "322250000101", 3_222_500_001);
}

#[test]
fn link_in_progress_of_an_expired_session_links_nothing() {
    link_of_an_ending_session_links_nothing(End::Expired, "322250000111", 3_222_500_011);
}
