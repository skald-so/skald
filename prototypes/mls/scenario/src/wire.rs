//! Reads the fields the scenarios check from encoded `MLSMessage`s, without either library
//! [RFC 9420, §6 and §10–12].

use crate::Result;

/// `mls10`.
pub const PROTOCOL_VERSION: u16 = 1;
/// `MLS_128_DHKEMX25519_AES128GCM_SHA256_Ed25519`.
pub const CIPHER_SUITE: u16 = 0x0001;
/// The `basic` credential type.
pub const BASIC: u16 = 1;

/// `WireFormat` values.
pub const PUBLIC_MESSAGE: u16 = 1;
pub const PRIVATE_MESSAGE: u16 = 2;
pub const WELCOME: u16 = 3;
pub const GROUP_INFO: u16 = 4;
pub const KEY_PACKAGE: u16 = 5;

/// `ContentType` values.
pub const APPLICATION: u8 = 1;
pub const COMMIT: u8 = 3;

/// `ProposalType` values.
pub const ADD: u16 = 1;
pub const REMOVE: u16 = 3;
pub const EXTERNAL_INIT: u16 = 6;

/// `SenderType` values.
const MEMBER: u8 = 1;
const NEW_MEMBER_COMMIT: u8 = 4;

/// The `last_resort` KeyPackage extension type [draft-ietf-mls-extensions, §5].
pub const LAST_RESORT: u16 = 0x000a;
/// The `ratchet_tree` and `external_pub` GroupInfo extension types [RFC 9420, §12.4.3].
pub const RATCHET_TREE: u16 = 0x0002;
pub const EXTERNAL_PUB: u16 = 0x0004;
/// The `key_package` `LeafNodeSource`.
const KEY_PACKAGE_SOURCE: u8 = 1;

/// A KeyPackage's `lifetime`, in seconds since the Unix epoch.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Lifetime {
    pub not_before: u64,
    pub not_after: u64,
}

impl Lifetime {
    /// Whether `now` is within it.
    pub fn holds(&self, now: u64) -> bool {
        (self.not_before..=self.not_after).contains(&now)
    }
}

/// An extension: its type and its `extension_data`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Extension {
    pub extension_type: u16,
    pub data: Vec<u8>,
}

/// A KeyPackage [RFC 9420, §10] with a leaf from a KeyPackage [§7.2], field by field. The fields
/// the scenarios do not read keep their encoding.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct KeyPackage {
    pub cipher_suite: u16,
    pub init_key: Vec<u8>,
    pub encryption_key: Vec<u8>,
    pub signature_key: Vec<u8>,
    pub credential_type: u16,
    /// A `basic` credential's `identity`; for another type, the content of its one vector.
    pub identity: Vec<u8>,
    /// The encoded `Capabilities`.
    pub capabilities: Vec<u8>,
    pub lifetime: Lifetime,
    /// The content of the leaf's `extensions` vector.
    pub leaf_extensions: Vec<u8>,
    pub leaf_signature: Vec<u8>,
    pub extensions: Vec<Extension>,
    pub signature: Vec<u8>,
}

impl KeyPackage {
    /// Whether it carries the `last_resort` extension.
    pub fn last_resort(&self) -> bool {
        self.extensions
            .iter()
            .any(|extension| extension.extension_type == LAST_RESORT)
    }

    /// The encoded `MLSMessage` with the KeyPackage.
    pub fn encode(&self) -> Vec<u8> {
        let mut out = header_bytes(KEY_PACKAGE);
        out.extend_from_slice(&self.tbs());
        put_vector(&mut out, &self.signature);
        out
    }

    /// Signs the leaf and then the KeyPackage with `signer`'s key, as `SignWithLabel` with
    /// `LeafNodeTBS` and `KeyPackageTBS` [RFC 9420, §5.1.2].
    pub fn sign(&mut self, signer: &crate::Identity) {
        self.leaf_signature = signer
            .sign(&sign_content("LeafNodeTBS", &self.leaf_tbs()))
            .to_vec();
        self.signature = signer
            .sign(&sign_content("KeyPackageTBS", &self.tbs()))
            .to_vec();
    }

    /// The `LeafNodeTBS` of a leaf from a KeyPackage.
    fn leaf_tbs(&self) -> Vec<u8> {
        let mut out = Vec::new();
        put_vector(&mut out, &self.encryption_key);
        put_vector(&mut out, &self.signature_key);
        out.extend_from_slice(&self.credential_type.to_be_bytes());
        put_vector(&mut out, &self.identity);
        out.extend_from_slice(&self.capabilities);
        out.push(KEY_PACKAGE_SOURCE);
        out.extend_from_slice(&self.lifetime.not_before.to_be_bytes());
        out.extend_from_slice(&self.lifetime.not_after.to_be_bytes());
        put_vector(&mut out, &self.leaf_extensions);
        out
    }

