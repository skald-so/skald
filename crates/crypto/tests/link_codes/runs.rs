//! Runs between the crate's own sides: with one code both get one ISK, and the request and grant
//! keys of each side open what the other sealed; a run with another code, account or `session`
//! gives another ISK, so the request does not open; each start and response takes a fresh scalar;
//! a side fails the run on a share that does not decode or gives the identity.

use skald_crypto::{CpaceInitiator, CpaceResponder, LinkCode};

use crate::fixtures::{
    ACCOUNT, OTHER_ACCOUNT, OTHER_SESSION, SESSION, SHOWN, bad_shares, bytes, code, run,
};

#[cfg_attr(not(target_arch = "wasm32"), test)]
#[cfg_attr(target_arch = "wasm32", wasm_bindgen_test::wasm_bindgen_test)]
fn a_run_with_one_code_gives_both_sides_one_isk() {
    let (isk_a, isk_b) = run(&code(SHOWN), &code("k7q m2x"));
    assert_eq!(bytes(&isk_a), bytes(&isk_b));

    let request = isk_a.request_key().seal(b"the new device's keys");
    let opened = isk_b
        .request_key()
        .open(&request)
        .expect("B opens the request");
    assert_eq!(opened.as_slice(), b"the new device's keys");
    let grant = isk_b.grant_key().seal(b"its certificate");
    let opened = isk_a.grant_key().open(&grant).expect("A opens the grant");
    assert_eq!(opened.as_slice(), b"its certificate");
}

#[cfg_attr(not(target_arch = "wasm32"), test)]
#[cfg_attr(target_arch = "wasm32", wasm_bindgen_test::wasm_bindgen_test)]
fn a_code_read_with_0_1_and_8_runs_with_the_code_shown() {
    let (isk_a, isk_b) = run(&code("OIB-ABC"), &code("018abc"));
    assert_eq!(bytes(&isk_a), bytes(&isk_b));
}

#[cfg_attr(not(target_arch = "wasm32"), test)]
#[cfg_attr(target_arch = "wasm32", wasm_bindgen_test::wasm_bindgen_test)]
fn a_run_with_another_code_fails() {
    for entered in ["K7Q-M2Y", "M2X-K7Q", "A7Q-M2X"] {
        let (isk_a, isk_b) = run(&code(SHOWN), &code(entered));
        assert_ne!(bytes(&isk_a), bytes(&isk_b), "{entered}");
        let request = isk_a.request_key().seal(b"the new device's keys");
        assert!(
            isk_b.request_key().open(&request).is_err(),
            "{entered}: B opens no request"
        );
    }
}

#[cfg_attr(not(target_arch = "wasm32"), test)]
#[cfg_attr(target_arch = "wasm32", wasm_bindgen_test::wasm_bindgen_test)]
fn a_run_on_another_account_or_session_fails() {
    let shown = code(SHOWN);
    for (what, account, session) in [
        ("another account", OTHER_ACCOUNT, SESSION),
        ("another session", ACCOUNT, OTHER_SESSION),
    ] {
        let (a, ya) = shown.initiate(ACCOUNT, &SESSION);
        let (yb, isk_b) = shown
            .respond(account, &session, &ya)
            .expect("B takes A's share");
        let isk_a = a.finish(&yb, &[]).expect("A takes B's share");
        assert_ne!(bytes(&isk_a), bytes(&isk_b), "{what}");
    }
}

#[cfg_attr(not(target_arch = "wasm32"), test)]
#[cfg_attr(target_arch = "wasm32", wasm_bindgen_test::wasm_bindgen_test)]
fn each_run_takes_a_fresh_scalar() {
    let shown = code(SHOWN);
    let (a1, ya1) = shown.initiate(ACCOUNT, &SESSION);
    let (a2, ya2) = shown.initiate(ACCOUNT, &SESSION);
    assert_ne!(ya1, ya2, "A's shares of one code and session");
    let (yb1, isk1) = shown.respond(ACCOUNT, &SESSION, &ya1).expect("B takes Ya");
    let (yb2, isk2) = shown
        .respond(ACCOUNT, &SESSION, &ya1)
        .expect("B takes Ya again");
    assert_ne!(yb1, yb2, "B's shares to one share");
    assert_ne!(bytes(&isk1), bytes(&isk2), "B's ISKs");
    assert_eq!(bytes(&a1.finish(&yb1, &[]).unwrap()), bytes(&isk1));
    assert_ne!(bytes(&a2.finish(&yb1, &[]).unwrap()), bytes(&isk1));

    let (_, ya) = CpaceInitiator::start(b"Password", b"ci", b"sid", b"");
    let (_, other) = CpaceInitiator::start(b"Password", b"ci", b"sid", b"");
    assert_ne!(ya, other, "generic starts");
    let (yb, _) = CpaceResponder::respond(b"Password", b"ci", b"sid", b"", &ya, b"").unwrap();
    let (other, _) = CpaceResponder::respond(b"Password", b"ci", b"sid", b"", &ya, b"").unwrap();
    assert_ne!(yb, other, "generic responses");
}

#[cfg_attr(not(target_arch = "wasm32"), test)]
#[cfg_attr(target_arch = "wasm32", wasm_bindgen_test::wasm_bindgen_test)]
fn a_side_fails_the_run_on_a_bad_share() {
    let shown = code(SHOWN);
    for (what, share) in bad_shares() {
        let (a, _) = shown.initiate(ACCOUNT, &SESSION);
        assert!(a.finish(&share, &[]).is_err(), "A: {what}");
        assert!(
            shown.respond(ACCOUNT, &SESSION, &share).is_err(),
            "B: {what}"
        );
        let (a, _) = CpaceInitiator::start(b"Password", b"ci", b"sid", b"");
        assert!(a.finish(&share, b"").is_err(), "generic A: {what}");
        let responded = CpaceResponder::respond(b"Password", b"ci", b"sid", b"", &share, b"");
        assert!(responded.is_err(), "generic B: {what}");
    }
}

#[cfg_attr(not(target_arch = "wasm32"), test)]
#[cfg_attr(target_arch = "wasm32", wasm_bindgen_test::wasm_bindgen_test)]
fn generated_codes_run() {
    let shown = LinkCode::generate();
    let entered = code(&shown.to_string().to_lowercase());
    let (isk_a, isk_b) = run(&shown, &entered);
    assert_eq!(bytes(&isk_a), bytes(&isk_b));
}
