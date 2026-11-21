//! The fixture certificate, its fields encoded with the tests' CBOR writer apart from the crates
//! under test, the SSH keys and the signatures `ssh-keygen` made over it, and the parts of an
//! SSH signature's binary form the tests take apart.

use skald_crypto::SshCheck;
use skald_proto::{AccountId, Cbor, CertBody, DeviceId};

use crate::cbor::{Value, bytes, text};

pub const CLIENT: u64 = 1;
pub const BROWSER: u64 = 2;
pub const SERVICE: u64 = 3;

/// The CBOR keys of `ssh` and of `github`, the GitHub user ID.
pub const SSH_FIELD: i64 = 8;
pub const GITHUB_FIELD: i64 = 10;

/// The GitHub user ID the certificates name.
pub const GITHUB: u64 = 583_231;

pub const SIGNING: [u8; 32] = [0x11; 32];
const HPKE: [u8; 32] = [0x22; 32];
const ISSUED: u64 = 1_762_819_200;

pub fn account() -> AccountId {
    "019a6f1e-2b3c-7d4e-8f50-61a2b3c4acc1".parse().unwrap()
}

pub fn other_account() -> AccountId {
    "019a6f1e-2b3c-7d4e-8f50-61a2b3c4acc2".parse().unwrap()
}

/// `0a0f8db0-155b-8745-a5ab-1ee5928b1105`. The fixed signatures sign the encoded
/// `["skald ssh", account, device, signing]`:
/// `8469736b616c642073736850019a6f1e2b3c7d4e8f5061a2b3c4acc1500a0f8db0155b8745a5ab1ee5928b1105`
/// `58201111111111111111111111111111111111111111111111111111111111111111`.
pub fn device() -> DeviceId {
    DeviceId::from_signing_key(&SIGNING)
}

/// The fields of the fixture certificate of `kind`, by the master key, without `ssh` and `github`.
pub fn fields(kind: u64) -> Vec<(i64, Value)> {
    vec![
        (1, bytes(account().as_bytes())),
        (2, bytes(device().as_bytes())),
        (3, Value::Uint(kind)),
        (4, bytes(&SIGNING)),
        (5, bytes(&HPKE)),
        (7, Value::Uint(ISSUED)),
        (9, text("Laptop")),
    ]
}

/// [`fields`] with `ssh` and [`GITHUB`].
pub fn signed_fields(kind: u64, ssh: &[u8]) -> Vec<(i64, Value)> {
    let mut fields = fields(kind);
    fields.push((SSH_FIELD, bytes(ssh)));
    fields.push((GITHUB_FIELD, Value::Uint(GITHUB)));
    fields
}

/// `fields` with the field at `key` set to `value`.
pub fn replaced(mut fields: Vec<(i64, Value)>, key: i64, value: Value) -> Vec<(i64, Value)> {
    fields.retain(|(k, _)| *k != key);
    fields.push((key, value));
    fields
}

pub fn encoded(fields: Vec<(i64, Value)>) -> Vec<u8> {
    Value::Map(fields).encode()
}

/// The certificate body of `fields`, which must decode.
pub fn body(fields: Vec<(i64, Value)>) -> CertBody {
    CertBody::from_cbor(&encoded(fields)).expect("the certificate body decodes")
}

/// A `client` certificate body of the fixture carrying `ssh` and [`GITHUB`].
pub fn signed_body(ssh: &[u8]) -> CertBody {
    body(signed_fields(CLIENT, ssh))
}

/// An encoded `cert` of the encoded `body`.
pub fn cert_bytes(body: &[u8]) -> Vec<u8> {
    Value::Array(vec![bytes(body), bytes(&[0x5a; 64]), bytes(&[0xa5; 64])]).encode()
}

// The public keys, as GitHub's API lists them: `<type> <base64>`, with no comment.

pub const ED25519: &str =
    "ssh-ed25519 AAAAC3NzaC1lZDI1NTE5AAAAIKIM7diVtSBxrs0iOAGP4ZTsnEjnS58n+iOs/Y5u4UHK";
