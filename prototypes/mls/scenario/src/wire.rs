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

/// The fields of a KeyPackage's leaf that the Skald profile fixes.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct KeyPackage {
    pub cipher_suite: u16,
    pub signature_key: Vec<u8>,
    pub credential_type: u16,
    pub identity: Vec<u8>,
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

/// The profile's fields of a KeyPackage message.
pub fn key_package(message: &[u8]) -> Result<KeyPackage> {
    let mut input = Input(message);
    expect_wire_format(&mut input, KEY_PACKAGE)?;
    if input.u16()? != PROTOCOL_VERSION {
        return Err("KeyPackage: not mls10".into());
    }
    let cipher_suite = input.u16()?;
    input.vector()?; // init_key
    input.vector()?; // leaf_node.encryption_key
    let signature_key = input.vector()?.to_vec();
    let credential_type = input.u16()?;
    let identity = if credential_type == BASIC {
        input.vector()?.to_vec()
    } else {
        Vec::new()
    };
    Ok(KeyPackage {
        cipher_suite,
        signature_key,
        credential_type,
        identity,
    })
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

    #[test]
    fn reads_a_key_package() {
        let mut message = vec![0, 1, 0, 5, 0, 1, 0, 1];
        message.extend_from_slice(b"\x01i\x01e\x02sk\x00\x01\x03dev");
        assert_eq!(
            key_package(&message).unwrap(),
            KeyPackage {
                cipher_suite: CIPHER_SUITE,
                signature_key: b"sk".to_vec(),
                credential_type: BASIC,
                identity: b"dev".to_vec(),
            }
        );
        message[3] = 3;
        assert!(key_package(&message).is_err());
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
