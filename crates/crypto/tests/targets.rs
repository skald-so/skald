//! What the crate makes on every target it builds for, natively and in WASM: the same bytes from
//! fixed keys, and keys from the cryptographically secure generator of the operating system, the
//! browser or Node.js.
//!
//! The expected bytes are pinned: on a native target they match what the crate makes there, so in
//! WASM they check that it makes the same bytes.

use std::collections::HashSet;

use skald_crypto::{HpkeKey, SigningKey, check_device, issue, verify};
use skald_proto::{AccountId, Cbor, Cert, CertBody, DeviceId, DeviceKind, Unknown};

mod common;

use common::{hex, hex32};

fn hex64(digits: &str) -> [u8; 64] {
    hex(digits).try_into().unwrap()
}

/// The secret key of RFC 8032, §7.1, "TEST 1".
const RFC8032_SECRET: &str = "9d61b19deffd5a60ba844af492ec2cc44449c5697b326919703bac031cae7f60";
/// Its public key.
const RFC8032_PUBLIC: &str = "d75a980182b10ab7d54bfed3c964073a0ee172f3daa62325af021a68f707511a";
/// Alice's and Bob's private and public keys of RFC 7748, §6.1.
const RFC7748_ALICE_SECRET: &str =
    "77076d0a7318a57d3c16c17251b26645df4c2f87ebc0992ab177fba51db92c2a";
const RFC7748_ALICE_PUBLIC: &str =
    "8520f0098930a754748b7ddcb43ef75a0dbf3a0d26381af4eba4a98eaa9b4e6a";
const RFC7748_BOB_SECRET: &str = "5dab087e624a8a4b79e17f8b83800ee66f3bb1292618b6fd1c2f8b27ff88e0eb";
const RFC7748_BOB_PUBLIC: &str = "de9edb7d7b7dc1b4d35b61c2ece435373f8343c85b78674dadfc7e146f882b4f";

/// The signature of the RFC 8032 key over `["skald test", "first", ""]`, made natively.
const SIGNATURE: &str = "4d69eec428b731b4db84712ee94d54056fed622daf9d11e829a3e7bfe8a51185\
                         0020470b3139ddc7061eeb3e1c801b428c78bbf919f82525d85e7dc575bab008";

/// The certificate [`certificate`] makes, made natively.
const CERTIFICATE: &str = "83587ba70150019a6f1e2b3c7d4e8f5061a2b3c4acc1025019ac5f7849388235\
                           86a6c5809a18f6dd0301045820d75a980182b10ab7d54bfed3c964073a0ee172\
                           f3daa62325af021a68f707511a0558208520f0098930a754748b7ddcb43ef75a\
                           0dbf3a0d26381af4eba4a98eaa9b4e6a071a69127c8009664c6170746f705840\
                           59ab279280230366008b48d974df56135e9e5f92b1e86cea3d8e7e03e69e7de7\
                           e471f89ba33dc0a4b97c542c971c90ec239bc42f575fd8f1e2d53d5c7233480a\
                           58408e75ce6db184a70ec58911a281847b26a747e1816b32dd9dda891bec3728\
                           55d6ed9a81506f603bbcdc4dacf1ec9d83b996f462962812d7a14228010212bc\
                           d20b";

fn master() -> SigningKey {
    SigningKey::from_bytes(&[0x4d; 32])
}

fn device() -> SigningKey {
    SigningKey::from_bytes(&hex32(RFC8032_SECRET))
}

/// The certificate of the RFC 8032 key's `client` device, with Alice's HPKE key, issued by
/// [`master`].
fn certificate() -> Cert {
    let device = device();
    let body = CertBody {
        account: "019a6f1e-2b3c-7d4e-8f50-61a2b3c4acc1"
            .parse::<AccountId>()
            .unwrap(),
        device: DeviceId::from_signing_key(&device.public_key()),
        kind: DeviceKind::Client,
        signing: device.public_key(),
        hpke: hex32(RFC7748_ALICE_PUBLIC),
        issuer: None,
        issued: 1_762_819_200,
        ssh: None,
        name: "Laptop".to_owned(),
        unknown: Unknown::default(),
    };
    issue(&body, &master(), &device).unwrap()
}

