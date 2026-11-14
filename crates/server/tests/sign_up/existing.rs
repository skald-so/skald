//! Closing sign-up keeps every existing account: listed or not, it signs in and links methods as
//! before.

use super::common::browser::Browser;
use super::*;

/// The page of the account's sign-in methods, and its button that links GitHub.
const METHODS_PAGE: &str = "/sign-in-methods";
const LINK_GITHUB: &str = "role=button[name=/link.*github/i]";

/// Accounts made while sign-up was open, none of them listed, sign in after it closes, with GitHub
/// and with Google; one links an unlisted GitHub login, which then signs in to it; a new unlisted
/// person is refused.
#[test]
fn existing_accounts_sign_in_and_link_while_closed() {
    let browser = Browser::launch();
    let google = default_server();
    let ada = github_user(&google, 7_310_000_101, "ada-existing");
    let grace = person(
        &google,
        "110248495921238991101",
        "grace.existing@example.com",
        true,
        None,
    );
    let ada_account = github_signed_up(&google, &browser.page(), &ada);
    let grace_account = google.signed_up(&browser, &grace);

    let lists = Lists {
        logins: "someone-else",
        emails: "someone@else.example",
        domains: "else.example",
    };
    let google = restarted(google, &lists.args(), &[]);

    let page = browser.page();
    let visit = github_visit(&google, &page, &ada);
    assert_eq!(visit.status, Some(200), "{visit:?}: {}", page.text());
    assert_eq!(google.account(&page), ada_account);

    let page = browser.page();
    let visit = google.sign_in(&page, &grace);
    assert_eq!(visit.status, Some(200), "{visit:?}: {}", page.text());
    assert_eq!(google.account(&page), grace_account);

    // Grace links GitHub's gus, whose login is not listed either.
    let gus = github_user(&google, 7_310_000_102, "gus-existing");
    let methods = format!("{}{METHODS_PAGE}", google.public);
    github_session(&page, google.github(), &gus);
    assert_eq!(page.goto(&methods).status, Some(200), "{methods}");
    page.click_settled(LINK_GITHUB);
    let other = browser.page();
    let visit = github_visit(&google, &other, &gus);
    assert_eq!(visit.status, Some(200), "{visit:?}: {}", other.text());
    assert_eq!(
        google.account(&other).account,
        grace_account.account,
        "GitHub is linked"
    );

    let dan = github_user(&google, 7_310_000_103, "dan-existing");
    let page = browser.page();
    let visit = github_visit(&google, &page, &dan);
    assert_refused(
        &google,
        &page,
        &visit,
        "a new unlisted login after the restart",
    );
    assert_no_github_account(&google, &dan, "a new unlisted login after the restart");
}