pub const ECDSA: &str = "ecdsa-sha2-nistp256 AAAAE2VjZHNhLXNoYTItbmlzdHAyNTYAAAAIbmlzdHAyNTYAAABBBHCzCkhfxBnX8LJBBB9ErckyhGDTVTEcbQTXZOgXwHm55Y088c74DYKAnGhlQ+vrQ5ZQzJFCay9M10cvuAxNBM4=";
pub const RSA: &str = "ssh-rsa AAAAB3NzaC1yc2EAAAADAQABAAABAQDd7winsXX/wLUMCTufvJWkQJ4UDnVw0WjFiFnYglPR6pSq4Ox87SKOpotMDzqy71U02RHTlbBElHKpUE+sBwkId9CGGtMWMpQ6Jtbymt8Qm9NONtbnzrYWaT9l1/m8IxyeiuDmAr7MG7Gz7OGn95D37Wp0vd73vF54195XWDr200j32hRzCYYbheb3BqEbAp2Q4FpDQy3bMqXfXAYTgxULPEp0OrnK9gXe8tvH7J7/1zjEfYviiIu7Lqd4XHzRL9Bvi5v8JI6OyqbfgsfIKmg2sQjSnY2byGqKLQuRrYuRpusfgk1DDnXtow57vwp2wvH/dkxLiCmHWwjfRyWeSqUP";
pub const OTHER_ED25519: &str =
    "ssh-ed25519 AAAAC3NzaC1lZDI1NTE5AAAAIH9Si1Kc/kojssIajcsNkkkDCvy6boic0klcalI12xXh";
pub const OTHER_ECDSA: &str = "ecdsa-sha2-nistp256 AAAAE2VjZHNhLXNoYTItbmlzdHAyNTYAAAAIbmlzdHAyNTYAAABBBLhTVNU/usLLOY8cWslQ4BoRxQxNIJtfPysWPPw0wpj8umYLd9P7XF0C4XFNrDTeAJJqFOTOtF+ASfLRXmH/JME=";

/// Every key of the fixtures, the signers among others: `ED25519` is at 2, `ECDSA` at 3, `RSA` at 4.
pub const KEYS: [&str; 5] = [OTHER_ED25519, OTHER_ECDSA, ED25519, ECDSA, RSA];

// The signatures over the fixture certificate, in namespace `skald.so` unless named otherwise,
// as the base64 of their armored text.

const ED25519_SIG: &str = "U1NIU0lHAAAAAQAAADMAAAALc3NoLWVkMjU1MTkAAAAgogzt2JW1IHGuzSI4AY/hlOycSOdLnyf6I6z9jm7hQcoAAAAIc2thbGQuc28AAAAAAAAABnNoYTUxMgAAAFMAAAALc3NoLWVkMjU1MTkAAABAPbxDGNr3pL9o9nZ/QmbQXoR+RbTVQF/28SvzDlOESD0jLy2qrF5MNpCcG/4PyjiWS8e58WbGNyDAp9IKbs5YCQ==";
const ECDSA_SIG: &str = "U1NIU0lHAAAAAQAAAGgAAAATZWNkc2Etc2hhMi1uaXN0cDI1NgAAAAhuaXN0cDI1NgAAAEEEcLMKSF/EGdfwskEEH0StyTKEYNNVMRxtBNdk6BfAebnljTzxzvgNgoCcaGVD6+tDllDMkUJrL0zXRy+4DE0EzgAAAAhza2FsZC5zbwAAAAAAAAAGc2hhNTEyAAAAZQAAABNlY2RzYS1zaGEyLW5pc3RwMjU2AAAASgAAACEAtZ4rHriGoj1wQr5i12KZ2mZO8ImDvF1nV/GQBnIDxbQAAAAhAIcCngTB2ZQT1oJ1CCOFiuZing65jQWLCNa8xMCypRCZ";
const RSA_SIG: &str = "U1NIU0lHAAAAAQAAARcAAAAHc3NoLXJzYQAAAAMBAAEAAAEBAN3vCKexdf/AtQwJO5+8laRAnhQOdXDRaMWIWdiCU9HqlKrg7HztIo6mi0wPOrLvVTTZEdOVsESUcqlQT6wHCQh30IYa0xYylDom1vKa3xCb00421ufOthZpP2XX+bwjHJ6K4OYCvswbsbPs4af3kPftanS93ve8XnjX3ldYOvbTSPfaFHMJhhuF5vcGoRsCnZDgWkNDLdsypd9cBhODFQs8SnQ6ucr2Bd7y28fsnv/XOMR9i+KIi7sup3hcfNEv0G+Lm/wkjo7Kpt+Cx8gqaDaxCNKdjZvIaootC5Gti5Gm6x+CTUMOde2jDnu/CnbC8f92TEuIKYdbCN9HJZ5KpQ8AAAAIc2thbGQuc28AAAAAAAAABnNoYTUxMgAAARQAAAAMcnNhLXNoYTItNTEyAAABAF8X25j/4ZuzuV1A9aW86zvbYqs1xnM0B35fAcim2rYwE/P3Xji11vCcnG+INL3pfYVh8NIkTn5INZaxShndULD65lhVdesgN0v1TXJ1w7ysAVJbvP1DeyIyjRtPZHvKC/rJMR9BYqusdeyum4adnHiBnvEIbRslxx3OPqFCtF65jJbSXpfYs7ajsnzJhWSQfyACVAry0WMT/xEnCQE1ugze7tufMQGhvDsD9GUvQgGi5Zr1CkO7lDAS+1VKXO32Yi00HLfp/BKsMo0xlC7D/5eVcrIiyPhyDVLR2+kUpbWOpJMlk0oi8JTNo7SfuujFlCLfzw5JxGXp7dDQwMn3z+M=";
/// By the `ED25519` key in namespace `git`, as for a signed commit.
const ED25519_GIT_SIG: &str = "U1NIU0lHAAAAAQAAADMAAAALc3NoLWVkMjU1MTkAAAAgogzt2JW1IHGuzSI4AY/hlOycSOdLnyf6I6z9jm7hQcoAAAADZ2l0AAAAAAAAAAZzaGE1MTIAAABTAAAAC3NzaC1lZDI1NTE5AAAAQGBffhkx07YY6rbKCB1XRuxNprZDbRveO/s6lbFsW+LdCv40MapT2Q5CvOJ1o0kN7csVwt8O0BswlePs8KnMjAg=";

