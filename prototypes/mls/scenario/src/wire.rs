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
}
