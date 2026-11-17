//! Removals that remove nothing: a sign-in older than the setting, a device session, a request from
//! another site or without `Origin`, no session, and a method of another account or an unknown ID.

use crate::common::google::HOST;
use crate::sessions::{cookie_of, token_of};
use crate::{
    BOTH, LINK_AGE, Provider, Stand, Via, assert_refused, assert_removed, listed, listed_with,
    machine_of, remove, remove_with, sorted,
};

/// With `--link-sign-in-age 60`, Ada's web session whose sign-in is moved back two minutes removes
/// nothing, though it still lists the methods; after she signs in again, the new session removes
/// one.
#[test]
fn sign_in_older_than_the_setting_removes_nothing() {
    let stand = Stand::launch(&[LINK_AGE, "60"]);
    let api = stand.api();
    let ada = stand.with_both(101, "ada-old");
    let (page, cookie) = (&ada.session.page, &ada.session.cookie);
    api.move_sign_in_back(token_of(cookie), "2 minutes");

    for provider in BOTH {
        assert_refused(&remove(&api, cookie, &ada.id(provider)), "an old sign-in");
    }
    assert_eq!(sorted(&listed(&api, cookie)), BOTH);
    assert!(api.web_works(cookie), "an old sign-in still works");

    stand.google.sign_in(page, &ada.person);
    let fresh = cookie_of(page);
    assert_ne!(&fresh, cookie, "a new session");
    assert_eq!(stand.google.account(page).account, ada.session.who.account);
    assert_removed(&remove(&api, &fresh, &ada.id(Provider::GitHub)));
    assert_eq!(sorted(&listed(&api, &fresh)), [Provider::Google]);
}

/// A machine of Ada's account, whose device session lists her methods, removes none of them, with
/// or without the server's `Origin`.
#[test]
fn device_session_removes_nothing() {
    let stand = Stand::launch(&[]);
    let api = stand.api();
    let ada = stand.with_both(111, "ada-device");
    let token = machine_of(&stand.google, &ada.session.page);
    let bearer = format!("Bearer {token}");
    let device = [("authorization", bearer.as_str())];
    assert_eq!(sorted(&listed_with(&api, &device)), BOTH);

    for provider in BOTH {
        let id = ada.id(provider);
        assert_refused(&remove_with(&api, &id, &device), "a device session");
        let with_origin = [("authorization", bearer.as_str()), ("origin", api.public)];
        assert_refused(&remove_with(&api, &id, &with_origin), "a device session");
    }
    assert_eq!(sorted(&listed(&api, &ada.session.cookie)), BOTH);
    assert!(api.device_works(&token));
}

/// Ada's web session removes nothing by a request from another site, one from a site whose name
/// starts with the server's, one without `Origin`, or one without the session.
#[test]
fn request_from_another_site_or_without_origin_or_session_removes_nothing() {
    let stand = Stand::launch(&[]);
    let api = stand.api();
    let ada = stand.with_both(121, "ada-site");
    let cookie = ada.session.cookie.as_str();
    let look_alike = api.public.replacen(HOST, &format!("{HOST}.evil.test"), 1);
    let refusals: [(&str, &[(&str, &str)]); 4] = [
        (
            "another site",
            &[("cookie", cookie), ("origin", "http://evil.test")],
        ),
        (
            "a look-alike site",
            &[("cookie", cookie), ("origin", &look_alike)],
        ),
        ("no Origin", &[("cookie", cookie)]),
        ("no session", &[("origin", api.public)]),
    ];

    for provider in BOTH {
        for (what, headers) in refusals {
            assert_refused(&remove_with(&api, &ada.id(provider), headers), what);
        }
    }
    assert_eq!(sorted(&listed(&api, cookie)), BOTH);
    assert!(api.web_works(cookie));
}

/// Ada's web session removes neither a method of Bob's account, by its ID from his list, nor one of
/// an ID no method has; both accounts keep their methods.
#[test]
fn method_of_another_account_or_an_unknown_id_is_not_removed() {
    let stand = Stand::launch(&[]);
    let api = stand.api();
    let ada = stand.with_both(131, "ada-other");
    let bob = stand.with_both(132, "bob-other");

    for provider in BOTH {
        assert_refused(
            &remove(&api, &ada.session.cookie, &bob.id(provider)),
            "Bob's method",
        );
    }
    let made_up = "0196f0c0-0000-7000-8000-00000000dead";
    assert_refused(&remove(&api, &ada.session.cookie, made_up), "an unknown ID");

    assert_eq!(sorted(&listed(&api, &bob.session.cookie)), BOTH);
    assert_eq!(sorted(&listed(&api, &ada.session.cookie)), BOTH);
    let by_github = stand.signed_in(Via::GitHub(&bob.user));
    assert_eq!(by_github.who.account, bob.session.who.account);
}