pub fn ed25519_sig() -> Vec<u8> {
    from_base64(ED25519_SIG)
}

pub fn ecdsa_sig() -> Vec<u8> {
    from_base64(ECDSA_SIG)
}

pub fn rsa_sig() -> Vec<u8> {
    from_base64(RSA_SIG)
}

pub fn ed25519_git_sig() -> Vec<u8> {
    from_base64(ED25519_GIT_SIG)
}

/// The armored text of the `ED25519` signature, as `ssh-keygen` writes it.
pub fn ed25519_sig_armored() -> Vec<u8> {
    format!("-----BEGIN SSH SIGNATURE-----\n{ED25519_SIG}\n-----END SSH SIGNATURE-----\n")
        .into_bytes()
}

// An SSH signature's binary form: `SSHSIG`, a version, then SSH strings, each a 4-byte big-endian
// length and its bytes: the public key, the namespace, a reserved one, the hash algorithm and the
// signature.

/// An SSH string of `value`.
pub fn ssh_string(value: &[u8]) -> Vec<u8> {
    let len = u32::try_from(value.len()).unwrap().to_be_bytes();
    [&len[..], value].concat()
}

/// The public key blob a signature carries, and where it ends.
fn public_key_blob(sig: &[u8]) -> (&[u8], usize) {
    assert_eq!(&sig[..6], b"SSHSIG", "an SSH signature");
    let len = u32::from_be_bytes(sig[10..14].try_into().unwrap()) as usize;
    (&sig[14..14 + len], 14 + len)
}

/// The public key line, `<type> <base64>`, of the key a signature names.
pub fn signer_line(sig: &[u8]) -> String {
    let (blob, _) = public_key_blob(sig);
    let len = u32::from_be_bytes(blob[..4].try_into().unwrap()) as usize;
    let kind = std::str::from_utf8(&blob[4..4 + len]).unwrap();
    format!("{kind} {}", to_base64(blob))
}

/// `sig` naming the public key of `line` in place of its own.
pub fn naming_key(sig: &[u8], line: &str) -> Vec<u8> {
    let (_, end) = public_key_blob(sig);
    let blob = from_base64(line.split_once(' ').unwrap().1);
    let parts: [&[u8]; 3] = [&sig[..10], &ssh_string(&blob), &sig[end..]];
    parts.concat()
}

// Base64 with padding, RFC 4648, §4.

const ALPHABET: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";

pub fn from_base64(text: &str) -> Vec<u8> {
    let sextets: Vec<u32> = text
        .bytes()
        .take_while(|&c| c != b'=')
        .map(|c| ALPHABET.iter().position(|&a| a == c).expect("base64") as u32)
        .collect();
    let mut out = Vec::new();
    for chunk in sextets.chunks(4) {
        let word = chunk
            .iter()
            .enumerate()
            .fold(0, |word, (at, sextet)| word | (sextet << (18 - 6 * at)));
        out.extend(&word.to_be_bytes()[1..chunk.len()]);
    }
    out
}

pub fn to_base64(data: &[u8]) -> String {
    let mut out = String::new();
    for chunk in data.chunks(3) {
        let word = chunk.iter().enumerate().fold(0u32, |word, (at, &byte)| {
            word | (u32::from(byte) << (16 - 8 * at))
        });
        for at in 0..4 {
            out.push(if at <= chunk.len() {
                char::from(ALPHABET[((word >> (18 - 6 * at)) & 63) as usize])
            } else {
                '='
            });
        }
    }
    out
}

/// Whether `result` is neither a pass nor a key change: no signature, or one not checked.
pub fn neither_passed_nor_failed(result: &SshCheck) -> bool {
    !matches!(result, SshCheck::Passed { .. } | SshCheck::Failed)
}
