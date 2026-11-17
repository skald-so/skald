//! A removed method's web sessions end at once; the account's other sessions, device sessions
//! approved through it included, stay and are listed.

use crate::sessions::Kind;
use crate::{Provider, Stand, Via, assert_removed, listed, machine_of, remove, sorted};

/// Ada, with Google and GitHub, has two web sessions signed in with Google, one with GitHub, and a
/// machine she approved in a Google-signed browser. From the GitHub session she removes Google: at
/// once both Google sessions are refused, while the GitHub session and the machine's session work.
/// The GitHub session's list of sessions holds itself and the machine's, and neither Google one.
#[test]
fn removal_ends_the_web_sessions_the_method_signed_in() {
    let stand = Stand::launch(&[]);
    let api = stand.api();
    let ada = stand.with_both(201, "ada-ends");
    let second = stand.signed_in(Via::Google(&ada.person));
    let by_github = stand.signed_in(Via::GitHub(&ada.user));
    let token = machine_of(&stand.google, &ada.session.page);
    let cookie = &by_github.cookie;
    let all = api.list(cookie);
    assert_eq!(all.len(), 4, "three web sessions and a machine's: {all:#?}");

    assert_removed(&remove(&api, cookie, &ada.id(Provider::Google)));

    for ended in [&ada.session.cookie, &second.cookie] {
        assert!(!api.web_works(ended), "a Google session");
    }
    assert!(api.web_works(cookie), "the GitHub session");
    assert!(api.device_works(&token), "the machine's session");
    let left = api.list(cookie);
    let kinds: Vec<(Kind, bool)> = left
        .iter()
        .map(|session| (session.kind, session.current))
        .collect();
    assert_eq!(kinds.len(), 2, "{left:#?}");
    assert!(kinds.contains(&(Kind::Web, true)), "{left:#?}");
    assert!(kinds.contains(&(Kind::Device, false)), "{left:#?}");
}

/// Ada removes Google from the web session she signed in with Google: the removal goes through and
/// ends that session at once, and a GitHub sign-in then reaches her account, which lists GitHub
/// alone.
#[test]
fn removing_the_method_of_this_session_ends_it() {
    let stand = Stand::launch(&[]);
    let api = stand.api();
    let ada = stand.with_both(211, "ada-self");
    let cookie = &ada.session.cookie;

    assert_removed(&remove(&api, cookie, &ada.id(Provider::Google)));

    assert!(!api.web_works(cookie), "this session ended");
    stand.google.assert_signed_out(&ada.session.page);
    let by_github = stand.signed_in(Via::GitHub(&ada.user));
    assert_eq!(by_github.who.account, ada.session.who.account);
    assert_eq!(sorted(&listed(&api, &by_github.cookie)), [Provider::GitHub]);
}
