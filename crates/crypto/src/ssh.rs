//! A certificate's SSH signature: `docs/protocol/membership.md`, "Device certificate".

use skald_proto::{CertBody, DeviceKind, SSH_LABEL, SSH_NAMESPACE, signed_data};
use ssh_key::encoding::Decode as _;
use ssh_key::public::KeyData;
use ssh_key::{PublicKey, SshSig};

/// The result of checking a certificate's `ssh` against a GitHub user's SSH keys.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SshCheck {
    /// The certificate carries no `ssh`.
    Unsigned,
    /// The certificate is not of a `client` device, so its `ssh` counts for nothing.
    Ignored,
    /// It verifies with the key at `key` in the list.
    Passed { key: usize },
    /// A key change: malformed, of another namespace, by a key not in the list or not verifying.
    Failed,
    /// By a key in the list of a type this client does not verify.
    NotChecked,
}

/// Checks `body`'s `ssh` against `keys`, OpenSSH public key lines such as GitHub's API lists for
/// the user `body.github` names: an SSH signature in namespace `skald.so` over the encoded
/// `["skald ssh", account, device, signing]`, verified for the key types and sizes the spec names.
/// A line that does not parse matches no signature.
pub fn check_ssh_signature(body: &CertBody, keys: &[&str]) -> SshCheck {
    if body.kind != DeviceKind::Client {
        return SshCheck::Ignored;
    }
    let Some(ssh) = body.ssh.as_deref() else {
        return SshCheck::Unsigned;
    };
    let Some(sig) = decode(ssh) else {
        return SshCheck::Failed;
    };
    let Some(key) = keys.iter().position(|line| {
        PublicKey::from_openssh(line).is_ok_and(|listed| listed.key_data() == sig.public_key())
    }) else {
        return SshCheck::Failed;
    };
    if !verifies(sig.public_key()) {
        return SshCheck::NotChecked;
    }
    let data = signed_data(
        SSH_LABEL,
        &[
            body.account.as_bytes(),
            body.device.as_bytes(),
            &body.signing,
        ],
    );
    match PublicKey::from(sig.public_key().clone()).verify(SSH_NAMESPACE, &data, &sig) {
        Ok(()) => SshCheck::Passed { key },
        Err(_) => SshCheck::Failed,
    }
}

/// The SSH signature of version 1 that `ssh` holds in its binary form, with nothing after it.
fn decode(ssh: &[u8]) -> Option<SshSig> {
    let mut rest = ssh;
    let sig = SshSig::decode(&mut rest).ok()?;
    (rest.is_empty() && sig.version() == SshSig::VERSION).then_some(sig)
}

/// The sizes of the RSA keys the client verifies, in bits: those the `rsa` crate takes.
const RSA_BITS: std::ops::RangeInclusive<u32> = 1024..=8192;

/// Whether the client verifies signatures by this key.
fn verifies(key: &KeyData) -> bool {
    match key {
        KeyData::Rsa(key) => RSA_BITS.contains(&key.key_size()),
        KeyData::Ed25519(_)
        | KeyData::Ecdsa(_)
        | KeyData::SkEd25519(_)
        | KeyData::SkEcdsaSha2NistP256(_) => true,
        _ => false,
    }
}

#[cfg(test)]
mod tests {
    //! Key types beyond those the integration tests in `tests/ssh_signatures` cover: ECDSA P-384
    //! and P-521, whose fixed signatures `ssh-keygen -Y sign -n skald.so` made with keys made for
    //! these tests alone, security keys, whose signatures the tests make as a security key would,
    //! RSA keys of sizes the client does not verify, a key of a type it does not verify that is
    //! not on the list, and the version and the end of a signature's binary form.

    use ed25519_dalek::Signer as _;
    use sha2::{Digest as _, Sha256};
    use skald_proto::{AccountId, DeviceId, Unknown};
    use ssh_key::encoding::Encode as _;
    use ssh_key::public::{Ed25519PublicKey, RsaPublicKey, SkEcdsaSha2NistP256, SkEd25519};
    use ssh_key::{Algorithm, HashAlg, Mpint, Signature};