    /// The `KeyPackageTBS`, with the signed leaf.
    fn tbs(&self) -> Vec<u8> {
        let mut out = PROTOCOL_VERSION.to_be_bytes().to_vec();
        out.extend_from_slice(&self.cipher_suite.to_be_bytes());
        put_vector(&mut out, &self.init_key);
        out.extend_from_slice(&self.leaf_tbs());
        put_vector(&mut out, &self.leaf_signature);
        put_vector(&mut out, &encode_extensions(&self.extensions));
        out
    }
}

/// A KeyPackage message.
pub fn key_package(message: &[u8]) -> Result<KeyPackage> {
    let mut input = Input(message);
    expect_wire_format(&mut input, KEY_PACKAGE)?;
    let key_package = read_key_package(&mut input)?;
    if !input.0.is_empty() {
        return Err("KeyPackage: trailing bytes".into());
    }
    Ok(key_package)
}

/// The KeyPackage at the start of `input`.
fn read_key_package(input: &mut Input) -> Result<KeyPackage> {
    if input.u16()? != PROTOCOL_VERSION {
        return Err("KeyPackage: not mls10".into());
    }
    let cipher_suite = input.u16()?;
    let init_key = input.vector()?.to_vec();
    let encryption_key = input.vector()?.to_vec();
    let signature_key = input.vector()?.to_vec();
    let credential_type = input.u16()?;
    let identity = input.vector()?.to_vec();
    let rest = input.0;
    for _ in 0..5 {
        input.vector()?; // versions, cipher_suites, extensions, proposals, credentials
    }
    let capabilities = rest[..rest.len() - input.0.len()].to_vec();
    if input.u8()? != KEY_PACKAGE_SOURCE {
        return Err("KeyPackage: a leaf that is not from a KeyPackage".into());
    }
    let lifetime = Lifetime {
        not_before: input.u64()?,
        not_after: input.u64()?,
    };
    let leaf_extensions = input.vector()?.to_vec();
    let leaf_signature = input.vector()?.to_vec();
    let extensions = extensions(input.vector()?)?;
    let signature = input.vector()?.to_vec();
    Ok(KeyPackage {
        cipher_suite,
        init_key,
        encryption_key,
        signature_key,
        credential_type,
        identity,
        capabilities,
        lifetime,
        leaf_extensions,
        leaf_signature,
        extensions,
        signature,
    })
}

/// The content of an `extensions` vector.
fn extensions(list: &[u8]) -> Result<Vec<Extension>> {
    let mut list = Input(list);
    let mut extensions = Vec::new();
    while !list.0.is_empty() {
        extensions.push(Extension {
            extension_type: list.u16()?,
            data: list.vector()?.to_vec(),
        });
    }
    Ok(extensions)
}

/// The content of an `extensions` vector with `extensions`.
fn encode_extensions(extensions: &[Extension]) -> Vec<u8> {
    let mut out = Vec::new();
    for extension in extensions {
        out.extend_from_slice(&extension.extension_type.to_be_bytes());
        put_vector(&mut out, &extension.data);
    }
    out
}

/// A GroupInfo [RFC 9420, §12.4.3], field by field. The fields the scenarios do not read keep
/// their encoding.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GroupInfo {
    pub cipher_suite: u16,
    pub group_id: Vec<u8>,
    pub epoch: u64,
    pub tree_hash: Vec<u8>,
    pub confirmed_transcript_hash: Vec<u8>,
    /// The content of the group context's `extensions` vector.
    pub context_extensions: Vec<u8>,
    pub extensions: Vec<Extension>,
    pub confirmation_tag: Vec<u8>,
    /// The signer's leaf index.
    pub signer: u32,
    pub signature: Vec<u8>,
}

impl GroupInfo {
    /// Whether it carries an extension of `extension_type`.
    pub fn has(&self, extension_type: u16) -> bool {
        self.extensions
            .iter()
            .any(|extension| extension.extension_type == extension_type)
    }

    /// The encoded `MLSMessage` with the GroupInfo.
    pub fn encode(&self) -> Vec<u8> {
        let mut out = header_bytes(GROUP_INFO);
        out.extend_from_slice(&self.tbs());
        put_vector(&mut out, &self.signature);
        out
    }

