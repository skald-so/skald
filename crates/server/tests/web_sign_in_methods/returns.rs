//! A link returns to the page that started it, as its `return_to` names it; never to another site.

use serde_json::json;

use crate::common::browser::Page;
use crate::common::google::Google;
use crate::common::sign_in::github_session;
use crate::common::spa::wait_until;
use crate::{BOTH, GITHUB_LINK, Stand, Via, listed, sorted};

/// Starts a GitHub link of `page`'s web session from a page of `google`'s server, its form posting
/// to `/auth/github/link` with `return_to` in the query as `return_to` gives it; waits until the
/// browser has left GitHub and the link's pages and gives where it ended.
fn link_returning_to(google: &Google, page: &Page, return_to: &str) -> String {
    page.goto(&format!("{}/health", google.public));
    let mut action = url::Url::parse(&format!("{}{GITHUB_LINK}", google.public)).expect("a URL");
    action.query_pairs_mut().append_pair("return_to", return_to);
    page.evaluate(&format!(
        r#"(() => {{
            const form = document.createElement("form");
            form.method = "post";
            form.action = {action};
            const button = document.createElement("button");
            button.id = "start-link";
            button.textContent = "Link GitHub";
            form.append(button);
            document.body.append(form);
        }})()"#,
        action = json!(action.as_str()),
    ));
    page.click_settled("#start-link");
    let github = google.github().url().to_string();
    let left = |page: &Page| {
        let at = page.evaluate("location.href");
        let at = at.as_str().unwrap_or_default().to_owned();
        let on_link = at.contains("/auth/github");
        (!at.starts_with(&github) && !on_link).then_some(at)
    };
    wait_until(page, "the end of the link", |page| left(page).is_some());
    left(page).expect("where the link ended")
}

/// Ada, signed in with Google, links GitHub from a page that names `/account?tab=methods` as the
/// page to return to: the link ends there, on the server's origin. Links of the same method again,
/// whose `return_to` names a page of another site by its full address or one without its scheme,
/// end on a page of the server's own origin. Her account has both methods.
#[test]
fn link_returns_to_the_page_that_started_it_and_never_to_another_site() {
    let stand = Stand::launch(&[]);
    let google = &stand.google;
    let (ada, user) = stand.person(401, "ada-returns");
    let session = stand.signed_in(Via::Google(&ada));
    let page = &session.page;
    github_session(page, google.github(), &user);

    let ended = link_returning_to(google, page, "/account?tab=methods");
    assert_eq!(ended, format!("{}/account?tab=methods", google.public));

    for away in ["http://evil.test/account", "//evil.test/account"] {
        let ended = link_returning_to(google, page, away);
        assert!(
            ended.starts_with(&format!("{}/", google.public)),
            "{away}: ended at {ended}"
        );
    }
    assert_eq!(sorted(&listed(&stand.api(), &session.cookie)), BOTH);
}