#[cfg_attr(not(target_arch = "wasm32"), test)]
#[cfg_attr(target_arch = "wasm32", wasm_bindgen_test::wasm_bindgen_test)]
fn fixed_secrets_give_the_rfc_public_keys() {
    assert_eq!(device().public_key(), hex32(RFC8032_PUBLIC));
    for (secret, public) in [
        (RFC7748_ALICE_SECRET, RFC7748_ALICE_PUBLIC),
        (RFC7748_BOB_SECRET, RFC7748_BOB_PUBLIC),
    ] {
        let key = HpkeKey::from_bytes(&hex32(secret));
        assert_eq!(key.public_key(), hex32(public), "{secret}");
        assert_eq!(*key.to_bytes(), hex32(secret));
    }
}

#[cfg_attr(not(target_arch = "wasm32"), test)]
#[cfg_attr(target_arch = "wasm32", wasm_bindgen_test::wasm_bindgen_test)]
fn signatures_with_a_fixed_key_are_the_native_bytes() {
    let parts: [&[u8]; 2] = [b"first", b""];
    let signature = device().sign("skald test", &parts);
    assert_eq!(signature, hex64(SIGNATURE));
    assert_eq!(
        verify(&hex32(RFC8032_PUBLIC), "skald test", &parts, &signature),
        Ok(())
    );
}

#[cfg_attr(not(target_arch = "wasm32"), test)]
#[cfg_attr(target_arch = "wasm32", wasm_bindgen_test::wasm_bindgen_test)]
fn certificates_with_fixed_keys_are_the_native_bytes() {
    let cert = certificate();
    assert_eq!(cert.to_cbor(), hex(CERTIFICATE));
    let certs = [cert.clone()];
    let device = DeviceId::from_signing_key(&hex32(RFC8032_PUBLIC));
    assert_eq!(
        check_device(&master().public_key(), &certs, device),
        Ok(&cert)
    );
}

/// 16 keys from `generate`, after checking their secrets and public keys: no two are the same,
/// and no secret is all zeros.
fn generated<K>(
    generate: fn() -> K,
    secret: fn(&K) -> [u8; 32],
    public: fn(&K) -> [u8; 32],
) -> Vec<K> {
    let keys: Vec<K> = (0..16).map(|_| generate()).collect();
    let secrets: HashSet<[u8; 32]> = keys.iter().map(secret).collect();
    let publics: HashSet<[u8; 32]> = keys.iter().map(public).collect();
    assert_eq!(secrets.len(), keys.len());
    assert_eq!(publics.len(), keys.len());
    assert!(!secrets.contains(&[0; 32]));
    keys
}

#[cfg_attr(not(target_arch = "wasm32"), test)]
#[cfg_attr(target_arch = "wasm32", wasm_bindgen_test::wasm_bindgen_test)]
fn generated_signing_keys_differ() {
    let keys = generated(
        SigningKey::generate,
        |key| *key.to_bytes(),
        SigningKey::public_key,
    );
    // A generated key signs, and its signature verifies with its public key only.
    let signature = keys[0].sign("skald test", &[b"part"]);
    assert_eq!(
        verify(&keys[0].public_key(), "skald test", &[b"part"], &signature),
        Ok(())
    );
    assert!(verify(&keys[1].public_key(), "skald test", &[b"part"], &signature).is_err());
}

#[cfg_attr(not(target_arch = "wasm32"), test)]
#[cfg_attr(target_arch = "wasm32", wasm_bindgen_test::wasm_bindgen_test)]
fn generated_hpke_keys_differ() {
    generated(
        HpkeKey::generate,
        |key| *key.to_bytes(),
        HpkeKey::public_key,
    );
}