    use super::*;

    const SIGNING: [u8; 32] = [0x11; 32];

    const P384: &str = "ecdsa-sha2-nistp384 AAAAE2VjZHNhLXNoYTItbmlzdHAzODQAAAAIbmlzdHAzODQAAABhBJGCBKP858qZH4ekK19v1rLyY36ClNC+N2T3pHwLs5Ygtrc1VwmEDY2rudpf0vijPdBfVq0UNTNMKkYz1sL9ZDIZGFoFt48PcddLiWre8nImv71MhlGeJLwnZSqBs/HExg==";
    const P384_SIG: &str = "-----BEGIN SSH SIGNATURE-----
U1NIU0lHAAAAAQAAAIgAAAATZWNkc2Etc2hhMi1uaXN0cDM4NAAAAAhuaXN0cDM4NAAAAGEEkYIEo/znypkfh6QrX2/WsvJjfoKU0L43ZPekfAuzliC2tzVXCYQNjau52l/S+KM90F9WrRQ1M0wqRjPWwv1kMhkYWgW3jw9x10uJat7ycia/vUyGUZ4kvCdlKoGz8cTGAAAACHNrYWxkLnNvAAAAAAAAAAZzaGE1MTIAAACDAAAAE2VjZHNhLXNoYTItbmlzdHAzODQAAABoAAAAMGfGuKwuhZLzUgeFKu9nnWrvSgruDUysAqtHMIc2EWo/qujM7md9xVAyCx2sMVDrPgAAADBZnbl7wc+EA0LMkN5D1Zuge/z1WbVJqenrypJdL1O0ufVqumeX2TSXpCk6ZiIbfbE=
-----END SSH SIGNATURE-----
";
    const P521: &str = "ecdsa-sha2-nistp521 AAAAE2VjZHNhLXNoYTItbmlzdHA1MjEAAAAIbmlzdHA1MjEAAACFBAHU2pDCUx37CIOEBAWYLrIAHhQpva4NkhiVDgpYRIaXgVKplLqIpyZQO31FJTW4LxwhublyGEGbCfLazdg0PujtkgHCddWSQrDkRNa/EsWKnGju4E86f6y2KvtrSqxsuOgjP+kD0gwML75EB4KspfceLSsSZtHlNn8sAisTwFBefP0G+w==";
    const P521_SIG: &str = "-----BEGIN SSH SIGNATURE-----
U1NIU0lHAAAAAQAAAKwAAAATZWNkc2Etc2hhMi1uaXN0cDUyMQAAAAhuaXN0cDUyMQAAAIUEAdTakMJTHfsIg4QEBZgusgAeFCm9rg2SGJUOClhEhpeBUqmUuoinJlA7fUUlNbgvHCG5uXIYQZsJ8trN2DQ+6O2SAcJ11ZJCsORE1r8SxYqcaO7gTzp/rLYq+2tKrGy46CM/6QPSDAwvvkQHgqyl9x4tKxJm0eU2fywCKxPAUF58/Qb7AAAACHNrYWxkLnNvAAAAAAAAAAZzaGE1MTIAAACnAAAAE2VjZHNhLXNoYTItbmlzdHA1MjEAAACMAAAAQgGYWf4suNPTBGJvtnQafmDJcbnDZvP+nW7QKwnPFbE2ZONFBysYt5N3DGcadtrk/8w5/kUHwvzC+ujZEX1nepjNyAAAAEIBGzk4U0T5geifbdzj8R3nINQfP01XoUsyEgEsWXq4kSpIkF3M1erclJP830mF+XM74IPTfJ9z/XAJI73mljXdmq4=
-----END SSH SIGNATURE-----
";

