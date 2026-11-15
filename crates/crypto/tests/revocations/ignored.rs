//! Revocations a client ignores, `membership.md`, "Revocation": one counts only if its signature
//! verifies with the key `signer` names: without `signer`, the master key; with `signer` and
//! without `also`, the key of the certificate of `signer`, a `client` or `browser` device of
//! `account` whose certificate passes checks 1 and 2. An ignored revocation revokes nothing and
//! has no say in `keep`.

use crate::fixtures::*;

/// Checks that each of `revs` is ignored: alone it revokes nothing, and next to a counting
/// revocation of the desktop that keeps the phone it does not revoke the phone.
fn assert_ignored(revs: &[Rev]) {
    for rev in revs {
        assert_revoked(std::slice::from_ref(rev), &[]);
        assert_revoked(
            &[Rev::by(LAPTOP, DESKTOP).keep(&[PHONE]), rev.clone()],
            &[DESKTOP, TABLET],
        );
    }
}

#[cfg_attr(not(target_arch = "wasm32"), test)]
#[cfg_attr(target_arch = "wasm32", wasm_bindgen_test::wasm_bindgen_test)]
fn a_revocation_with_a_broken_signature_is_ignored() {
    let broken = |rev: Rev| Rev {
        broken: true,
        ..rev
    };
    assert_ignored(&[
        broken(Rev::by(LAPTOP, DESKTOP)),
        broken(Rev::by(DESKTOP, DESKTOP)),
        broken(Rev::by_master(DESKTOP)),
    ]);
}

#[cfg_attr(not(target_arch = "wasm32"), test)]
#[cfg_attr(target_arch = "wasm32", wasm_bindgen_test::wasm_bindgen_test)]
fn a_revocation_signed_with_another_key_than_its_signers_is_ignored() {
    assert_ignored(&[
        // `signer` the laptop, signed by the browser.
        Rev {
            key: BROWSER,
            ..Rev::by(LAPTOP, DESKTOP)
        },
        // No `signer`, signed by a device's key.
        Rev {
            key: LAPTOP,
            ..Rev::by_master(DESKTOP)
        },
        // No `signer`, signed by the other account's master key.
        Rev {
            key: OTHER_MASTER,
            ..Rev::by_master(DESKTOP)
        },
    ]);
}

#[cfg_attr(not(target_arch = "wasm32"), test)]
#[cfg_attr(target_arch = "wasm32", wasm_bindgen_test::wasm_bindgen_test)]
fn a_revocation_signed_over_another_label_is_ignored() {
    let over = |label, rev: Rev| Rev { label, ..rev };
    assert_ignored(&[
        over("skald roster", Rev::by(LAPTOP, DESKTOP)),
        over("skald cert", Rev::by(LAPTOP, DESKTOP)),
        over("skald revocation ", Rev::by(LAPTOP, DESKTOP)),
        over("skald cert", Rev::by_master(DESKTOP)),
        over("skald account", Rev::by_master(DESKTOP)),
    ]);
}

#[cfg_attr(not(target_arch = "wasm32"), test)]
#[cfg_attr(target_arch = "wasm32", wasm_bindgen_test::wasm_bindgen_test)]
fn a_revocation_of_another_account_is_ignored() {
    let of_other = |rev: Rev| Rev {
        account: OTHER_ACCOUNT,
        ..rev
    };
    assert_ignored(&[
        // Its `account` another, by a device or the master key of this one.
        of_other(Rev::by(LAPTOP, DESKTOP)),
        of_other(Rev::by_master(DESKTOP)),
        // Its `account` another, by that account's device or master key.
        of_other(Rev::by(OTHER_LAPTOP, DESKTOP)),
        of_other(Rev {
            key: OTHER_MASTER,
            ..Rev::by_master(DESKTOP)
        }),
        // Its `account` this one, by a device of another account, also one chained to this
        // account's master key.
        Rev::by(OTHER_LAPTOP, DESKTOP),
        Rev::by(FOREIGN, DESKTOP),
    ]);
}

#[cfg_attr(not(target_arch = "wasm32"), test)]
#[cfg_attr(target_arch = "wasm32", wasm_bindgen_test::wasm_bindgen_test)]
fn a_revocation_by_a_service_device_is_ignored() {
    assert_ignored(&[Rev::by(SERVICE, DESKTOP)]);
    assert_revoked(&[Rev::by(SERVICE, SERVICE)], &[]);
}

#[cfg_attr(not(target_arch = "wasm32"), test)]
#[cfg_attr(target_arch = "wasm32", wasm_bindgen_test::wasm_bindgen_test)]
fn a_revocation_by_a_device_without_a_valid_certificate_is_ignored() {
    assert_ignored(&[Rev::by(BY_SERVER, DESKTOP), Rev::by(UNCERTIFIED, DESKTOP)]);
}

#[cfg_attr(not(target_arch = "wasm32"), test)]
#[cfg_attr(target_arch = "wasm32", wasm_bindgen_test::wasm_bindgen_test)]
fn a_revocation_by_a_device_with_also_is_ignored() {
    assert_ignored(&[
        Rev::by(LAPTOP, DESKTOP).also(&[TABLET]),
        Rev::by(LAPTOP, DESKTOP).also(&[DESKTOP]),
        Rev::by(BROWSER, DESKTOP).also(&[SERVICE]).keep(&[PHONE]),
    ]);
    assert_revoked(&[Rev::by(LAPTOP, TABLET).also(&[BROWSER])], &[]);
}

#[cfg_attr(not(target_arch = "wasm32"), test)]
#[cfg_attr(target_arch = "wasm32", wasm_bindgen_test::wasm_bindgen_test)]
fn a_revocation_with_a_signer_signed_by_the_master_key_is_ignored() {
    let by_master = |rev: Rev| Rev { key: MASTER, ..rev };
    assert_ignored(&[
        by_master(Rev::by(LAPTOP, DESKTOP)),
        by_master(Rev::by(DESKTOP, DESKTOP)),
        by_master(Rev::by(LAPTOP, DESKTOP).also(&[TABLET])),
        by_master(Rev::by(UNCERTIFIED, DESKTOP)),
    ]);
}
