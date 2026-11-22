//! The recovery key's form, natively and in WASM (`docs/protocol/membership.md`, "Account"): 12
//! characters of Crockford's base32 from a cryptographically secure generator, 60 bits, shown as 3
//! groups of 4. Its input takes any case, with or without spaces and `-`, reads O as 0 and I and L
//! as 1, and skips blank lines, a terminal's bracketed-paste markers, removed whole, and every other
//! character outside the alphabet; it refuses an input with fewer or more than 12 characters of the
//! alphabet left.
//!
//! The API, as these tests use it: `skald_crypto::RecoveryKey`: `generate()`; `shown()`, the text a
//! person writes down, `XXXX-XXXX-XXXX` in the upper-case alphabet [`ALPHABET`], its groups joined
//! by `-`; `parse(&str) -> Result<RecoveryKey, _>`, the text a person types or pastes.

use std::collections::HashSet;

use skald_crypto::RecoveryKey;

/// Crockford's base32 alphabet, in the order of its values.
const ALPHABET: &str = "0123456789ABCDEFGHJKMNPQRSTVWXYZ";

/// A key in its shown form, with 0s and 1s for the input's O, I and L.
const KEY: &str = "1D0F-4R1K-0VZ8";

/// A terminal's bracketed-paste markers, around what is pasted.
const PASTE_START: &str = "\x1b[200~";
const PASTE_END: &str = "\x1b[201~";

/// Whether `text` is in the shown form: 3 groups of 4 characters of [`ALPHABET`] joined by `-`.
fn in_shown_form(text: &str) -> bool {
    let groups: Vec<&str> = text.split('-').collect();
    groups.len() == 3
        && groups
            .iter()
            .all(|group| group.len() == 4 && group.chars().all(|c| ALPHABET.contains(c)))
}

/// The shown form of the key `input` reads as; fails if it is refused.
#[track_caller]
fn read(input: &str) -> String {
    RecoveryKey::parse(input)
        .unwrap_or_else(|_| panic!("{input:?} is refused"))
        .shown()
        .as_str()
        .to_owned()
}

/// Fails unless each of `inputs` reads as [`KEY`].
#[track_caller]
fn assert_read_as_key(inputs: &[String]) {
    for input in inputs {
        assert_eq!(read(input), KEY, "{input:?}");
    }
}

#[cfg_attr(not(target_arch = "wasm32"), test)]
#[cfg_attr(target_arch = "wasm32", wasm_bindgen_test::wasm_bindgen_test)]
fn a_key_is_shown_as_three_groups_of_four_characters_of_crockfords_base32() {
    let key = RecoveryKey::parse(KEY).expect("a key in its shown form reads");

    assert_eq!(key.shown().as_str(), KEY);
    assert!(in_shown_form(KEY));
}

/// A thousand generated keys are each in the shown form, read back from it, and differ; each of
/// the 12 places takes every character of the alphabet, so each carries 5 bits and the key 60.
#[cfg_attr(not(target_arch = "wasm32"), test)]
#[cfg_attr(target_arch = "wasm32", wasm_bindgen_test::wasm_bindgen_test)]
fn generated_keys_are_new_each_time_and_carry_sixty_bits() {
    let shown: Vec<String> = (0..1000)
        .map(|_| RecoveryKey::generate().shown().as_str().to_owned())
        .collect();

    for key in &shown {
        assert!(in_shown_form(key), "{key:?}");
        assert_eq!(&read(key), key, "it reads back from its shown form");
    }
    let distinct: HashSet<&String> = shown.iter().collect();
    assert_eq!(distinct.len(), shown.len(), "a generated key repeats");
    let digits: Vec<Vec<char>> = shown
        .iter()
        .map(|key| key.chars().filter(|c| *c != '-').collect())
        .collect();
    for place in 0..12 {
        let seen: HashSet<char> = digits.iter().map(|key| key[place]).collect();
        assert_eq!(
            seen.len(),
            32,
            "place {place} takes only {} characters: {seen:?}",
            seen.len()
        );
    }
}

#[cfg_attr(not(target_arch = "wasm32"), test)]
#[cfg_attr(target_arch = "wasm32", wasm_bindgen_test::wasm_bindgen_test)]
fn a_key_reads_back_in_any_case_with_spaces_and_without_dashes() {
    let joined: String = KEY.split('-').collect();
    assert_read_as_key(&[
        KEY.to_owned(),
        KEY.to_lowercase(),
        "1d0F-4r1K-0vZ8".to_owned(),
        joined.clone(),
        joined.to_lowercase(),
        KEY.replace('-', " "),
        KEY.replace('-', " - "),
        format!("  {}  ", KEY.replace('-', "   ")),
        format!("-{}-", KEY.replace('-', "--")),
        "1 d 0 f 4 r 1 k 0 v z 8".to_owned(),
    ]);
}

