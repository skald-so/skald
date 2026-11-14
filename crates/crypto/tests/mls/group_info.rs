//! Reads a GroupInfo in an `MLSMessage` from its bytes as RFC 9420, §6 and §12.4.3, encode it,
//! apart from the crate under test, and encodes it again, changed and signed by any key, as a
//! hostile member or server could.

use ed25519_dalek::Signer as _;

use crate::wire::{MLS10, Reader, varint};

/// The `mls_group_info` wire format.
const GROUP_INFO_MESSAGE: u16 = 4;
/// The `ratchet_tree` GroupInfo extension type.
pub const RATCHET_TREE: u16 = 2;
/// The `external_pub` GroupInfo extension type.
pub const EXTERNAL_PUB: u16 = 4;

/// The fields of a GroupInfo the tests check or change.
#[derive(Debug, Clone)]
pub struct GroupInfo {
    /// The encoded `GroupContext`.
    pub context: Vec<u8>,
    pub group_id: Vec<u8>,
    pub epoch: u64,
    /// Each extension's type and `extension_data`, in order.
    pub extensions: Vec<(u16, Vec<u8>)>,
    pub confirmation_tag: Vec<u8>,
    /// The signer's leaf index.
    pub signer: u32,
    pub signature: Vec<u8>,
}

/// A GroupInfo in an `MLSMessage`.
///
/// # Panics
/// If `bytes` is not one, or has bytes after it.
pub fn group_info(bytes: &[u8]) -> GroupInfo {
    let mut reader = Reader { bytes, at: 0 };
    assert_eq!(reader.u16(), MLS10, "the protocol version");
    assert_eq!(reader.u16(), GROUP_INFO_MESSAGE, "a GroupInfo");
    let start = reader.at;
    assert_eq!(reader.u16(), MLS10, "the group context's protocol version");
    reader.u16(); // cipher_suite
    let group_id = reader.vector().to_vec();
    let epoch = u64::from_be_bytes(reader.take(8).try_into().unwrap());
    for _ in ["tree_hash", "confirmed_transcript_hash", "extensions"] {
        reader.vector();
    }
    let context = bytes[start..reader.at].to_vec();
    let mut list = Reader {
        bytes: reader.vector(),
        at: 0,
    };
    let mut extensions = Vec::new();
    while list.at < list.bytes.len() {
        let extension_type = list.u16();
        extensions.push((extension_type, list.vector().to_vec()));
    }
    let confirmation_tag = reader.vector().to_vec();
    let signer = u32::from_be_bytes(reader.take(4).try_into().unwrap());
    let signature = reader.vector().to_vec();
    assert_eq!(reader.at, bytes.len(), "bytes after the GroupInfo");
    GroupInfo {
        context,
        group_id,
        epoch,
        extensions,
        confirmation_tag,
        signer,
        signature,
    }
}

/// `value` as an `opaque<V>`.
fn opaque(value: &[u8]) -> Vec<u8> {
    [varint(value.len()), value.to_vec()].concat()
}

impl GroupInfo {
    /// Whether it carries an extension of `extension_type`.
    pub fn has(&self, extension_type: u16) -> bool {
        self.extensions
            .iter()
            .any(|(kind, _)| *kind == extension_type)
    }

    /// The `extension_data` of its extension of `extension_type`.
    ///
    /// # Panics
    /// If it carries none.
    pub fn extension(&mut self, extension_type: u16) -> &mut Vec<u8> {
        self.extensions
            .iter_mut()
            .find_map(|(kind, data)| (*kind == extension_type).then_some(data))
            .unwrap_or_else(|| panic!("an extension of type {extension_type}"))
    }

    /// `GroupInfoTBS`: the encoded GroupInfo up to its signature.
    fn to_be_signed(&self) -> Vec<u8> {
        let extensions: Vec<u8> = (self.extensions.iter())
            .flat_map(|(kind, data)| [kind.to_be_bytes().to_vec(), opaque(data)].concat())
            .collect();
        [
            self.context.clone(),
            opaque(&extensions),
            opaque(&self.confirmation_tag),
            self.signer.to_be_bytes().to_vec(),
        ]
        .concat()
    }

    /// Signs it again with key `seed`, as `SignWithLabel(_, "GroupInfoTBS", GroupInfoTBS)`.
    pub fn sign(&mut self, seed: u8) {
        let content = [
            opaque(b"MLS 1.0 GroupInfoTBS"),
            opaque(&self.to_be_signed()),
        ]
        .concat();
        let key = ed25519_dalek::SigningKey::from_bytes(&[seed; 32]);
        self.signature = key.sign(&content).to_bytes().to_vec();
    }

    /// The `MLSMessage` with it.
    pub fn encode(&self) -> Vec<u8> {
        [
            MLS10.to_be_bytes().to_vec(),
            GROUP_INFO_MESSAGE.to_be_bytes().to_vec(),
            self.to_be_signed(),
            opaque(&self.signature),
        ]
        .concat()
    }
}

/// `group_info` changed by `change` and signed again with key `seed`.
pub fn resigned(group_info: &[u8], seed: u8, change: impl FnOnce(&mut GroupInfo)) -> Vec<u8> {
    let mut fields = self::group_info(group_info);
    change(&mut fields);
    fields.sign(seed);
    fields.encode()
}

/// Replaces the one occurrence of `from` in `bytes` with `to`, of the same length.
///
/// # Panics
/// If `from` occurs other than once.
pub fn replace_once(bytes: &mut [u8], from: &[u8], to: &[u8]) {
    assert_eq!(from.len(), to.len(), "the same length");
    let found: Vec<usize> = (0..=bytes.len() - from.len())
        .filter(|&at| &bytes[at..at + from.len()] == from)
        .collect();
    let [at] = found[..] else {
        panic!("{} occurrences, not one", found.len());
    };
    bytes[at..at + to.len()].copy_from_slice(to);
}
