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
pub const KEY_PACKAGE: u16 = 5;

/// `ContentType` values.
pub const APPLICATION: u8 = 1;
pub const COMMIT: u8 = 3;

/// The `remove` `ProposalType`.
pub const REMOVE: u16 = 3;

/// The `last_resort` KeyPackage extension type [draft-ietf-mls-extensions, §5].
pub const LAST_RESORT: u16 = 0x000a;
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
        let mut extensions = Vec::new();
        for extension in &self.extensions {
            extensions.extend_from_slice(&extension.extension_type.to_be_bytes());
            put_vector(&mut extensions, &extension.data);
        }
        put_vector(&mut out, &extensions);
        out
    }
}

/// A KeyPackage message.
pub fn key_package(message: &[u8]) -> Result<KeyPackage> {
    let mut input = Input(message);
    expect_wire_format(&mut input, KEY_PACKAGE)?;
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
    let mut list = Input(input.vector()?);
    let mut extensions = Vec::new();
    while !list.0.is_empty() {
        extensions.push(Extension {
            extension_type: list.u16()?,
            data: list.vector()?.to_vec(),
        });
    }
    let signature = input.vector()?.to_vec();
    if !input.0.is_empty() {
        return Err("KeyPackage: trailing bytes".into());
    }
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

/// A Commit in a PublicMessage: its sender, `authenticated_data`, proposals and path.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PublicCommit {
    /// The sender's leaf index.
    pub sender: u32,
    pub authenticated_data: Vec<u8>,
    /// The leaf indexes of its Remove proposals, by value.
    pub removed: Vec<u32>,
    /// The number of its other proposals, by value or by reference.
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

/// The Commit of a PublicMessage from a member.
pub fn public_commit(message: &[u8]) -> Result<PublicCommit> {
    let mut input = Input(message);
    expect_wire_format(&mut input, PUBLIC_MESSAGE)?;
    input.vector()?; // group_id
    input.u64()?; // epoch
    if input.u8()? != 1 {
        return Err("PublicMessage: not from a member".into());
    }
    let sender = input.u32()?;
    let authenticated_data = input.vector()?.to_vec();
    if input.u8()? != COMMIT {
        return Err("PublicMessage: not a Commit".into());
    }
    let mut proposals = Input(input.vector()?);
    let (mut removed, mut other_proposals) = (Vec::new(), 0);
    while !proposals.0.is_empty() {
        match proposals.u8()? {
            1 => match proposals.u16()? {
                REMOVE => removed.push(proposals.u32()?),
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
                sender: 2,
                authenticated_data: b"ad".to_vec(),
                removed: vec![0x0102_0304],
                other_proposals: 1,
                path: true,
            }
        );

        let empty = public_commit(&public_message(b"", &[COMMIT, 0, 1])).unwrap();
        assert_eq!((empty.removed, empty.other_proposals), (vec![], 0));
        assert!(empty.path);
        assert!(
            !public_commit(&public_message(b"", &[COMMIT, 0, 0]))
                .unwrap()
                .path
        );
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
                "an Add by value",
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
        ] {
            assert!(public_commit(&message).is_err(), "{why}");
        }
        let mut external = public_message(b"", &[COMMIT, 0, 1]);
        external[14] = 4;
        assert!(public_commit(&external).is_err(), "a new member's Commit");
    }
}
