//! Reads the framing of an `MLSMessage` from its bytes as RFC 9420, §6, encodes it, apart from the
//! crate under test.

use crate::wire::{MLS10, Reader};

/// The `mls_public_message` wire format.
pub const PUBLIC_MESSAGE: u16 = 1;
/// The `mls_private_message` wire format.
pub const PRIVATE_MESSAGE: u16 = 2;
/// The `mls_welcome` wire format.
pub const WELCOME: u16 = 3;
/// The `application` content type.
pub const APPLICATION: u8 = 1;
/// The `commit` content type.
pub const COMMIT: u8 = 3;

/// The fields of a PublicMessage's or a PrivateMessage's framing the tests check.
#[derive(Debug)]
pub struct Framing {
    pub wire_format: u16,
    pub group_id: Vec<u8>,
    pub epoch: u64,
    pub content_type: u8,
}

/// The framing of a PublicMessage or a PrivateMessage in an `MLSMessage`.
///
/// # Panics
/// If `bytes` is neither.
pub fn framing(bytes: &[u8]) -> Framing {
    let mut reader = Reader { bytes, at: 0 };
    assert_eq!(reader.u16(), MLS10, "the protocol version");
    let wire_format = reader.u16();
    let group_id = reader.vector().to_vec();
    let epoch = u64::from_be_bytes(reader.take(8).try_into().unwrap());
    let content_type = match wire_format {
        PUBLIC_MESSAGE => {
            // FramedContent: the sender, a member's or an external sender's with an index.
            if matches!(reader.u8(), 1 | 2) {
                reader.take(4);
            }
            reader.vector(); // authenticated_data
            reader.u8()
        }
        PRIVATE_MESSAGE => reader.u8(),
        other => panic!("wire format {other}, not a PublicMessage or a PrivateMessage"),
    };
    Framing {
        wire_format,
        group_id,
        epoch,
        content_type,
    }
}

/// The ciphersuite of a Welcome in an `MLSMessage`.
///
/// # Panics
/// If `bytes` is not one.
pub fn welcome_suite(bytes: &[u8]) -> u16 {
    let mut reader = Reader { bytes, at: 0 };
    assert_eq!(reader.u16(), MLS10, "the protocol version");
    assert_eq!(reader.u16(), WELCOME, "a Welcome");
    reader.u16()
}
