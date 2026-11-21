//! A deleted account's sign-in methods and username: a method's next sign-in signs up as a new
//! account, or is refused while sign-up is closed, and another account can take the username.

use serde_json::json;

use crate::common::device_flow::{DeviceFlow, INTERVAL, text};
use crate::common::devices::machine;
use crate::common::get_url;
use crate::common::sign_in::{cookie_set, github_rows};
use crate::deletion::{assert_deleted, bearer, delete_by_token};
use crate::sessions::Api;

/// The problem type of a refused sign-up.
const SIGN_UP_CLOSED: &str = "https://skald.so/problems/sign-up-closed";

/// `PUT /account/username` of `username` from the device session `token`; gives the status.
fn set_username(flow: &DeviceFlow, token: &str, username: &str) -> u16 {
    let body = serde_json::to_vec(&json!({ "username": username })).expect("JSON");
    let authorization = bearer(token);
    let headers = [("authorization", authorization.as_str())];
    let response = flow.request("put", "/account/username", &headers, Some(&body), true);
    response.status
}

/// Ada deletes her account; her GitHub user's next sign-in signs up a new account, which takes the
/// username her deleted account had, and her old web session stays refused.
#[test]
fn sign_in_with_a_deleted_accounts_method_signs_up_a_new_account() {
    let flow = DeviceFlow::start();
    let api = Api::of(&flow);
    let ada = flow.stand.user(3_303_030_001, "ada-anew-standin");
    let signed_in = machine(&flow, &ada);
    let deleted = flow.who_of(&signed_in.token);

    assert_deleted(&delete_by_token(&api, &signed_in.token));

    let cookie = flow.web_cookie(&ada);
    let response = flow.request("get", "/session", &[("cookie", &cookie)], None, true);
    let new = crate::common::accounts::Who::of(&response);
    assert_ne!(new.account, deleted.account, "a new account");
    assert_eq!(new.username, deleted.username, "the freed username");
    assert_eq!(new.github_login.as_deref(), Some(ada.login.as_str()));
    assert!(!api.web_works(&signed_in.cookie), "the old web session");
}

/// With sign-up closed and only `ada-listed-standin` on the list, Ada signs up with that login,
/// then renames her GitHub user to a login the list does not have; her account still signs in.
/// Once she deletes it, a sign-in with that GitHub user is refused with `sign-up-closed` and
/// creates nothing.
#[test]
fn sign_in_with_a_deleted_accounts_method_is_refused_while_sign_up_is_closed() {
    let flow = DeviceFlow::start_with(&[
        INTERVAL,
        "1",
        "--sign-up",
        "closed",
        "--sign-up-github-logins",
        "ada-listed-standin",
    ]);
    let api = Api::of(&flow);
    let listed = flow.stand.user(3_303_030_011, "ada-listed-standin");
    let signed_in = machine(&flow, &listed);
    let renamed = flow.stand.user(3_303_030_011, "ada-renamed-standin");
    let before = flow.web_cookie(&renamed);
    assert!(
        api.web_works(&before),
        "an existing account, unlisted, signs in"
    );

    assert_deleted(&delete_by_token(&api, &signed_in.token));

    let rows = github_rows(&flow.stand.db, &renamed);
    let (started, callback) = flow.stand.to_callback(&renamed);
    let state = cookie_set(&started, "skald_sign_in");
    let refused = get_url(&callback, &[("cookie", &state)]);
    assert_eq!(refused.status, 403, "{}", text(&refused));
    assert!(
        text(&refused).contains(SIGN_UP_CLOSED),
        "{}",
        text(&refused)
    );
    assert_eq!(
        github_rows(&flow.stand.db, &renamed),
        rows,
        "creates nothing"
    );
}

/// Bob cannot take Ada's username while her account has it; once she deletes her account, he
/// takes it.
#[test]
fn another_account_takes_a_deleted_accounts_username() {
    let flow = DeviceFlow::start();
    let api = Api::of(&flow);
    let ada = machine(&flow, &flow.stand.user(3_303_030_021, "ada-name-standin"));
    let bob = machine(&flow, &flow.stand.user(3_303_030_022, "bob-name-standin"));
    let username = flow.who_of(&ada.token).username;
    assert_eq!(set_username(&flow, &bob.token, &username), 409, "taken");

    assert_deleted(&delete_by_token(&api, &ada.token));

    let status = set_username(&flow, &bob.token, &username);
    assert!(matches!(status, 200 | 204), "set: {status}");
    assert_eq!(flow.who_of(&bob.token).username, username);
}
