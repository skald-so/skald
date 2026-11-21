//! Runs of a new device without a session: the run's pairing, 6 characters of the code's alphabet;
//! an entry of 12 characters, the pairing then the code, read as a code is, and its shown form in
//! three groups of four; and the run's CI, the encoded `["skald link code", pairing]`, given in
//! `docs/protocol/test-vectors.md`: each side runs on it with `pakery-cpace`.

use std::collections::HashSet;

use getrandom::SysRng;
use getrandom::rand_core::UnwrapErr;
use pakery_cpace::{CpaceInitiator as TheirInitiator, CpaceMode, CpaceResponder as TheirResponder};
use pakery_crypto::CpaceRistretto255;
use skald_crypto::{LinkCode, LinkEntry, LinkPairing};

use crate::fixtures::{ACCOUNT, SESSION, SHOWN, SHOWN_PRS, bytes, code, hex, vector_section};

const ALPHABET: &str = "ABCDEFGHIJKLMNOPQRSTUVWXYZ234567";

/// The pairing of the vector, and another.
const PAIRING: &str = "PQRSTU";
const OTHER_PAIRING: &str = "PQRSTV";

/// The CI of a run with [`PAIRING`], computed apart from the crate: the encoded `["skald link
/// code", "PQRSTU"]`, the label a text string and the pairing a byte string.
const CI: &str = "826f736b616c64206c696e6b20636f646546505152535455";

/// The section of the doc that gives the CI.
const SECTION: &str = "## Link code without a session";

/// The pairing `text`, which must be one.
#[track_caller]
fn pairing(text: &str) -> LinkPairing {
    LinkPairing::from_entry(text).unwrap_or_else(|_| panic!("{text:?} is a pairing"))
}

/// The pairing and code `entry` holds, which must be 12 characters.
#[track_caller]
fn paired(entry: &str) -> (String, String) {
    match LinkEntry::from_entry(entry) {
        Ok(LinkEntry::Paired { pairing, code }) => (pairing.to_string(), code.to_string()),
        other => panic!("{entry:?} is no pairing and code: {other:?}"),
    }
}

/// A full run with `shown` and `ran` on A's side and `entered` and `paired` on B's: both ISKs.
fn run(shown: &str, ran: &str, entered: &str, paired: &str) -> (Vec<u8>, Vec<u8>) {
    let (a, ya) = code(shown).initiate_paired(&pairing(ran), &SESSION);
    let (yb, isk_b) = code(entered)
        .respond_paired(&pairing(paired), &SESSION, &ya)
        .expect("B takes A's share");
    let isk_a = a.finish(&yb, &[]).expect("A takes B's share");
    (bytes(&isk_a), bytes(&isk_b))
}

#[cfg_attr(not(target_arch = "wasm32"), test)]
#[cfg_attr(target_arch = "wasm32", wasm_bindgen_test::wasm_bindgen_test)]
fn generated_pairings_are_6_characters_of_the_alphabet_and_differ() {
    let pairings: Vec<String> = (0..400)
        .map(|_| LinkPairing::generate().to_string())
        .collect();
    for shown in &pairings {
        assert_eq!(shown.len(), 6, "{shown:?}");
        assert!(shown.chars().all(|c| ALPHABET.contains(c)), "{shown:?}");
        assert_eq!(pairing(shown).as_str(), shown, "reads back");
    }
    let distinct: HashSet<&String> = pairings.iter().collect();
    assert!(distinct.len() >= 399, "{} distinct of 400", distinct.len());
    let used: HashSet<char> = pairings.iter().flat_map(|shown| shown.chars()).collect();
    assert_eq!(used.len(), 32, "every character of the alphabet: {used:?}");
}

#[cfg_attr(not(target_arch = "wasm32"), test)]
#[cfg_attr(target_arch = "wasm32", wasm_bindgen_test::wasm_bindgen_test)]
fn a_code_with_its_pairing_is_shown_in_three_groups_of_four_and_reads_back() {
    let shown = code(SHOWN).shown_with(&pairing(PAIRING));
    assert_eq!(shown, "PQRS-TUK7-QM2X");
    assert_eq!(paired(&shown), (PAIRING.to_owned(), SHOWN.to_owned()));
    for entry in [
        "pqrs-tuk7-qm2x",
        "PQRSTUK7QM2X",
        " pqr stu k7q m2x ",
        "PQRS--TUK7-QM2X-",
    ] {
        assert_eq!(
            paired(entry),
            (PAIRING.to_owned(), SHOWN.to_owned()),
            "{entry:?}"
        );
    }
    assert_eq!(
        paired("018ABC-018ABC"),
        ("OIBABC".to_owned(), "OIB-ABC".to_owned()),
        "0, 1 and 8 read as O, I and B"
    );
    for _ in 0..20 {
        let (code, pairing) = (LinkCode::generate(), LinkPairing::generate());
        let shown = code.shown_with(&pairing);
        assert_eq!(paired(&shown), (pairing.to_string(), code.to_string()));
    }
}

