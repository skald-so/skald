//! Checking a certificate's `ssh` against a list of OpenSSH public keys, as GitHub lists a user's:
//! it passes with the key it verifies with; one that verifies with none of them is a key change;
//! one by a key type the client does not verify is not checked; a certificate of another kind than
//! `client` has its `ssh` ignored.

use skald_crypto::{SshCheck, check_ssh_signature};
use skald_proto::DeviceId;

use crate::cbor::bytes;
use crate::fixtures::{
    BROWSER, CLIENT, ECDSA, ED25519, KEYS, OTHER_ECDSA, OTHER_ED25519, RSA, SERVICE, body,
    ecdsa_sig, ed25519_git_sig, ed25519_sig, ed25519_sig_armored, fields, naming_key,
    neither_passed_nor_failed, other_account, replaced, rsa_sig, signed_body, signed_fields,
    ssh_string, to_base64,
};

fn assert_passes(sig: &[u8], key: usize) {
    assert_eq!(
        check_ssh_signature(&signed_body(sig), &KEYS),
        SshCheck::Passed { key }
    );
}

#[cfg_attr(not(target_arch = "wasm32"), test)]
#[cfg_attr(target_arch = "wasm32", wasm_bindgen_test::wasm_bindgen_test)]
fn an_ed25519_signature_verifies_with_its_key_among_others() {
    assert_passes(&ed25519_sig(), 2);
    assert_eq!(
        check_ssh_signature(&signed_body(&ed25519_sig()), &[ED25519]),
        SshCheck::Passed { key: 0 }
    );
}

#[cfg_attr(not(target_arch = "wasm32"), test)]
#[cfg_attr(target_arch = "wasm32", wasm_bindgen_test::wasm_bindgen_test)]
fn an_ecdsa_p256_signature_verifies_with_its_key_among_others() {
    assert_passes(&ecdsa_sig(), 3);
}

#[cfg_attr(not(target_arch = "wasm32"), test)]
#[cfg_attr(target_arch = "wasm32", wasm_bindgen_test::wasm_bindgen_test)]
fn an_rsa_signature_verifies_with_its_key_among_others() {
    assert_passes(&rsa_sig(), 4);
}

#[cfg_attr(not(target_arch = "wasm32"), test)]
#[cfg_attr(target_arch = "wasm32", wasm_bindgen_test::wasm_bindgen_test)]
fn a_signature_in_another_namespace_is_a_key_change() {
    assert_eq!(
        check_ssh_signature(&signed_body(&ed25519_git_sig()), &KEYS),
        SshCheck::Failed
    );
}

#[cfg_attr(not(target_arch = "wasm32"), test)]
#[cfg_attr(target_arch = "wasm32", wasm_bindgen_test::wasm_bindgen_test)]
fn a_signature_over_another_account_device_or_signing_key_is_a_key_change() {
    let other_device = DeviceId::from_signing_key(&[0x13; 32]);
    for (key, value, what) in [
        (1, bytes(other_account().as_bytes()), "another account"),
        (2, bytes(other_device.as_bytes()), "another device"),
        (4, bytes(&[0x12; 32]), "another signing key"),
    ] {
        for sig in [ed25519_sig(), ecdsa_sig(), rsa_sig()] {
            let changed = body(replaced(signed_fields(CLIENT, &sig), key, value.clone()));
            assert_eq!(
                check_ssh_signature(&changed, &KEYS),
                SshCheck::Failed,
                "{what}"
            );
        }
    }
}

#[cfg_attr(not(target_arch = "wasm32"), test)]
#[cfg_attr(target_arch = "wasm32", wasm_bindgen_test::wasm_bindgen_test)]
fn a_signature_by_a_key_not_in_the_list_is_a_key_change() {
    let body = signed_body(&ed25519_sig());
    assert_eq!(
        check_ssh_signature(&body, &[OTHER_ED25519, OTHER_ECDSA, ECDSA, RSA]),
        SshCheck::Failed
    );
    assert_eq!(check_ssh_signature(&body, &[]), SshCheck::Failed);
}