    /// The `client` certificate body the signatures sign, carrying `ssh`.
    fn body(ssh: Vec<u8>) -> CertBody {
        let account: AccountId = "019a6f1e-2b3c-7d4e-8f50-61a2b3c4acc1".parse().unwrap();
        CertBody {
            account,
            device: DeviceId::from_signing_key(&SIGNING),
            kind: DeviceKind::Client,
            signing: SIGNING,
            hpke: [0x22; 32],
            issuer: None,
            issued: 1_762_819_200,
            ssh: Some(ssh),
            github: Some(583_231),
            name: "Laptop".to_owned(),
            unknown: Unknown::default(),
        }
    }

    fn binary(pem: &str) -> Vec<u8> {
        SshSig::from_pem(pem).unwrap().encode_vec().unwrap()
    }

    /// What a security key signs for an SSH signature over [`body`]'s data: the SHA-256 of its
    /// application, `flags`, `counter` and the SHA-256 of the data `SshSig` wraps.
    fn sk_signed(application: &str, flags: u8, counter: u32) -> Vec<u8> {
        let data = signed_data(
            SSH_LABEL,
            &[
                body(vec![]).account.as_bytes(),
                body(vec![]).device.as_bytes(),
                &SIGNING,
            ],
        );
        let wrapped = SshSig::signed_data(SSH_NAMESPACE, HashAlg::Sha512, &data).unwrap();
        [
            &Sha256::digest(application)[..],
            &[flags],
            &counter.to_be_bytes(),
            &Sha256::digest(&wrapped)[..],
        ]
        .concat()
    }

    /// The binary form of an SSH signature by `key`, whose raw signature `raw` a security key made
    /// with `flags` and `counter`.
    fn sk_sig(key: KeyData, algorithm: Algorithm, raw: &[u8], flags: u8, counter: u32) -> Vec<u8> {
        let data = [raw, &[flags], &counter.to_be_bytes()].concat();
        let signature = Signature::new(algorithm, data).unwrap();
        SshSig::new(key, SSH_NAMESPACE, HashAlg::Sha512, signature)
            .unwrap()
            .encode_vec()
            .unwrap()
    }

    fn sk_ed25519() -> (Vec<u8>, String) {
        let secret = ed25519_dalek::SigningKey::from_bytes(&[0x31; 32]);
        let key = KeyData::SkEd25519(SkEd25519::new(
            Ed25519PublicKey(secret.verifying_key().to_bytes()),
            "ssh:",
        ));
        let raw = secret.sign(&sk_signed("ssh:", 1, 7)).to_bytes();
        let line = PublicKey::from(key.clone()).to_openssh().unwrap();
        (sk_sig(key, Algorithm::SkEd25519, &raw, 1, 7), line)
    }

    fn sk_ecdsa() -> (Vec<u8>, String) {
        let secret = p256::ecdsa::SigningKey::from_slice(&[0x32; 32]).unwrap();
        let point = ssh_key::public::EcdsaPublicKey::from(secret.verifying_key());
        let ssh_key::public::EcdsaPublicKey::NistP256(point) = point else {
            unreachable!("a P-256 key")
        };
        let key = KeyData::SkEcdsaSha2NistP256(SkEcdsaSha2NistP256::new(point, "ssh:"));
        let raw: p256::ecdsa::Signature =
            p256::ecdsa::signature::Signer::sign(&secret, &sk_signed("ssh:", 5, 9));
        let raw = Signature::try_from(raw).unwrap();
        let line = PublicKey::from(key.clone()).to_openssh().unwrap();
        (
            sk_sig(key, Algorithm::SkEcdsaSha2NistP256, raw.as_bytes(), 5, 9),
            line,
        )
    }

    #[cfg_attr(not(target_arch = "wasm32"), test)]
    #[cfg_attr(target_arch = "wasm32", wasm_bindgen_test::wasm_bindgen_test)]
    fn ecdsa_p384_and_p521_signatures_verify_with_their_keys() {
        assert_eq!(
            check_ssh_signature(&body(binary(P384_SIG)), &[P521, P384]),
            SshCheck::Passed { key: 1 }
        );
        assert_eq!(
            check_ssh_signature(&body(binary(P521_SIG)), &[P521, P384]),
            SshCheck::Passed { key: 0 }
        );
        assert_eq!(
            check_ssh_signature(&body(binary(P521_SIG)), &[P384]),
            SshCheck::Failed
        );
    }