    /// Signs it with `signer`'s key, as `SignWithLabel` with `GroupInfoTBS` [RFC 9420, §12.4.3].
    pub fn sign(&mut self, signer: &crate::Identity) {
        self.signature = signer
            .sign(&sign_content("GroupInfoTBS", &self.tbs()))
            .to_vec();
    }

    /// The `GroupInfoTBS`.
    fn tbs(&self) -> Vec<u8> {
        let mut out = PROTOCOL_VERSION.to_be_bytes().to_vec();
        out.extend_from_slice(&self.cipher_suite.to_be_bytes());
        put_vector(&mut out, &self.group_id);
        out.extend_from_slice(&self.epoch.to_be_bytes());
        put_vector(&mut out, &self.tree_hash);
        put_vector(&mut out, &self.confirmed_transcript_hash);
        put_vector(&mut out, &self.context_extensions);
        put_vector(&mut out, &encode_extensions(&self.extensions));
        put_vector(&mut out, &self.confirmation_tag);
        out.extend_from_slice(&self.signer.to_be_bytes());
        out
    }
}

/// A GroupInfo message.
pub fn group_info(message: &[u8]) -> Result<GroupInfo> {
    let mut input = Input(message);
    expect_wire_format(&mut input, GROUP_INFO)?;
    if input.u16()? != PROTOCOL_VERSION {
        return Err("GroupInfo: not mls10".into());
    }
    let group_info = GroupInfo {
        cipher_suite: input.u16()?,
        group_id: input.vector()?.to_vec(),
        epoch: input.u64()?,
        tree_hash: input.vector()?.to_vec(),
        confirmed_transcript_hash: input.vector()?.to_vec(),
        context_extensions: input.vector()?.to_vec(),
        extensions: extensions(input.vector()?)?,
        confirmation_tag: input.vector()?.to_vec(),
        signer: input.u32()?,
        signature: input.vector()?.to_vec(),
    };
    if !input.0.is_empty() {
        return Err("GroupInfo: trailing bytes".into());
    }
    Ok(group_info)
}

/// The `SignContent` of `SignWithLabel(_, label, content)`.
fn sign_content(label: &str, content: &[u8]) -> Vec<u8> {
    let mut out = Vec::new();
    put_vector(&mut out, format!("MLS 1.0 {label}").as_bytes());
    put_vector(&mut out, content);
    out
}

/// The `MLSMessage` header with `wire_format`.
fn header_bytes(wire_format: u16) -> Vec<u8> {
    [PROTOCOL_VERSION.to_be_bytes(), wire_format.to_be_bytes()].concat()
}

/// Appends a vector with the shortest variable-length size [RFC 9420, §2.1.2].
fn put_vector(out: &mut Vec<u8>, value: &[u8]) {
    let len = value.len();
    match len {
        0..0x40 => out.push(len as u8),
        // The top two bits give the size; the length is below them.
        0x40..0x4000 => out.extend_from_slice(&(0x4000 + len as u16).to_be_bytes()),
        _ => out.extend_from_slice(&(0x8000_0000 + len as u32).to_be_bytes()),
    }
    out.extend_from_slice(value);
}

/// The plaintext header of a PublicMessage or a PrivateMessage.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Framing {
    pub wire_format: u16,
    pub group_id: Vec<u8>,
    pub epoch: u64,
    pub content_type: u8,
}

/// The sender of a Commit.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CommitSender {
    /// A member, by its leaf index.
    Member(u32),
    /// A new member: an external Commit.
    NewMember,
}

/// A Commit in a PublicMessage: its sender, `authenticated_data`, proposals and path.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PublicCommit {
    pub sender: CommitSender,
    pub authenticated_data: Vec<u8>,
    /// The leaf indexes of its Remove proposals, by value.
    pub removed: Vec<u32>,
    /// Whether it has an ExternalInit proposal by value, as an external Commit does.
    pub external_init: bool,
    /// The number of its other proposals: Adds by value and any by reference.
    pub other_proposals: usize,
    /// Whether it has an UpdatePath.
    pub path: bool,
}

/// The `cipher_suite` of a Welcome message.
pub fn welcome_cipher_suite(message: &[u8]) -> Result<u16> {
    let mut input = Input(message);
    expect_wire_format(&mut input, WELCOME)?;
    input.u16()
}

