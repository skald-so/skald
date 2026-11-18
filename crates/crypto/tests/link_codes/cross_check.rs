//! Each side against `pakery-cpace`, another implementation of the draft, with its
//! CPACE-RISTR255-SHA512 suite of `pakery-crypto`: both give the same ISK, with any inputs and with
//! those of "Link code", whose PRS and CI these tests make apart from the crate.

use getrandom::SysRng;
use getrandom::rand_core::UnwrapErr;
use pakery_cpace::{
    CpaceInitiator as TheirInitiator, CpaceMode, CpaceResponder as TheirResponder, InitiatorState,
};
use pakery_crypto::CpaceRistretto255;
use skald_crypto::{CpaceInitiator, CpaceResponder};

use crate::fixtures::{ACCOUNT, SESSION, SHOWN, SHOWN_PRS, bytes, ci, code};

const MODE: CpaceMode = CpaceMode::InitiatorResponder;

/// The inputs of a run.
struct Inputs<'a> {
    prs: &'a [u8],
    ci: &'a [u8],
    sid: &'a [u8],
    ad_a: &'a [u8],
    ad_b: &'a [u8],
}

/// The inputs of generic runs.
const GENERIC: [Inputs<'static>; 3] = [
    Inputs {
        prs: b"Password",
        ci: b"\x0bA_initiator\x0bB_responder",
        sid: b"sid of a run",
        ad_a: b"ADa",
        ad_b: b"ADb",
    },
    Inputs {
        prs: b"K7QM2X",
        ci: b"ci",
        sid: &[0x5e; 16],
        ad_a: b"",
        ad_b: b"",
    },
    Inputs {
        prs: b"",
        ci: b"",
        sid: b"",
        ad_a: b"",
        ad_b: b"",
    },
];

impl Inputs<'_> {
    /// pakery's B, given A's share: its share and ISK.
    fn their_response(&self, ya: &[u8; 32]) -> ([u8; 32], Vec<u8>) {
        let (yb, output) = TheirResponder::<CpaceRistretto255>::respond(
            ya,
            self.prs,
            self.ci,
            self.sid,
            self.ad_a,
            self.ad_b,
            MODE,
            &mut UnwrapErr(SysRng),
        )
        .expect("pakery takes Ya");
        let yb = yb.try_into().expect("a 32-byte Yb");
        (yb, output.isk.as_bytes().to_vec())
    }

    /// pakery's A: its state and share.
    fn their_start(&self) -> (InitiatorState<CpaceRistretto255>, [u8; 32]) {
        let (ya, state) = TheirInitiator::<CpaceRistretto255>::start(
            self.prs,
            self.ci,
            self.sid,
            self.ad_a,
            &mut UnwrapErr(SysRng),
        )
        .expect("pakery's Ya");
        (state, ya.try_into().expect("a 32-byte Ya"))
    }

    /// The ISK pakery's A takes from B's share.
    fn their_finish(&self, state: InitiatorState<CpaceRistretto255>, yb: &[u8; 32]) -> Vec<u8> {
        let output = state.finish(yb, self.ad_b, MODE).expect("pakery takes Yb");
        output.isk.as_bytes().to_vec()
    }
}

/// The inputs of a link run with the PRS `prs`, the CI `ci` and [`SESSION`].
fn link_inputs<'a>(prs: &'a [u8], ci: &'a [u8]) -> Inputs<'a> {
    Inputs {
        prs,
        ci,
        sid: &SESSION,
        ad_a: b"",
        ad_b: b"",
    }
}

#[cfg_attr(not(target_arch = "wasm32"), test)]
#[cfg_attr(target_arch = "wasm32", wasm_bindgen_test::wasm_bindgen_test)]
fn the_initiator_runs_with_pakerys_responder() {
    for inputs in GENERIC {
        let (a, ya) = CpaceInitiator::start(inputs.prs, inputs.ci, inputs.sid, inputs.ad_a);
        let (yb, theirs) = inputs.their_response(&ya);
        let isk = a.finish(&yb, inputs.ad_b).expect("A takes pakery's Yb");
        assert_eq!(bytes(&isk), theirs, "{:?}", inputs.prs);
    }
}

#[cfg_attr(not(target_arch = "wasm32"), test)]
#[cfg_attr(target_arch = "wasm32", wasm_bindgen_test::wasm_bindgen_test)]
fn the_responder_runs_with_pakerys_initiator() {
    for inputs in GENERIC {
        let (state, ya) = inputs.their_start();
        let (yb, isk) = CpaceResponder::respond(
            inputs.prs,
            inputs.ci,
            inputs.sid,
            inputs.ad_b,
            &ya,
            inputs.ad_a,
        )
        .expect("B takes pakery's Ya");
        assert_eq!(
            bytes(&isk),
            inputs.their_finish(state, &yb),
            "{:?}",
            inputs.prs
        );
    }
}

#[cfg_attr(not(target_arch = "wasm32"), test)]
#[cfg_attr(target_arch = "wasm32", wasm_bindgen_test::wasm_bindgen_test)]
fn the_new_devices_side_runs_with_pakery_on_the_link_codes_inputs() {
    let ci = ci(ACCOUNT);
    let inputs = link_inputs(SHOWN_PRS, &ci);
    for entry in [SHOWN, "k7q m2x", "K7QM2X"] {
        let (a, ya) = code(entry).initiate(ACCOUNT, &SESSION);
        let (yb, theirs) = inputs.their_response(&ya);
        let isk = a.finish(&yb, &[]).expect("A takes pakery's Yb");
        assert_eq!(bytes(&isk), theirs, "{entry:?}");
    }
}

#[cfg_attr(not(target_arch = "wasm32"), test)]
#[cfg_attr(target_arch = "wasm32", wasm_bindgen_test::wasm_bindgen_test)]
fn the_issuing_devices_side_runs_with_pakery_on_the_link_codes_inputs() {
    let ci = ci(ACCOUNT);
    for (entry, prs) in [(SHOWN, SHOWN_PRS), ("018-abc", b"OIBABC".as_slice())] {
        let inputs = link_inputs(prs, &ci);
        let (state, ya) = inputs.their_start();
        let (yb, isk) = code(entry)
            .respond(ACCOUNT, &SESSION, &ya)
            .expect("B takes pakery's Ya");
        assert_eq!(bytes(&isk), inputs.their_finish(state, &yb), "{entry:?}");
    }
}
