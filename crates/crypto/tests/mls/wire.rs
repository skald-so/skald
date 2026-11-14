//! Reads a KeyPackage from its bytes as RFC 9420, §2.1 and §10, encodes it, apart from the crate
//! under test, and checks its signatures (§5.1.2).

use ed25519_dalek::{Signature, VerifyingKey};

/// `mls10`.
pub const MLS10: u16 = 1;
/// `MLS_128_DHKEMX25519_AES128GCM_SHA256_Ed25519`.
pub const PROFILE_SUITE: u16 = 0x0001;
/// The `basic` credential type.
pub const BASIC_CREDENTIAL: u16 = 1;
/// The `key_package` leaf node source.
pub const FROM_KEY_PACKAGE: u8 = 1;
/// The `mls_key_package` wire format of an `MLSMessage`.
const KEY_PACKAGE_MESSAGE: u16 = 5;

/// The fields of a KeyPackage the tests check.
#[derive(Debug)]
pub struct KeyPackage {
    pub cipher_suite: u16,
    pub init_key: Vec<u8>,
    pub leaf: LeafNode,
    /// `KeyPackageTBS`: the encoded KeyPackage up to its signature.
    pub to_be_signed: Vec<u8>,
    pub signature: Vec<u8>,
}

/// The fields of a leaf node the tests check.
#[derive(Debug)]
pub struct LeafNode {
    pub encryption_key: Vec<u8>,
    pub signature_key: Vec<u8>,
    pub credential_type: u16,
    /// A `basic` credential's `identity`.
    pub identity: Vec<u8>,
    /// `capabilities.cipher_suites`.
    pub cipher_suites: Vec<u16>,
    pub source: u8,
    /// `LeafNodeTBS` of a leaf from a KeyPackage: the encoded leaf up to its signature.
    pub to_be_signed: Vec<u8>,
    pub signature: Vec<u8>,
}

/// A KeyPackage, bare or in an `MLSMessage`.
///
/// # Panics
/// If `bytes` is not one, or has bytes after it.
pub fn key_package(bytes: &[u8]) -> KeyPackage {
    let mut reader = Reader { bytes, at: 0 };
    assert_eq!(reader.u16(), MLS10, "the protocol version");
    if reader.bytes[reader.at..].starts_with(&KEY_PACKAGE_MESSAGE.to_be_bytes()) {
        reader.u16();
        assert_eq!(reader.u16(), MLS10, "the KeyPackage's protocol version");
    }
    let start = reader.at - 2;
    let cipher_suite = reader.u16();
    let init_key = reader.vector().to_vec();
    let leaf = leaf_node(&mut reader);
    reader.vector(); // extensions
    let to_be_signed = bytes[start..reader.at].to_vec();
    let signature = reader.vector().to_vec();
    assert_eq!(reader.at, bytes.len(), "bytes after the KeyPackage");
    KeyPackage {
        cipher_suite,
        init_key,
        leaf,
        to_be_signed,
        signature,
    }
}

fn leaf_node(reader: &mut Reader<'_>) -> LeafNode {
    let start = reader.at;
    let encryption_key = reader.vector().to_vec();
    let signature_key = reader.vector().to_vec();
    let credential_type = reader.u16();
    let identity = if credential_type == BASIC_CREDENTIAL {
        reader.vector().to_vec()
    } else {
        Vec::new()
    };
    reader.vector(); // capabilities.versions
    let mut suites = Reader {
        bytes: reader.vector(),
        at: 0,
    };
    let mut cipher_suites = Vec::new();
    while suites.at < suites.bytes.len() {
        cipher_suites.push(suites.u16());
    }
    for _ in ["extensions", "proposals", "credentials"] {
        reader.vector();
    }
    let source = reader.u8();
    match source {
        FROM_KEY_PACKAGE => {
            reader.take(16); // lifetime: not_before, not_after
        }
        2 => {}
        3 => {
            reader.vector(); // parent_hash
        }
        other => panic!("leaf node source {other}"),
    }
    reader.vector(); // extensions
    let to_be_signed = reader.bytes[start..reader.at].to_vec();
    let signature = reader.vector().to_vec();
    LeafNode {
        encryption_key,
        signature_key,
        credential_type,
        identity,
        cipher_suites,
        source,
        to_be_signed,
        signature,
    }
}

/// Whether `suite` is a GREASE value (RFC 9420, §13.5), which a library may list among the
/// capabilities without supporting it.
pub fn is_grease(suite: u16) -> bool {
    let [high, low] = suite.to_be_bytes();
    high == low && low & 0x0f == 0x0a
}

/// Whether `signature` is the Ed25519 signature of `public_key` over `content` with `label`:
/// `VerifyWithLabel`.
pub fn verifies_with_label(
    public_key: &[u8],
    label: &str,
    content: &[u8],
    signature: &[u8],
) -> bool {
    let (Ok(public_key), Ok(signature)) = (
        <[u8; 32]>::try_from(public_key),
        <[u8; 64]>::try_from(signature),
    ) else {
        return false;
    };
    let Ok(key) = VerifyingKey::from_bytes(&public_key) else {
        return false;
    };
    // SignContent: the label and the content, each an opaque<V>.
    let mut sign_content = Vec::new();
    for field in [format!("MLS 1.0 {label}").as_bytes(), content] {
        sign_content.extend(varint(field.len()));
        sign_content.extend_from_slice(field);
    }
    key.verify_strict(&sign_content, &Signature::from_bytes(&signature))
        .is_ok()
}

/// The variable-size length of RFC 9420, §2.1.2.
pub fn varint(len: usize) -> Vec<u8> {
    let len = u32::try_from(len).expect("a length below 2^30");
    match len {
        0..0x40 => vec![len as u8],
        0x40..0x4000 => (0x4000 | len as u16).to_be_bytes().to_vec(),
        _ => (0x8000_0000 | len).to_be_bytes().to_vec(),
    }
}

/// Reads RFC 9420's encoding from `bytes`, at byte `at`; panics on a truncated field.
pub struct Reader<'a> {
    pub bytes: &'a [u8],
    pub at: usize,
}

impl<'a> Reader<'a> {
    pub fn take(&mut self, len: usize) -> &'a [u8] {
        let end = self.at + len;
        assert!(end <= self.bytes.len(), "truncated at byte {}", self.at);
        let taken = &self.bytes[self.at..end];
        self.at = end;
        taken
    }

    pub fn u8(&mut self) -> u8 {
        self.take(1)[0]
    }

    pub fn u16(&mut self) -> u16 {
        u16::from_be_bytes(self.take(2).try_into().unwrap())
    }

    /// An `opaque<V>` or a vector: its length, then its bytes.
    pub fn vector(&mut self) -> &'a [u8] {
        let first = self.u8();
        let len = match first >> 6 {
            0 => usize::from(first),
            1 => usize::from(first & 0x3f) << 8 | usize::from(self.u8()),
            2 => self
                .take(3)
                .iter()
                .fold(usize::from(first & 0x3f), |len, &byte| {
                    len << 8 | usize::from(byte)
                }),
            _ => panic!("a length prefix of 8 bytes at byte {}", self.at - 1),
        };
        self.take(len)
    }
}
