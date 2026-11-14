//! The single-shot HPKE seal and open in base mode with DHKEM(X25519, HKDF-SHA256), HKDF-SHA256
//! and AES-128-GCM, with any `info` and `aad` (RFC 9180, §6.1).

use skald_crypto::HpkeKey;

use crate::fixtures::{Device, hex, hex32};

/// RFC 9180, Appendix A.1.1: base mode of this suite, and its encryption at sequence number 0.
const SK_RM: &str = "4612c550263fc8ad58375df3f557aac531d26850903e55a9f23f21d8534e8ac8";
const PK_RM: &str = "3948cfe0ad1ddb695d780e59077195da6c56506b027329794ab02bca80815c4d";
const ENC: &str = "37fda3567bdbd628e88668c3c8d7e97d1d1253b6d4ea6d44c150f741f1bf4431";
const INFO: &str = "4f6465206f6e2061204772656369616e2055726e";
const PT: &str = "4265617574792069732074727574682c20747275746820626561757479";
const AAD: &str = "436f756e742d30";
const CT: &str = "f938558b5d72f1a23810b4be2ab4f84331acc02fc97babc53a52ae8218a355a9
6d8770ac83d07bea87e13c512a";

fn bytes(opened: impl AsRef<[u8]>) -> Vec<u8> {
    opened.as_ref().to_vec()
}

#[cfg_attr(not(target_arch = "wasm32"), test)]
#[cfg_attr(target_arch = "wasm32", wasm_bindgen_test::wasm_bindgen_test)]
fn the_rfc_base_mode_vector_opens() {
    let key = HpkeKey::from_bytes(&hex32(SK_RM));
    assert_eq!(key.public_key(), hex32(PK_RM));
    let opened = key
        .open(&hex32(ENC), &hex(INFO), &hex(AAD), &hex(CT))
        .expect("the vector opens");
    assert_eq!(bytes(opened), hex(PT));
}

#[cfg_attr(not(target_arch = "wasm32"), test)]
#[cfg_attr(target_arch = "wasm32", wasm_bindgen_test::wasm_bindgen_test)]
fn the_rfc_vector_opens_only_with_its_key_info_aad_enc_and_ct() {
    let key = HpkeKey::from_bytes(&hex32(SK_RM));
    let (enc, info, aad, ct) = (hex32(ENC), hex(INFO), hex(AAD), hex(CT));
    assert!(
        Device::Bob.hpke().open(&enc, &info, &aad, &ct).is_err(),
        "another key"
    );
    assert!(
        key.open(&enc, b"skald hpke", &aad, &ct).is_err(),
        "another info"
    );
    assert!(
        key.open(&enc, &info, b"Count-1", &ct).is_err(),
        "another aad"
    );
    assert!(key.open(&enc, &info, &[], &ct).is_err(), "no aad");
    let mut other_enc = enc;
    other_enc[0] ^= 1;
    assert!(
        key.open(&other_enc, &info, &aad, &ct).is_err(),
        "another enc"
    );
    for at in [0, ct.len() / 2, ct.len() - 1] {
        let mut other_ct = ct.clone();
        other_ct[at] ^= 1;
        assert!(
            key.open(&enc, &info, &aad, &other_ct).is_err(),
            "ct changed at {at}"
        );
    }
    for len in 0..ct.len() {
        assert!(
            key.open(&enc, &info, &aad, &ct[..len]).is_err(),
            "ct cut to {len}"
        );
    }
}

#[cfg_attr(not(target_arch = "wasm32"), test)]
#[cfg_attr(target_arch = "wasm32", wasm_bindgen_test::wasm_bindgen_test)]
fn a_seal_opens_with_its_recipients_key_info_and_aad() {
    let bob = Device::Bob.hpke();
    for (info, aad, plaintext) in [
        (
            &b"skald history wrap"[..],
            &b"channel, epoch, recipient"[..],
            &b"a key"[..],
        ),
        (b"", b"", b""),
        (b"any info", b"", &[7; 1000][..]),
    ] {
        let (enc, ct) = HpkeKey::seal(&bob.public_key(), info, aad, plaintext).expect("sealed");
        assert_eq!(ct.len(), plaintext.len() + 16, "the AES-128-GCM tag");
        assert_eq!(
            bytes(bob.open(&enc, info, aad, &ct).expect("opens")),
            plaintext
        );
        assert!(
            Device::Carol.hpke().open(&enc, info, aad, &ct).is_err(),
            "another key"
        );
        assert!(
            bob.open(&enc, b"other info", aad, &ct).is_err(),
            "another info"
        );
        assert!(
            bob.open(&enc, info, b"other aad", &ct).is_err(),
            "another aad"
        );
    }
}

#[cfg_attr(not(target_arch = "wasm32"), test)]
#[cfg_attr(target_arch = "wasm32", wasm_bindgen_test::wasm_bindgen_test)]
fn each_seal_takes_a_new_ephemeral_key() {
    let bob = Device::Bob.hpke().public_key();
    let (first, first_ct) = HpkeKey::seal(&bob, b"info", b"aad", b"same").unwrap();
    let (second, second_ct) = HpkeKey::seal(&bob, b"info", b"aad", b"same").unwrap();
    assert_ne!(first, second);
    assert_ne!(first_ct, second_ct);
}

#[cfg_attr(not(target_arch = "wasm32"), test)]
#[cfg_attr(target_arch = "wasm32", wasm_bindgen_test::wasm_bindgen_test)]
fn sealing_to_a_small_order_key_is_refused() {
    // X25519 points of small order give an all-zero shared secret: RFC 9180, §7.1.4.
    let mut one = [0; 32];
    one[0] = 1;
    for key in [[0; 32], one] {
        assert!(
            HpkeKey::seal(&key, b"info", b"aad", b"secret").is_err(),
            "{key:?}"
        );
    }
}
