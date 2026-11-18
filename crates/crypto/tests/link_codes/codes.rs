//! The code, its shown form and its entry: 6 characters of the base32 alphabet of RFC 4648, §6,
//! from a cryptographically secure generator, shown in two groups of three joined by `-`; entry
//! ignores case, spaces and `-` and reads `0`, `1` and `8` as `O`, `I` and `B`.

use std::collections::HashSet;

use skald_crypto::LinkCode;

use crate::fixtures::{SHOWN, code};

const ALPHABET: &str = "ABCDEFGHIJKLMNOPQRSTUVWXYZ234567";

/// Checks that `shown` is a shown code: `XXX-XXX` of the alphabet.
#[track_caller]
fn assert_shown_form(shown: &str) {
    let (first, second) = shown
        .split_once('-')
        .unwrap_or_else(|| panic!("{shown:?} has no `-`"));
    for group in [first, second] {
        assert_eq!(group.len(), 3, "{shown:?}");
        assert!(
            group.chars().all(|c| ALPHABET.contains(c)),
            "{shown:?} is not of the base32 alphabet"
        );
    }
}

/// The shown form of the code read from `entry`.
#[track_caller]
fn read(entry: &str) -> String {
    code(entry).to_string()
}

#[cfg_attr(not(target_arch = "wasm32"), test)]
#[cfg_attr(target_arch = "wasm32", wasm_bindgen_test::wasm_bindgen_test)]
fn a_generated_code_is_shown_in_two_groups_of_three() {
    for _ in 0..50 {
        let shown = LinkCode::generate().to_string();
        assert_shown_form(&shown);
        assert_eq!(read(&shown), shown, "a shown code reads back");
    }
}

#[cfg_attr(not(target_arch = "wasm32"), test)]
#[cfg_attr(target_arch = "wasm32", wasm_bindgen_test::wasm_bindgen_test)]
fn generated_codes_differ_and_use_the_whole_alphabet() {
    let codes: Vec<String> = (0..400).map(|_| LinkCode::generate().to_string()).collect();
    let distinct: HashSet<&String> = codes.iter().collect();
    // 400 codes of 30 bits collide with a chance below 1 in 10^4.
    assert!(distinct.len() >= 399, "{} distinct of 400", distinct.len());
    let used: HashSet<char> = codes.iter().flat_map(|shown| shown.chars()).collect();
    // 2400 characters miss one of 32 with a chance below 1 in 10^31.
    for c in ALPHABET.chars() {
        assert!(used.contains(&c), "no generated code holds {c:?}");
    }
}

#[cfg_attr(not(target_arch = "wasm32"), test)]
#[cfg_attr(target_arch = "wasm32", wasm_bindgen_test::wasm_bindgen_test)]
fn an_entry_ignores_case_spaces_and_hyphens() {
    for entry in [
        "K7Q-M2X",
        "k7q-m2x",
        "K7qM2x",
        "K7QM2X",
        " k7q m2x ",
        "K 7 Q - M 2 X",
        "-K7Q--M2X-",
    ] {
        assert_eq!(read(entry), SHOWN, "{entry:?}");
    }
}

#[cfg_attr(not(target_arch = "wasm32"), test)]
#[cfg_attr(target_arch = "wasm32", wasm_bindgen_test::wasm_bindgen_test)]
fn an_entry_reads_0_1_and_8_as_o_i_and_b() {
    assert_eq!(read("018-ABC"), "OIB-ABC");
    assert_eq!(read("k1o 8b0"), "KIO-BBO");
    assert_eq!(read("OIB-ABC"), "OIB-ABC");
}

#[cfg_attr(not(target_arch = "wasm32"), test)]
#[cfg_attr(target_arch = "wasm32", wasm_bindgen_test::wasm_bindgen_test)]
fn entries_of_another_length_or_character_are_refused() {
    for entry in [
        "",
        " - ",
        "K7Q-M2",
        "K7QM2",
        "K7Q-M2XA",
        "K7Q-M2X-K7Q",
        "K7Q-M29",
        "K7Q-M2!",
        "K7Q_M2X",
        "K7Q.M2X",
        "K7Q-M2Ä",
        "К7Q-M2X",
        "K7Q\u{0}M2X",
    ] {
        assert!(LinkCode::from_entry(entry).is_err(), "{entry:?}");
    }
}
