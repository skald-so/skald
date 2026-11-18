//! The recovery key: 32 bytes from a cryptographically secure generator, shown as base32 of RFC
//! 4648, §6, without padding, in groups of four characters joined by `-`; its input ignores case,
//! spaces and `-`.

use skald_crypto::RecoveryKey;

use crate::common::hex32;
use crate::fixtures::{base32, in_shown_form};

/// 32 bytes 00 to 1f, and their shown form.
const SEQUENCE: &str = "000102030405060708090a0b0c0d0e0f101112131415161718191a1b1c1d1e1f";
const SEQUENCE_SHOWN: &str = "AAAQ-EAYE-AUDA-OCAJ-BIFQ-YDIO-B4IB-CEQT-CQKR-MFYY-DENB-WHA5-DYPQ";

/// The bytes `input` reads as; fails if it is refused.
#[track_caller]
fn read(input: &str) -> [u8; 32] {
    *RecoveryKey::parse(input)
        .unwrap_or_else(|_| panic!("{input:?} is refused"))
        .to_bytes()
}

#[cfg_attr(not(target_arch = "wasm32"), test)]
#[cfg_attr(target_arch = "wasm32", wasm_bindgen_test::wasm_bindgen_test)]
fn a_recovery_key_is_shown_as_base32_in_groups_of_four() {
    let key = RecoveryKey::from_bytes(&hex32(SEQUENCE));
    assert_eq!(key.shown().as_str(), SEQUENCE_SHOWN);
    assert_eq!(*key.to_bytes(), hex32(SEQUENCE));
}

#[cfg_attr(not(target_arch = "wasm32"), test)]
#[cfg_attr(target_arch = "wasm32", wasm_bindgen_test::wasm_bindgen_test)]
fn a_generated_recovery_key_is_new_each_time_and_reads_back_from_its_shown_form() {
    let keys = [RecoveryKey::generate(), RecoveryKey::generate()];
    for key in &keys {
        let shown = key.shown();
        assert!(in_shown_form(shown.as_str()), "{}", shown.as_str());
        let digits: String = shown.as_str().split('-').collect();
        assert_eq!(
            base32(&digits).as_deref(),
            Some(key.to_bytes().as_slice()),
            "base32 of the key's bytes"
        );
        assert_eq!(read(shown.as_str()), *key.to_bytes());
    }
    assert_ne!(*keys[0].to_bytes(), *keys[1].to_bytes());
    assert_ne!(*keys[0].to_bytes(), [0; 32]);
}

#[cfg_attr(not(target_arch = "wasm32"), test)]
#[cfg_attr(target_arch = "wasm32", wasm_bindgen_test::wasm_bindgen_test)]
fn input_ignores_case_spaces_and_dashes() {
    let expected = hex32(SEQUENCE);
    let joined: String = SEQUENCE_SHOWN.split('-').collect();
    let inputs = [
        SEQUENCE_SHOWN.to_owned(),
        SEQUENCE_SHOWN.to_lowercase(),
        "aAaQ-EaYe-AUDA-ocaj-BIFQ-YDIO-b4ib-CEQT-CQKR-MFYY-DENB-wha5-DYPQ".to_owned(),
        joined.clone(),
        joined.to_lowercase(),
        SEQUENCE_SHOWN.replace('-', " "),
        SEQUENCE_SHOWN.replace('-', " - "),
        format!("  {}  ", SEQUENCE_SHOWN.replace('-', "  ")),
        format!("-{}-", SEQUENCE_SHOWN.replace('-', "--")),
        joined
            .chars()
            .enumerate()
            .map(|(at, c)| {
                if at % 3 == 2 {
                    format!("{c} ")
                } else {
                    c.to_string()
                }
            })
            .collect(),
    ];
    for input in &inputs {
        assert_eq!(read(input), expected, "{input:?}");
    }
}

#[cfg_attr(not(target_arch = "wasm32"), test)]
#[cfg_attr(target_arch = "wasm32", wasm_bindgen_test::wasm_bindgen_test)]
fn input_of_another_length_or_character_is_refused() {
    let joined: String = SEQUENCE_SHOWN.split('-').collect();
    let mut inputs = vec![
        String::new(),
        " - ".to_owned(),
        // A character short and a character over.
        joined[..51].to_owned(),
        format!("{joined}A"),
        SEQUENCE_SHOWN[..SEQUENCE_SHOWN.len() - 1].to_owned(),
        format!("{SEQUENCE_SHOWN}-AAAA"),
        // Padding.
        format!("{joined}===="),
        format!("{joined}="),
    ];
    // Characters outside the alphabet, in place of the second one.
    for c in ['0', '1', '8', '9', '=', '_', '+', '/', '.', 'é', 'Ａ'] {
        let mut input: Vec<char> = SEQUENCE_SHOWN.chars().collect();
        input[1] = c;
        inputs.push(input.into_iter().collect());
    }
    for input in &inputs {
        assert!(RecoveryKey::parse(input).is_err(), "{input:?} is read");
    }
}
