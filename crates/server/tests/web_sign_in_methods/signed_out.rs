//! A link started without a web session offers sign-in links that return to its `return_to`.

use crate::common::device_flow::text;
use crate::common::send;
use crate::{GITHUB_LINK, Stand};

/// A Link GitHub from the Account page of a browser whose session has ended is refused; the page
/// offers sign-in with each provider, each returning to that page, not to the server's own page of
/// the account's sign-in methods.
#[test]
fn link_without_a_session_offers_sign_in_that_returns_to_its_page() {
    let stand = Stand::launch(&[]);
    let google = &stand.google;
    let url = format!("{}{GITHUB_LINK}?return_to=%2Fprofile", google.server.base());

    let response = send("post", &url, &[("origin", &google.public)], &[]);

    assert_eq!(response.status, 403, "{}", text(&response));
    let page = text(&response);
    for start in ["/auth/github", "/auth/google"] {
        let link = format!("href=\"{start}?return_to=%2Fprofile\"");
        assert!(page.contains(&link), "{link} in {page}");
    }
    assert!(!page.contains("return_to=%2Fsign-in-methods"), "{page}");
}