#[cfg_attr(not(target_arch = "wasm32"), test)]
#[cfg_attr(target_arch = "wasm32", wasm_bindgen_test::wasm_bindgen_test)]
fn o_reads_as_0_and_i_and_l_as_1() {
    assert_read_as_key(&[
        "ID0F-4RLK-OVZ8".to_owned(),
        "LDOF-4RIK-0VZ8".to_owned(),
        "id0f-4rlk-ovz8".to_owned(),
        "lDoF 4RiK OVZ8".to_owned(),
        "iDOF4RLKoVZ8".to_owned(),
    ]);
}

/// Characters outside the alphabet that are not O, I or L are skipped wherever they are: U,
/// punctuation, other dashes and spaces, control characters, line ends and blank lines, and
/// characters outside ASCII.
#[cfg_attr(not(target_arch = "wasm32"), test)]
#[cfg_attr(target_arch = "wasm32", wasm_bindgen_test::wasm_bindgen_test)]
fn characters_outside_the_alphabet_are_skipped() {
    assert_read_as_key(&[
        format!("\"{KEY}\""),
        format!("«{KEY}»"),
        format!("{KEY}."),
        format!("<{KEY}>"),
        KEY.replace('-', "_"),
        KEY.replace('-', "/"),
        KEY.replace('-', "="),
        KEY.replace('-', "—"),
        KEY.replace('-', "\u{a0}"),
        KEY.replace('-', "\u{200b}"),
        KEY.replace('-', "\t"),
        format!("U{}U", KEY.replace('-', "u")),
        format!("*{KEY}* ✓"),
        format!("{KEY}\n"),
        format!("{KEY}\r\n"),
        format!("\n\n{KEY}\n\n"),
        format!("\r\n\r\n{KEY}\r\n"),
        format!("\x07{KEY}\x7f"),
    ]);
}

/// The markers a terminal puts around a paste, `ESC [200~` and `ESC [201~`, are removed whole,
/// their digits with them, on their own or among other skipped characters and blank lines.
#[cfg_attr(not(target_arch = "wasm32"), test)]
#[cfg_attr(target_arch = "wasm32", wasm_bindgen_test::wasm_bindgen_test)]
fn bracketed_paste_markers_are_removed_whole() {
    let joined: String = KEY.split('-').collect();
    assert_read_as_key(&[
        format!("{PASTE_START}{KEY}{PASTE_END}"),
        format!("{PASTE_START}{joined}{PASTE_END}"),
        format!("{PASTE_START}{}{PASTE_END}", KEY.to_lowercase()),
        format!("{PASTE_START}{KEY}"),
        format!("{KEY}{PASTE_END}"),
        format!("{PASTE_START}\n{KEY}\n{PASTE_END}"),
        format!("\r\n{PASTE_START}\r\n{KEY}\r\n{PASTE_END}\r\n"),
        format!("{PASTE_START} «{}» {PASTE_END}\n", KEY.replace('-', " ")),
        format!("{PASTE_START}id0f 4rlk ovz8{PASTE_END}"),
        format!("{PASTE_START}{PASTE_START}{KEY}{PASTE_END}{PASTE_END}"),
    ]);
}

/// An input with exactly 12 characters of the alphabet left reads; with fewer or more, it is
/// refused, whatever else it holds.
#[cfg_attr(not(target_arch = "wasm32"), test)]
#[cfg_attr(target_arch = "wasm32", wasm_bindgen_test::wasm_bindgen_test)]
fn an_input_with_fewer_or_more_than_twelve_characters_of_the_alphabet_is_refused() {
    assert_eq!(read(KEY), KEY);
    let joined: String = KEY.split('-').collect();
    let refused = [
        String::new(),
        " - ".to_owned(),
        "\n\n".to_owned(),
        format!("{PASTE_START}{PASTE_END}"),
        format!("{PASTE_START}\r\n{PASTE_END}"),
        // A character short and a character over.
        KEY[..KEY.len() - 1].to_owned(),
        joined[..11].to_owned(),
        format!("{KEY}0"),
        format!("{joined}A"),
        format!("{KEY}O"),
        format!("{KEY}-L"),
        format!("I{KEY}"),
        // Skipped characters do not count.
        format!("{}U", &KEY[..KEY.len() - 1]),
        format!("{PASTE_START}{}{PASTE_END}", &KEY[..KEY.len() - 1]),
        // Two keys, and the 52 characters of a 32-byte key of the form before.
        format!("{KEY} {KEY}"),
        "AAAQ-EAYE-AUDA-OCAJ-BIFQ-YDIO-B4IB-CEQT-CQKR-MFYY-DENB-WHA5-DYPQ".to_owned(),
    ];
    for input in &refused {
        assert!(RecoveryKey::parse(input).is_err(), "{input:?} is read");
    }
}