/// The `new_member` of each of a Welcome message's `secrets`: the KeyPackageRef of each device it
/// is for, in order.
pub fn welcome_recipients(message: &[u8]) -> Result<Vec<Vec<u8>>> {
    let mut input = Input(message);
    expect_wire_format(&mut input, WELCOME)?;
    input.u16()?; // cipher_suite
    let mut secrets = Input(input.vector()?);
    let mut recipients = Vec::new();
    while !secrets.0.is_empty() {
        recipients.push(secrets.vector()?.to_vec());
        secrets.vector()?; // kem_output
        secrets.vector()?; // ciphertext
    }
    Ok(recipients)
}

/// The KeyPackageRef of a KeyPackage message with ciphersuite `0x0001`:
/// `RefHash("MLS 1.0 KeyPackage Reference", KeyPackage)` with SHA-256 [RFC 9420, §5.2].
pub fn key_package_ref(message: &[u8]) -> Result<Vec<u8>> {
    use sha2::{Digest, Sha256};

    let mut input = Input(message);
    expect_wire_format(&mut input, KEY_PACKAGE)?;
    let mut hashed = Vec::new();
    put_vector(&mut hashed, b"MLS 1.0 KeyPackage Reference");
    put_vector(&mut hashed, input.0);
    Ok(Sha256::digest(&hashed).to_vec())
}

/// The plaintext header of a PublicMessage or a PrivateMessage.
pub fn framing(message: &[u8]) -> Result<Framing> {
    let mut input = Input(message);
    let wire_format = header(&mut input)?;
    let group_id = input.vector()?.to_vec();
    let epoch = input.u64()?;
    let content_type = match wire_format {
        PUBLIC_MESSAGE => {
            // Sender: member and external senders have an index, new members none.
            match input.u8()? {
                1 | 2 => {
                    input.u32()?;
                }
                3 | 4 => {}
                other => return Err(format!("PublicMessage: sender type {other}").into()),
            }
            input.vector()?; // authenticated_data
            input.u8()?
        }
        PRIVATE_MESSAGE => input.u8()?,
        other => return Err(format!("not a PublicMessage or PrivateMessage: {other}").into()),
    };
    Ok(Framing {
        wire_format,
        group_id,
        epoch,
        content_type,
    })
}

/// The Commit of a PublicMessage from a member or a new member.
pub fn public_commit(message: &[u8]) -> Result<PublicCommit> {
    let mut input = Input(message);
    expect_wire_format(&mut input, PUBLIC_MESSAGE)?;
    input.vector()?; // group_id
    input.u64()?; // epoch
    let sender = match input.u8()? {
        MEMBER => CommitSender::Member(input.u32()?),
        NEW_MEMBER_COMMIT => CommitSender::NewMember,
        other => return Err(format!("PublicMessage: a Commit from sender type {other}").into()),
    };
    let authenticated_data = input.vector()?.to_vec();
    if input.u8()? != COMMIT {
        return Err("PublicMessage: not a Commit".into());
    }
    let mut proposals = Input(input.vector()?);
    let (mut removed, mut external_init, mut other_proposals) = (Vec::new(), false, 0);
    while !proposals.0.is_empty() {
        match proposals.u8()? {
            1 => match proposals.u16()? {
                ADD => {
                    read_key_package(&mut proposals)?;
                    other_proposals += 1;
                }
                REMOVE => removed.push(proposals.u32()?),
                EXTERNAL_INIT if !external_init => {
                    proposals.vector()?; // kem_output
                    external_init = true;
                }
                other => return Err(format!("Commit: proposal type {other} by value").into()),
            },
            2 => {
                proposals.vector()?;
                other_proposals += 1;
            }
            other => return Err(format!("Commit: ProposalOrRef type {other}").into()),
        }
    }
    let path = match input.u8()? {
        0 => false,
        1 => true,
        other => return Err(format!("Commit: optional path {other}").into()),
    };
    Ok(PublicCommit {
        sender,
        authenticated_data,
        removed,
        external_init,
        other_proposals,
        path,
    })
}

fn header(input: &mut Input) -> Result<u16> {
    if input.u16()? != PROTOCOL_VERSION {
        return Err("MLSMessage: not mls10".into());
    }
    input.u16()
}

fn expect_wire_format(input: &mut Input, want: u16) -> Result<()> {
    let got = header(input)?;
    if got == want {
        Ok(())
    } else {
        Err(format!("MLSMessage: wire format {got}, want {want}").into())
    }
}

struct Input<'a>(&'a [u8]);