    #[cfg_attr(not(target_arch = "wasm32"), test)]
    #[cfg_attr(target_arch = "wasm32", wasm_bindgen_test::wasm_bindgen_test)]
    fn a_signature_of_version_0_or_with_bytes_after_it_is_a_key_change() {
        let sig = binary(P384_SIG);
        let mut version = sig.clone();
        version[9] = 0;
        let mut longer = sig.clone();
        longer.push(0);
        for (sig, what) in [(version, "version 0"), (longer, "a byte after it")] {
            assert_eq!(
                check_ssh_signature(&body(sig), &[P384]),
                SshCheck::Failed,
                "{what}"
            );
        }
    }

    #[cfg_attr(not(target_arch = "wasm32"), test)]
    #[cfg_attr(target_arch = "wasm32", wasm_bindgen_test::wasm_bindgen_test)]
    fn security_key_signatures_verify_with_their_keys() {
        for (sig, line) in [sk_ed25519(), sk_ecdsa()] {
            assert_eq!(
                check_ssh_signature(&body(sig.clone()), &[P384, &line]),
                SshCheck::Passed { key: 1 },
                "{line}"
            );
            let mut flipped = sig.clone();
            // The counter, the last byte: what the security key signed changes.
            *flipped.last_mut().unwrap() ^= 1;
            assert_eq!(
                check_ssh_signature(&body(flipped), &[&line]),
                SshCheck::Failed,
                "{line}"
            );
            assert_eq!(
                check_ssh_signature(&body(sig), &[P384]),
                SshCheck::Failed,
                "{line}"
            );
        }
    }

    #[cfg_attr(not(target_arch = "wasm32"), test)]
    #[cfg_attr(target_arch = "wasm32", wasm_bindgen_test::wasm_bindgen_test)]
    fn a_signature_by_a_listed_rsa_key_of_another_size_is_not_checked() {
        for bytes in [127, 1025] {
            let n = Mpint::from_positive_bytes(&vec![0xc5; bytes]);
            let e = Mpint::from_positive_bytes(&[1, 0, 1]);
            let key = KeyData::Rsa(RsaPublicKey::new(e, n).unwrap());
            let line = PublicKey::from(key.clone()).to_openssh().unwrap();
            let signature = Signature::new(
                Algorithm::Rsa {
                    hash: Some(HashAlg::Sha512),
                },
                vec![0x24; bytes],
            )
            .unwrap();
            let sig = SshSig::new(key, SSH_NAMESPACE, HashAlg::Sha512, signature)
                .unwrap()
                .encode_vec()
                .unwrap();
            assert_eq!(
                check_ssh_signature(&body(sig.clone()), &[P384, &line]),
                SshCheck::NotChecked,
                "{} bits",
                bytes * 8
            );
            assert_eq!(
                check_ssh_signature(&body(sig), &[P384]),
                SshCheck::Failed,
                "{} bits",
                bytes * 8
            );
        }
    }

    #[cfg_attr(not(target_arch = "wasm32"), test)]
    #[cfg_attr(target_arch = "wasm32", wasm_bindgen_test::wasm_bindgen_test)]
    fn a_signature_by_a_key_type_it_does_not_verify_and_not_listed_is_a_key_change() {
        let key = KeyData::Other(ssh_key::public::OpaquePublicKey::new(
            vec![0x42; 32],
            "ssh-unknown@example.com".parse().unwrap(),
        ));
        let signature = Signature::new(
            Algorithm::Other("ssh-unknown@example.com".parse().unwrap()),
            vec![0x24; 64],
        )
        .unwrap();
        let sig = SshSig::new(key, SSH_NAMESPACE, HashAlg::Sha512, signature)
            .unwrap()
            .encode_vec()
            .unwrap();
        assert_eq!(check_ssh_signature(&body(sig), &[P384]), SshCheck::Failed);
    }
}
