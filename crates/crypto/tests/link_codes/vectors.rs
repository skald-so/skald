//! The test vectors of draft-irtf-cfrg-cpace-21, Appendix B.3, for CPACE-RISTR255-SHA512 in
//! initiator-responder mode: with the draft's scalars, each side makes the draft's share and ISK,
//! and a side refuses the draft's invalid shares.

use skald_crypto::{CpaceInitiator, CpaceResponder};

use crate::fixtures::{bad_shares, bytes, draft, hex, hex32};

#[cfg_attr(not(target_arch = "wasm32"), test)]
#[cfg_attr(target_arch = "wasm32", wasm_bindgen_test::wasm_bindgen_test)]
fn the_initiator_makes_the_drafts_share_and_isk() {
    let (a, ya) = CpaceInitiator::start_with_scalar(
        &hex32(draft::YA),
        draft::PRS,
        &hex(draft::CI),
        &hex(draft::SID),
        draft::AD_A,
    );
    assert_eq!(ya, hex32(draft::SHARE_A), "Ya");
    let isk = a
        .finish(&hex32(draft::SHARE_B), draft::AD_B)
        .expect("A takes the draft's Yb");
    assert_eq!(bytes(&isk), hex(draft::ISK), "ISK_IR");
    assert_ne!(bytes(&isk), hex(draft::ISK_PARALLEL));
}

#[cfg_attr(not(target_arch = "wasm32"), test)]
#[cfg_attr(target_arch = "wasm32", wasm_bindgen_test::wasm_bindgen_test)]
fn the_responder_makes_the_drafts_share_and_isk() {
    let (yb, isk) = CpaceResponder::respond_with_scalar(
        &hex32(draft::YB),
        draft::PRS,
        &hex(draft::CI),
        &hex(draft::SID),
        draft::AD_B,
        &hex32(draft::SHARE_A),
        draft::AD_A,
    )
    .expect("B takes the draft's Ya");
    assert_eq!(yb, hex32(draft::SHARE_B), "Yb");
    assert_eq!(bytes(&isk), hex(draft::ISK), "ISK_IR");
}

#[cfg_attr(not(target_arch = "wasm32"), test)]
#[cfg_attr(target_arch = "wasm32", wasm_bindgen_test::wasm_bindgen_test)]
fn the_drafts_vector_gives_another_isk_with_any_other_input() {
    let (ci, sid) = (hex(draft::CI), hex(draft::SID));
    let isk = |prs: &[u8], ci: &[u8], sid: &[u8], ad_a: &[u8], ad_b: &[u8]| {
        let (a, _) = CpaceInitiator::start_with_scalar(&hex32(draft::YA), prs, ci, sid, ad_a);
        bytes(&a.finish(&hex32(draft::SHARE_B), ad_b).expect("A takes Yb"))
    };
    let expected = hex(draft::ISK);
    assert_eq!(
        isk(draft::PRS, &ci, &sid, draft::AD_A, draft::AD_B),
        expected
    );
    assert_ne!(
        isk(b"Passwort", &ci, &sid, draft::AD_A, draft::AD_B),
        expected,
        "PRS"
    );
    assert_ne!(
        isk(
            draft::PRS,
            b"\x0bA_initiator",
            &sid,
            draft::AD_A,
            draft::AD_B
        ),
        expected,
        "CI"
    );
    assert_ne!(
        isk(draft::PRS, &ci, &[0x7e; 16], draft::AD_A, draft::AD_B),
        expected,
        "sid"
    );
    assert_ne!(
        isk(draft::PRS, &ci, &sid, b"", draft::AD_B),
        expected,
        "ADa"
    );
    assert_ne!(
        isk(draft::PRS, &ci, &sid, draft::AD_A, b""),
        expected,
        "ADb"
    );
}

#[cfg_attr(not(target_arch = "wasm32"), test)]
#[cfg_attr(target_arch = "wasm32", wasm_bindgen_test::wasm_bindgen_test)]
fn each_side_refuses_the_drafts_invalid_shares() {
    let (ci, sid) = (hex(draft::CI), hex(draft::SID));
    for (what, share) in bad_shares() {
        let (a, _) = CpaceInitiator::start_with_scalar(
            &hex32(draft::YA),
            draft::PRS,
            &ci,
            &sid,
            draft::AD_A,
        );
        assert!(a.finish(&share, draft::AD_B).is_err(), "A: {what}");
        let responded = CpaceResponder::respond_with_scalar(
            &hex32(draft::YB),
            draft::PRS,
            &ci,
            &sid,
            draft::AD_B,
            &share,
            draft::AD_A,
        );
        assert!(responded.is_err(), "B: {what}");
    }
}
