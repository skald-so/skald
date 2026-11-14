//! The responses of Google sign-in conform to the OpenAPI spec in `api/openapi.json`.

use skald_oidc_stand_in::Fault;

use super::common::sign_in::{SESSION, with_param};
use super::common::spec::{spec, spec_violations};
use super::common::{Response, get_url};
use super::{CALLBACK, Google, Jar, START, Setup, Who, text};

#[test]
fn sign_in_responses_conform_to_spec() {
    let spec = spec();
    let google = Google::launch(Setup {
        local: true,
        ..Setup::default()
    });
    let ada = google.person("110248495921238987100", "ada.spec@example.com");
    let check = |path: &str, response: &Response| {
        assert_eq!(spec_violations(&spec, path, response), Vec::<String>::new());
    };

    // The start, with a return address.
    let start = get_url(&google.start_url(Some("/settings")), &[]);
    assert_eq!(start.status, 302, "{}", text(&start));
    check(START, &start);
    let Some(authorize) = start.header("location") else {
        panic!("no redirect to the provider: {:?}", start.headers);
    };
    assert!(
        authorize.starts_with(&format!("{}?", google.provider.authorize_url())),
        "{authorize}"
    );

    // Refused callbacks: no state, a wrong one, none of anything.
    let (jar, callback) = google.to_callback(&ada);
    for refused in [
        with_param(&callback, "state", None),
        with_param(&callback, "state", Some("0123456789abcdef")),
    ] {
        check(CALLBACK, &get_url(&refused, &[("cookie", &jar.header())]));
    }
    check(CALLBACK, &google.server.get(CALLBACK));

    // Callbacks the provider fails: an error instead of a code, a refused code, a failed token
    // endpoint, an ID token with another sign-in's nonce.
    for fault in [
        Fault::AccessDenied,
        Fault::InvalidGrant,
        Fault::TokenStatus { status: 500 },
        Fault::Nonce {
            nonce: "n-another-sign-in-0123456789".to_owned(),
        },
    ] {
        google.provider.set_faults(std::slice::from_ref(&fault));
        let (jar, callback) = google.to_callback(&ada);
        let failed = get_url(&callback, &[("cookie", &jar.header())]);
        assert!(
            (400..600).contains(&failed.status),
            "{fault:?}: {}",
            failed.status
        );
        check(CALLBACK, &failed);
    }
    google.provider.set_faults(&[]);

    // A sign-in, and the session's account it gives.
    let signed_in = get_url(&callback, &[("cookie", &jar.header())]);
    assert_eq!(signed_in.status, 200);
    check(CALLBACK, &signed_in);
    let mut session_jar = jar.clone();
    session_jar.take(&signed_in);
    let session = google.session_of(&session_jar);
    assert_eq!(session.status, 200);
    check(SESSION, &session);
    let who = Who::of(&session);
    assert_eq!(who.username, "ada-spec");
    assert_eq!(who.github_login, None);

    let none = google.session_of(&Jar::default());
    assert_eq!(none.status, 401);
    check(SESSION, &none);
}
