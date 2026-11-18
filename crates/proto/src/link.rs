//! The messages of a link code's CPace run, which the server relays: `docs/protocol/membership.md`,
//! "Link code".

use crate::cbor::{self, Codec, DecodeError, FieldProblem, Item, field_error};

const HELLO: u64 = 1;
const REPLY: u64 = 2;
const REQUEST: u64 = 3;
const GRANT: u64 = 4;
const ABORT: u64 = 5;

/// A `link-message`. `session` is the run's CPace sid, from the new device; `share` an encoded
/// ristretto255 element; `sealed` an AES-128-GCM ciphertext followed by its tag.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LinkMessage {
    /// New device → issuing device: Ya.
    Hello { session: [u8; 16], share: [u8; 32] },
    /// Issuing device → new device: Yb.
    Reply { session: [u8; 16], share: [u8; 32] },
    /// New device → issuing device: its kind, keys and `name`, sealed.
    Request { session: [u8; 16], sealed: Vec<u8> },
    /// Issuing device → new device: its certificate, the master key and the link object, sealed.
    Grant { session: [u8; 16], sealed: Vec<u8> },
    /// Either way: the run failed.
    Abort { session: [u8; 16] },
}

impl LinkMessage {
    /// The run it belongs to.
    pub fn session(&self) -> &[u8; 16] {
        match self {
            Self::Hello { session, .. }
            | Self::Reply { session, .. }
            | Self::Request { session, .. }
            | Self::Grant { session, .. }
            | Self::Abort { session } => session,
        }
    }
}

impl Codec for LinkMessage {
    fn encode(&self) -> Vec<u8> {
        let (kind, rest) = match self {
            Self::Hello { share, .. } => (HELLO, Some(cbor::bytes(share))),
            Self::Reply { share, .. } => (REPLY, Some(cbor::bytes(share))),
            Self::Request { sealed, .. } => (REQUEST, Some(cbor::bytes(sealed))),
            Self::Grant { sealed, .. } => (GRANT, Some(cbor::bytes(sealed))),
            Self::Abort { .. } => (ABORT, None),
        };
        cbor::array(
            [cbor::uint(kind), cbor::bytes(self.session())]
                .into_iter()
                .chain(rest),
        )
    }

    fn decode(item: &Item<'_>) -> Result<Self, DecodeError> {
        let kind = item
            .array("link-message")?
            .first()
            .ok_or(field_error("link-message", FieldProblem::Missing))?
            .uint("link-message.type")?;
        if kind == ABORT {
            let [_, session] = item.tuple("link-abort")?;
            return Ok(Self::Abort {
                session: session.fixed("link-abort.session")?,
            });
        }
        let [_, session, last] = item.tuple("link-message")?;
        let session = session.fixed("link-message.session")?;
        Ok(match kind {
            HELLO => Self::Hello {
                session,
                share: last.fixed("link-hello.share")?,
            },
            REPLY => Self::Reply {
                session,
                share: last.fixed("link-reply.share")?,
            },
            REQUEST => Self::Request {
                session,
                sealed: last.bytes("link-request.sealed")?.to_vec(),
            },
            GRANT => Self::Grant {
                session,
                sealed: last.bytes("link-grant.sealed")?.to_vec(),
            },
            other => {
                return Err(field_error("link-message.type", FieldProblem::Value(other)));
            }
        })
    }
}

cbor::cbor!(LinkMessage);