impl<'a> Input<'a> {
    fn take(&mut self, len: usize) -> Result<&'a [u8]> {
        if len > self.0.len() {
            return Err("MLSMessage: truncated".into());
        }
        let (value, rest) = self.0.split_at(len);
        self.0 = rest;
        Ok(value)
    }

    fn u8(&mut self) -> Result<u8> {
        Ok(self.take(1)?[0])
    }

    fn u16(&mut self) -> Result<u16> {
        Ok(u16::from_be_bytes(self.take(2)?.try_into()?))
    }

    fn u32(&mut self) -> Result<u32> {
        Ok(u32::from_be_bytes(self.take(4)?.try_into()?))
    }

    fn u64(&mut self) -> Result<u64> {
        Ok(u64::from_be_bytes(self.take(8)?.try_into()?))
    }

    /// A vector with a variable-length size [RFC 9420, §2.1.2].
    fn vector(&mut self) -> Result<&'a [u8]> {
        let first = self.u8()?;
        let len = match first >> 6 {
            0 => usize::from(first),
            1 => usize::from(first & 0x3f) << 8 | usize::from(self.u8()?),
            2 => {
                let rest = self.take(3)?;
                usize::from(first & 0x3f) << 24
                    | usize::from(rest[0]) << 16
                    | usize::from(rest[1]) << 8
                    | usize::from(rest[2])
            }
            _ => return Err("MLSMessage: invalid vector length".into()),
        };
        self.take(len)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_vector_lengths_of_each_size() {
        assert_eq!(Input(b"\x02ab").vector().unwrap(), b"ab");
        let long = [vec![0x41, 0x00], vec![9; 256]].concat();
        assert_eq!(Input(&long).vector().unwrap(), &[9; 256][..]);
        let longer = [vec![0x80, 0x01, 0x00, 0x00], vec![8; 65536]].concat();
        assert_eq!(Input(&longer).vector().unwrap().len(), 65536);
        // Each byte of a 4-byte length counts: 0x01010101.
        let len = 0x0101_0101;
        let longest = [vec![0x81, 0x01, 0x01, 0x01], vec![7; len]].concat();
        assert_eq!(Input(&longest).vector().unwrap().len(), len);
        assert!(Input(b"\xc0").vector().is_err());
        assert!(Input(b"\x03ab").vector().is_err());
    }

    /// A KeyPackage message with a last-resort extension and its fields.
    fn a_key_package() -> (Vec<u8>, KeyPackage) {
        let mut message = vec![0, 1, 0, 5, 0, 1, 0, 1];
        message.extend_from_slice(b"\x01i\x01e\x02sk\x00\x01\x03dev");
        let capabilities = b"\x02\x00\x01\x02\x00\x01\x02\x00\x0a\x00\x02\x00\x01";
        message.extend_from_slice(capabilities);
        message.push(1);
        message.extend_from_slice(&1u64.to_be_bytes());
        message.extend_from_slice(&2u64.to_be_bytes());
        message.extend_from_slice(b"\x00\x02ls\x03\x00\x0a\x00\x02sg");
        let fields = KeyPackage {
            cipher_suite: CIPHER_SUITE,
            init_key: b"i".to_vec(),
            encryption_key: b"e".to_vec(),
            signature_key: b"sk".to_vec(),
            credential_type: BASIC,
            identity: b"dev".to_vec(),
            capabilities: capabilities.to_vec(),
            lifetime: Lifetime {
                not_before: 1,
                not_after: 2,
            },
            leaf_extensions: Vec::new(),
            leaf_signature: b"ls".to_vec(),
            extensions: vec![Extension {
                extension_type: LAST_RESORT,
                data: Vec::new(),
            }],
            signature: b"sg".to_vec(),
        };
        (message, fields)
    }

    #[test]
    fn reads_and_encodes_a_key_package() {
        let (message, fields) = a_key_package();
        assert_eq!(key_package(&message).unwrap(), fields);
        assert_eq!(fields.encode(), message);
        assert!(fields.last_resort());

        let mut other = fields.clone();
        other.extensions = vec![
            Extension {
                extension_type: 0xff00,
                data: b"x".to_vec(),
            },
            Extension {
                extension_type: 0x0009,
                data: Vec::new(),
            },
        ];
        other.leaf_extensions = b"\x00\x0b\x00".to_vec();
        assert!(!other.last_resort());
        assert_eq!(key_package(&other.encode()).unwrap(), other);
    }

    #[test]
    fn rejects_what_is_not_a_key_package() {
        let (message, _) = a_key_package();
        let changed = |at: usize, byte: u8| {
            let mut message = message.clone();
            message[at] = byte;
            message
        };
        // The leaf's source follows the 13 bytes of capabilities.
        let source = message.len() - 28;
        assert_eq!(message[source], 1);
        for (message, why) in [
            (changed(3, 3), "a Welcome"),
            (changed(5, 2), "not mls10"),
            (changed(source, 2), "a leaf from an Update"),
            ([message.clone(), vec![0]].concat(), "trailing bytes"),
            (message[..message.len() - 1].to_vec(), "truncated"),
            (changed(message.len() - 7, 0x04), "a truncated extension"),
        ] {
            assert!(key_package(&message).is_err(), "{why}");
        }
    }

    #[test]
    fn signs_the_leaf_and_then_the_key_package() {
        let (_, mut fields) = a_key_package();
        let signer = crate::Identity::generate();
        fields.sign(&signer);

        // The leaf's TBS is the message from its encryption key up to its signature.
        let (message, _) = a_key_package();
        let leaf_tbs = &message[10..message.len() - 10];
        let mut leaf_content = b"\x13MLS 1.0 LeafNodeTBS".to_vec();
        leaf_content.push(leaf_tbs.len() as u8);
        leaf_content.extend_from_slice(leaf_tbs);
        assert_eq!(fields.leaf_signature, signer.sign(&leaf_content));

        let encoded = fields.encode();
        let tbs = &encoded[4..encoded.len() - 66];
        let mut content = b"\x15MLS 1.0 KeyPackageTBS".to_vec();
        content.push(0x40 | (tbs.len() >> 8) as u8);
        content.push(tbs.len() as u8);
        content.extend_from_slice(tbs);
        assert_eq!(fields.signature, signer.sign(&content));
        assert_eq!(
            &encoded[encoded.len() - 66..encoded.len() - 64],
            [0x40, 0x40]
        );
        assert_eq!(key_package(&encoded).unwrap(), fields);
    }

    #[test]
    fn writes_vectors_with_the_shortest_length() {
        for (len, prefix) in [
            (0, &[0x00][..]),
            (0x3f, &[0x3f]),
            (0x40, &[0x40, 0x40]),
            (0x3fff, &[0x7f, 0xff]),
            (0x4000, &[0x80, 0x00, 0x40, 0x00]),
            (0x1_0203, &[0x80, 0x01, 0x02, 0x03]),
        ] {
            let value = vec![7; len];
            let mut out = Vec::new();
            put_vector(&mut out, &value);
            assert_eq!(&out[..prefix.len()], prefix, "{len}");
            assert_eq!(Input(&out).vector().unwrap(), value, "{len}");
            assert_eq!(out.len(), prefix.len() + len, "{len}");
        }
    }

    #[test]
    fn a_lifetime_holds_between_its_bounds() {
        let lifetime = Lifetime {
            not_before: 10,
            not_after: 20,
        };
        for (now, want) in [(9, false), (10, true), (20, true), (21, false)] {
            assert_eq!(lifetime.holds(now), want, "{now}");
        }
    }

    #[test]
    fn reads_a_welcomes_cipher_suite() {
        assert_eq!(welcome_cipher_suite(&[0, 1, 0, 3, 0, 3]).unwrap(), 3);
        assert!(welcome_cipher_suite(&[0, 1, 0, 5, 0, 3]).is_err());
    }

    #[test]
    fn reads_a_welcomes_recipients() {
        let welcome = b"\x00\x01\x00\x03\x00\x01\x0d\x02r1\x01k\x02ct\x02r2\x00\x00\x03gi!";
        assert_eq!(
            welcome_recipients(welcome).unwrap(),
            [b"r1".to_vec(), b"r2".to_vec()]
        );
        let no_secrets = b"\x00\x01\x00\x03\x00\x01\x00\x00";
        assert!(welcome_recipients(no_secrets).unwrap().is_empty());
        let mut other = welcome.to_vec();
        other[3] = 5;
        assert!(welcome_recipients(&other).is_err(), "a KeyPackage");
        let truncated = b"\x00\x01\x00\x03\x00\x01\x04\x02r1\x01k";
        assert!(welcome_recipients(truncated).is_err(), "a truncated secret");
    }

    #[test]
    fn hashes_a_key_package_to_its_ref() {
        use sha2::{Digest, Sha256};

        let (message, _) = a_key_package();
        let mut hashed = b"\x1cMLS 1.0 KeyPackage Reference".to_vec();
        hashed.push((message.len() - 4) as u8);
        hashed.extend_from_slice(&message[4..]);
        assert_eq!(
            key_package_ref(&message).unwrap(),
            Sha256::digest(&hashed).to_vec()
        );
        let mut other = message.clone();
        other[3] = 3;
        assert!(key_package_ref(&other).is_err(), "a Welcome");
    }

    /// A GroupInfo message of group `g1` at epoch 7 with `extensions`, signed by leaf 3.
    fn a_group_info(extensions: &[u8]) -> Vec<u8> {
        let mut message = vec![0, 1, 0, 4, 0, 1, 0, 1, 2, b'g', b'1'];
        message.extend_from_slice(&7u64.to_be_bytes());
        message.extend_from_slice(b"\x02th\x03cth\x03\x00\x01\x00");
        message.push(extensions.len() as u8);
        message.extend_from_slice(extensions);
        message.extend_from_slice(b"\x01t\x00\x00\x00\x03\x02sg");
        message
    }

    #[test]
    fn reads_and_encodes_a_group_info() {
        let message = a_group_info(b"\x00\x02\x02rt\x00\x04\x01x");
        let fields = group_info(&message).unwrap();
        assert_eq!(
            fields,
            GroupInfo {
                cipher_suite: CIPHER_SUITE,
                group_id: b"g1".to_vec(),
                epoch: 7,
                tree_hash: b"th".to_vec(),
                confirmed_transcript_hash: b"cth".to_vec(),
                context_extensions: b"\x00\x01\x00".to_vec(),
                extensions: vec![
                    Extension {
                        extension_type: RATCHET_TREE,
                        data: b"rt".to_vec(),
                    },
                    Extension {
                        extension_type: EXTERNAL_PUB,
                        data: b"x".to_vec(),
                    },
                ],
                confirmation_tag: b"t".to_vec(),
                signer: 3,
                signature: b"sg".to_vec(),
            }
        );
        assert_eq!(fields.encode(), message);
        assert!(fields.has(RATCHET_TREE) && fields.has(EXTERNAL_PUB));
        assert!(!fields.has(LAST_RESORT));
        let bare = group_info(&a_group_info(b"")).unwrap();
        assert_eq!(bare.extensions, []);
        assert!(!bare.has(RATCHET_TREE));
    }

    #[test]
    fn signs_a_group_info() {
        let mut fields = group_info(&a_group_info(b"")).unwrap();
        let signer = crate::Identity::generate();
        fields.sign(&signer);

        // The TBS is the message from its version up to its signature.
        let message = a_group_info(b"");
        let tbs = &message[4..message.len() - 3];
        let mut content = b"\x14MLS 1.0 GroupInfoTBS".to_vec();
        content.push(tbs.len() as u8);
        content.extend_from_slice(tbs);
        assert_eq!(fields.signature, signer.sign(&content));
        assert_eq!(group_info(&fields.encode()).unwrap(), fields);
    }

    #[test]
    fn rejects_what_is_not_a_group_info() {
        let message = a_group_info(b"");
        let changed = |at: usize, byte: u8| {
            let mut message = message.clone();
            message[at] = byte;
            message
        };
        for (message, why) in [
            (changed(3, 3), "a Welcome"),
            (changed(5, 2), "not mls10"),
            ([message.clone(), vec![0]].concat(), "trailing bytes"),
            (message[..message.len() - 1].to_vec(), "truncated"),
        ] {
            assert!(group_info(&message).is_err(), "{why}");
        }
    }

    #[test]
    fn reads_framing_of_public_and_private_messages() {
        let mut public = vec![0, 1, 0, 1, 2, b'g', b'1'];
        public.extend_from_slice(&7u64.to_be_bytes());
        public.extend_from_slice(&[1, 0, 0, 0, 4, 0, COMMIT]);
        assert_eq!(
            framing(&public).unwrap(),
            Framing {
                wire_format: PUBLIC_MESSAGE,
                group_id: b"g1".to_vec(),
                epoch: 7,
                content_type: COMMIT,
            }
        );

        let mut private = vec![0, 1, 0, 2, 1, b'g'];
        private.extend_from_slice(&2u64.to_be_bytes());
        private.push(APPLICATION);
        assert_eq!(framing(&private).unwrap().content_type, APPLICATION);

        assert!(framing(&[0, 1, 0, 3]).is_err());
        assert!(framing(&[0, 2, 0, 1]).is_err());
    }

    /// A member's PublicMessage of epoch 1 with `content`, from leaf 2 with `ad`.
    fn public_message(ad: &[u8], content: &[u8]) -> Vec<u8> {
        let mut message = vec![0, 1, 0, 1, 1, b'g'];
        message.extend_from_slice(&1u64.to_be_bytes());
        message.extend_from_slice(&[1, 0, 0, 0, 2]);
        message.push(ad.len() as u8);
        message.extend_from_slice(ad);
        message.extend_from_slice(content);
        message
    }

    #[test]
    fn reads_a_public_commit() {
        // A Remove of leaf 0x01020304 by value, a proposal by reference, and a path.
        let content = [COMMIT, 11, 1, 0, 3, 1, 2, 3, 4, 2, 2, 0xaa, 0xbb, 1, 0xff];
        assert_eq!(
            public_commit(&public_message(b"ad", &content)).unwrap(),
            PublicCommit {
                sender: CommitSender::Member(2),
                authenticated_data: b"ad".to_vec(),
                removed: vec![0x0102_0304],
                external_init: false,
                other_proposals: 1,
                path: true,
            }
        );

        let empty = public_commit(&public_message(b"", &[COMMIT, 0, 1])).unwrap();
        assert_eq!((empty.removed, empty.other_proposals), (vec![], 0));
        assert!(empty.path && !empty.external_init);
        assert!(
            !public_commit(&public_message(b"", &[COMMIT, 0, 0]))
                .unwrap()
                .path
        );
    }

    #[test]
    fn reads_a_commit_of_adds_by_value() {
        let (key_package, _) = a_key_package();
        let add = [&[1, 0, 1][..], &key_package[4..]].concat();
        let mut content = vec![COMMIT];
        put_vector(&mut content, &[add.clone(), add].concat());
        content.push(0);
        let commit = public_commit(&public_message(b"", &content)).unwrap();
        assert_eq!(
            (commit.removed, commit.other_proposals, commit.path),
            (vec![], 2, false)
        );

        // The second Add's KeyPackage lacks its last byte.
        let last = content.len() - 2;
        content.remove(last);
        content[2] -= 1;
        assert!(public_commit(&public_message(b"", &content)).is_err());
    }

    #[test]
    fn rejects_what_is_not_a_members_public_commit() {
        let mut private = vec![0, 1, 0, 2, 1, b'g'];
        private.extend_from_slice(&[0; 8]);
        for (message, why) in [
            (private, "a PrivateMessage"),
            (
                public_message(b"", &[APPLICATION, 0]),
                "an application message",
            ),
            (
                public_message(b"", &[COMMIT, 3, 1, 0, 1]),
                "an Add without a KeyPackage",
            ),
            (
                public_message(b"", &[COMMIT, 3, 1, 0, 2]),
                "an Update by value",
            ),
            (
                public_message(b"", &[COMMIT, 1, 3]),
                "an unknown ProposalOrRef type",
            ),
            (
                public_message(b"", &[COMMIT, 0, 2]),
                "a malformed optional path",
            ),
            (public_message(b"", &[COMMIT, 0]), "a truncated Commit"),
            (
                external_commit(&[COMMIT, 10, 1, 0, 6, 1, 0xaa, 1, 0, 6, 1, 0xbb, 1]),
                "two ExternalInit proposals",
            ),
        ] {
            assert!(public_commit(&message).is_err(), "{why}");
        }
        for sender_type in [2, 3] {
            let mut message = public_message(b"", &[COMMIT, 0, 1]);
            message[14] = sender_type;
            assert!(
                public_commit(&message).is_err(),
                "a Commit from sender type {sender_type}"
            );
        }
    }

    /// A new member's PublicMessage of epoch 1 with `content`, without `authenticated_data`.
    fn external_commit(content: &[u8]) -> Vec<u8> {
        let mut message = vec![0, 1, 0, 1, 1, b'g'];
        message.extend_from_slice(&1u64.to_be_bytes());
        message.extend_from_slice(&[4, 0]);
        message.extend_from_slice(content);
        message
    }

    #[test]
    fn reads_an_external_commit() {
        // An ExternalInit and the Remove of leaf 2 by value, and a path.
        let content = [
            COMMIT, 13, 1, 0, 6, 2, 0xaa, 0xbb, 1, 0, 3, 0, 0, 0, 2, 1, 0xff,
        ];
        assert_eq!(
            public_commit(&external_commit(&content)).unwrap(),
            PublicCommit {
                sender: CommitSender::NewMember,
                authenticated_data: Vec::new(),
                removed: vec![2],
                external_init: true,
                other_proposals: 0,
                path: true,
            }
        );
        assert_eq!(
            framing(&external_commit(&content)).unwrap(),
            Framing {
                wire_format: PUBLIC_MESSAGE,
                group_id: b"g".to_vec(),
                epoch: 1,
                content_type: COMMIT,
            }
        );
    }
}
