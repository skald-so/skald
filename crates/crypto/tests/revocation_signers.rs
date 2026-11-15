//! The key that verifies a revocation, `membership.md`, "Revocation": `signer` alone names it, so
//! one with `signer` that verifies only with the master key is ignored.

#[allow(dead_code)]
#[path = "rosters/cbor.rs"]
mod cbor;
#[allow(dead_code)]
#[path = "revocations/fixtures.rs"]
mod fixtures;

use fixtures::*;

#[cfg_attr(not(target_arch = "wasm32"), test)]
#[cfg_attr(target_arch = "wasm32", wasm_bindgen_test::wasm_bindgen_test)]
fn a_revocation_with_a_signer_signed_by_the_master_key_is_ignored() {
    let by_master = |rev: Rev| Rev { key: MASTER, ..rev };
    for rev in [
        by_master(Rev::by(LAPTOP, DESKTOP)),
        by_master(Rev::by(DESKTOP, DESKTOP)),
        by_master(Rev::by(LAPTOP, DESKTOP).also(&[TABLET])),
        by_master(Rev::by(UNCERTIFIED, DESKTOP)),
    ] {
        assert_revoked(std::slice::from_ref(&rev), &[]);
        assert_revoked(
            &[Rev::by(LAPTOP, DESKTOP).keep(&[PHONE]), rev],
            &[DESKTOP, TABLET],
        );
    }
}