#[cfg_attr(not(target_arch = "wasm32"), test)]
#[cfg_attr(target_arch = "wasm32", wasm_bindgen_test::wasm_bindgen_test)]
fn a_signature_naming_a_listed_key_it_was_not_made_with_is_a_key_change() {
    let forged = naming_key(&ed25519_sig(), OTHER_ED25519);
    assert_eq!(
        check_ssh_signature(&signed_body(&forged), &KEYS),
        SshCheck::Failed
    );
    assert_eq!(
        check_ssh_signature(&signed_body(&forged), &[OTHER_ED25519]),
        SshCheck::Failed
    );
}

#[cfg_attr(not(target_arch = "wasm32"), test)]
#[cfg_attr(target_arch = "wasm32", wasm_bindgen_test::wasm_bindgen_test)]
fn a_malformed_signature_is_a_key_change() {
    let sig = ed25519_sig();
    let mut flipped = sig.clone();
    *flipped.last_mut().unwrap() ^= 1;
    let mut magic = sig.clone();
    magic[5] = b'H';
    let mut version = sig.clone();
    version[9] = 2;
    for (malformed, what) in [
        (flipped, "a bit of the signature flipped"),
        (magic, "another magic"),
        (version, "another version"),
        (ed25519_sig_armored(), "the armored text"),
        (b"not a signature".to_vec(), "other bytes"),
        (vec![], "no bytes"),
    ] {
        assert_eq!(
            check_ssh_signature(&signed_body(&malformed), &KEYS),
            SshCheck::Failed,
            "{what}"
        );
    }
}

#[cfg_attr(not(target_arch = "wasm32"), test)]
#[cfg_attr(target_arch = "wasm32", wasm_bindgen_test::wasm_bindgen_test)]
fn a_truncated_signature_is_a_key_change() {
    for sig in [ed25519_sig(), ecdsa_sig(), rsa_sig()] {
        for at in 0..sig.len() {
            assert_eq!(
                check_ssh_signature(&signed_body(&sig[..at]), &KEYS),
                SshCheck::Failed,
                "the first {at} of {} bytes",
                sig.len()
            );
        }
    }
}

#[cfg_attr(not(target_arch = "wasm32"), test)]
#[cfg_attr(target_arch = "wasm32", wasm_bindgen_test::wasm_bindgen_test)]
fn a_signature_by_a_key_type_it_does_not_verify_is_not_checked() {
    let kind = b"ssh-unknown@example.com";
    let blob = [ssh_string(kind), ssh_string(&[0x42; 32])].concat();
    let line = format!("ssh-unknown@example.com {}", to_base64(&blob));
    let parts: [&[u8]; 7] = [
        b"SSHSIG",
        &1u32.to_be_bytes(),
        &ssh_string(&blob),
        &ssh_string(b"skald.so"),
        &ssh_string(b""),
        &ssh_string(b"sha512"),
        &ssh_string(&[ssh_string(kind), ssh_string(&[0x24; 64])].concat()),
    ];
    let sig = parts.concat();
    let result = check_ssh_signature(&signed_body(&sig), &[line.as_str(), ED25519]);
    assert!(neither_passed_nor_failed(&result), "{result:?}");
}

#[cfg_attr(not(target_arch = "wasm32"), test)]
#[cfg_attr(target_arch = "wasm32", wasm_bindgen_test::wasm_bindgen_test)]
fn a_browser_or_service_certificate_s_signature_is_ignored() {
    for kind in [BROWSER, SERVICE] {
        for sig in [ed25519_sig(), b"not a signature".to_vec()] {
            let result = check_ssh_signature(&body(signed_fields(kind, &sig)), &KEYS);
            assert!(
                neither_passed_nor_failed(&result),
                "kind {kind}: {result:?}"
            );
        }
    }
}

#[cfg_attr(not(target_arch = "wasm32"), test)]
#[cfg_attr(target_arch = "wasm32", wasm_bindgen_test::wasm_bindgen_test)]
fn a_certificate_without_an_ssh_signature_is_neither_passed_nor_a_key_change() {
    let result = check_ssh_signature(&body(fields(CLIENT)), &KEYS);
    assert!(neither_passed_nor_failed(&result), "{result:?}");
}