#[cfg_attr(not(target_arch = "wasm32"), test)]
#[cfg_attr(target_arch = "wasm32", wasm_bindgen_test::wasm_bindgen_test)]
fn an_entry_of_6_characters_is_a_code_and_others_are_refused() {
    assert_eq!(
        LinkEntry::from_entry("k7q m2x"),
        Ok(LinkEntry::Code(code(SHOWN)))
    );
    for entry in [
        "",
        "K7QM2",
        "K7QM2XA",
        "PQRSTUK7QM2",
        "PQRSTUK7QM2XA",
        "PQRS-TUK7-QM29",
        "PQRS_TUK7_QM2X",
        "PQRS-TUK7-QM2Ä",
    ] {
        assert!(LinkEntry::from_entry(entry).is_err(), "{entry:?}");
    }
    for text in ["", "PQRST", "PQRSTUV", "PQRST9", "PQRST!"] {
        assert!(LinkPairing::from_entry(text).is_err(), "{text:?}");
    }
}

#[cfg_attr(not(target_arch = "wasm32"), test)]
#[cfg_attr(target_arch = "wasm32", wasm_bindgen_test::wasm_bindgen_test)]
fn a_run_with_the_pairing_entered_gives_both_sides_one_isk() {
    let (isk_a, isk_b) = run(SHOWN, PAIRING, "k7qm2x", "pqrstu");
    assert_eq!(isk_a, isk_b);
}

#[cfg_attr(not(target_arch = "wasm32"), test)]
#[cfg_attr(target_arch = "wasm32", wasm_bindgen_test::wasm_bindgen_test)]
fn a_run_with_another_pairing_or_code_fails() {
    for (entered, paired, what) in [
        (SHOWN, OTHER_PAIRING, "another pairing"),
        ("K7Q-M2Y", PAIRING, "another code"),
    ] {
        let (isk_a, isk_b) = run(SHOWN, PAIRING, entered, paired);
        assert_ne!(isk_a, isk_b, "{what}");
    }
    // A run on the account's CI does not meet one on the pairing's.
    let (a, ya) = code(SHOWN).initiate_paired(&pairing(PAIRING), &SESSION);
    let (yb, isk_b) = code(SHOWN).respond(ACCOUNT, &SESSION, &ya).unwrap();
    assert_ne!(bytes(&a.finish(&yb, &[]).unwrap()), bytes(&isk_b));
}

#[cfg_attr(not(target_arch = "wasm32"), test)]
#[cfg_attr(target_arch = "wasm32", wasm_bindgen_test::wasm_bindgen_test)]
fn each_side_runs_with_pakery_on_the_pairings_ci() {
    let mode = CpaceMode::InitiatorResponder;
    let ci = hex(CI);
    let (a, ya) = code(SHOWN).initiate_paired(&pairing(PAIRING), &SESSION);
    let (yb, theirs) = TheirResponder::<CpaceRistretto255>::respond(
        &ya,
        SHOWN_PRS,
        &ci,
        &SESSION,
        b"",
        b"",
        mode,
        &mut UnwrapErr(SysRng),
    )
    .expect("pakery takes Ya");
    let yb: [u8; 32] = yb.try_into().expect("a 32-byte Yb");
    let isk = a.finish(&yb, &[]).expect("A takes pakery's Yb");
    assert_eq!(bytes(&isk), theirs.isk.as_bytes().to_vec(), "A");

    let (ya, state) = TheirInitiator::<CpaceRistretto255>::start(
        SHOWN_PRS,
        &ci,
        &SESSION,
        b"",
        &mut UnwrapErr(SysRng),
    )
    .expect("pakery's Ya");
    let ya: [u8; 32] = ya.try_into().expect("a 32-byte Ya");
    let (yb, isk) = code(SHOWN)
        .respond_paired(&pairing(PAIRING), &SESSION, &ya)
        .expect("B takes pakery's Ya");
    let theirs = state.finish(&yb, b"", mode).expect("pakery takes Yb");
    assert_eq!(bytes(&isk), theirs.isk.as_bytes().to_vec(), "B");
}

#[cfg_attr(not(target_arch = "wasm32"), test)]
#[cfg_attr(target_arch = "wasm32", wasm_bindgen_test::wasm_bindgen_test)]
fn the_test_vectors_give_the_ci() {
    let section = vector_section(SECTION);
    assert!(section.contains(CI), "{SECTION:?} gives the CI: {CI}");
}
