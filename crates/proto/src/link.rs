//! Link objects and the messages of a link code's CPace run, which the server relays, with the
//! request and grant bodies they seal: `docs/protocol/membership.md`, "Linking a device" and "Link
//! code".

use crate::anchors::{finish_anchors, take_anchors};
use crate::cbor::{
    self, Cbor as _, Codec, DecodeError, FieldProblem, Item, MapOut, Unknown, field_error,
};
use crate::cert::{CertBody, DeviceKind, device_name};
use crate::id::{DeviceId, SpaceId};
use crate::signed::{Signed, SignedBody};

/// The label of a link object's signature.
pub const LINK_LABEL: &str = "skald link";

/// A link object, signed by its `issuer` over `["skald link", body]`.
pub type Link = Signed<LinkBody>;

/// What a [`Link`] signs: the revisions a device linked with a link code starts from.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LinkBody {
    /// The new device.
    pub device: DeviceId,
    /// The device that issued the new device's certificate: the signer.
    pub issuer: DeviceId,
    /// Each space and the hash of the latest revision of it the issuer accepted, in the body's
    /// order.
    pub anchors: Vec<(SpaceId, [u8; 32])>,
    /// The account's personal space the issuer pinned, if any.
    pub personal: Option<SpaceId>,
    pub unknown: Unknown,
}

impl SignedBody for LinkBody {
    const LABEL: &'static str = LINK_LABEL;
    const FIELDS: [&'static str; 3] = ["link", "link.body", "link.signature"];
}

impl Codec for LinkBody {
    fn encode(&self) -> Vec<u8> {
        let map = (MapOut::new(&self.unknown).put(1, cbor::id(&self.device)))
            .put(2, cbor::id(&self.issuer));
        finish_anchors(map, &self.anchors, self.personal.as_ref())
    }

    fn decode(item: &Item<'_>) -> Result<Self, DecodeError> {
        let mut fields = item.map("link-body")?;
        let device = (fields.required(1, "link-body.device")?).id("link-body.device")?;
        let issuer = (fields.required(2, "link-body.issuer")?).id("link-body.issuer")?;
        let (anchors, personal) =
            take_anchors(&mut fields, ["link-body.anchors", "link-body.personal"])?;
        Ok(Self {
            device,
            issuer,
            anchors,
            personal,
            unknown: fields.unknown(),
        })
    }
}

/// A `link-request-body`: what the new device asks to be certified as.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LinkRequestBody {
    pub kind: DeviceKind,
    /// The Ed25519 public key.
    pub signing: [u8; 32],
    /// The X25519 public key.
    pub hpke: [u8; 32],
    /// 1 to 64 bytes.
    pub name: String,
    /// The new device's SSH signature for its certificate's `ssh`. Set exactly when `github` is.
    pub ssh: Option<Vec<u8>>,
    /// GitHub's user ID for its certificate's `github`. Set exactly when `ssh` is.
    pub github: Option<u64>,
    pub unknown: Unknown,
}

impl Codec for LinkRequestBody {
    fn encode(&self) -> Vec<u8> {
        MapOut::new(&self.unknown)
            .put(1, cbor::uint(self.kind as u64))
            .put(2, cbor::bytes(&self.signing))
            .put(3, cbor::bytes(&self.hpke))
            .put(4, cbor::text(&self.name))
            .put_some(5, self.ssh.as_deref().map(cbor::bytes))
            .put_some(6, self.github.map(cbor::uint))
            .finish()
    }

    fn decode(item: &Item<'_>) -> Result<Self, DecodeError> {
        let mut fields = item.map("link-request-body")?;
        let body = Self {
            kind: DeviceKind::decode(
                fields.required(1, "link-request-body.kind")?,
                "link-request-body.kind",
            )?,
            signing: fields
                .required(2, "link-request-body.signing")?
                .fixed("link-request-body.signing")?,
            hpke: fields
                .required(3, "link-request-body.hpke")?
                .fixed("link-request-body.hpke")?,
            name: device_name(
                fields.required(4, "link-request-body.name")?,
                "link-request-body.name",
            )?,
            ssh: fields
                .take(5)
                .map(|item| item.bytes("link-request-body.ssh").map(<[u8]>::to_vec))
                .transpose()?,
            github: fields
                .take(6)
                .map(|item| item.uint("link-request-body.github"))
                .transpose()?,
            unknown: fields.unknown(),
        };
        // `ssh` and `github` come both or neither, as in a certificate.
        match (&body.ssh, body.github) {
            (Some(_), None) => Err(field_error(
                "link-request-body.github",
                FieldProblem::Missing,
            )),
            (None, Some(_)) => Err(field_error("link-request-body.ssh", FieldProblem::Missing)),
            _ => Ok(body),
        }
    }
}

/// A `link-grant-body`: what the issuing device grants the new device.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LinkGrantBody {
    /// The master key the issuing device pinned.
    pub master: [u8; 32],
    /// The new device's certificate body, encoded, as the issuing device signed it; it decodes as a
    /// [`CertBody`].
    pub cert: Vec<u8>,
    /// By the issuing device over `["skald cert", cert]`.
    pub signature: [u8; 64],
    pub link: Link,
    pub unknown: Unknown,
}

impl Codec for LinkGrantBody {
    fn encode(&self) -> Vec<u8> {
        MapOut::new(&self.unknown)
            .put(1, cbor::bytes(&self.master))
            .put(2, cbor::bytes(&self.cert))
            .put(3, cbor::bytes(&self.signature))
            .put(4, self.link.to_cbor())
            .finish()
    }

    fn decode(item: &Item<'_>) -> Result<Self, DecodeError> {
        let mut fields = item.map("link-grant-body")?;
        let cert = fields.required(2, "link-grant-body.cert")?;
        // The body must be one, and stays as it came.
        cert.embedded::<CertBody>("link-grant-body.cert")?;
        let link = fields.required(4, "link-grant-body.link")?;
        Ok(Self {
            master: fields
                .required(1, "link-grant-body.master")?
                .fixed("link-grant-body.master")?,
            cert: cert.bytes("link-grant-body.cert")?.to_vec(),
            signature: fields
                .required(3, "link-grant-body.signature")?
                .fixed("link-grant-body.signature")?,
            link: Link::from_cbor(link.raw()).map_err(|error| {
                field_error(
                    "link-grant-body.link",
                    FieldProblem::Embedded(Box::new(error)),
                )
            })?,
            unknown: fields.unknown(),
        })
    }
}

cbor::cbor!(LinkBody, LinkRequestBody, LinkGrantBody);

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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn each_message_encodes_as_its_type_its_session_and_its_payload() {
        let abort = LinkMessage::Abort { session: [7; 16] };
        let mut want = vec![0x82, 0x05, 0x50];
        want.extend([7; 16]);
        assert_eq!(abort.to_cbor(), want);
        let hello = LinkMessage::Hello {
            session: [7; 16],
            share: [9; 32],
        };
        let mut want = vec![0x83, 0x01, 0x50];
        want.extend([7; 16]);
        want.extend([0x58, 0x20]);
        want.extend([9; 32]);
        assert_eq!(hello.to_cbor(), want);
    }

    /// As in a certificate, a request carries `ssh` and `github` both or neither.
    #[test]
    fn a_request_with_only_one_of_ssh_and_github_is_refused() {
        let request = |ssh: Option<Vec<u8>>, github: Option<u64>| LinkRequestBody {
            kind: DeviceKind::Client,
            signing: [1; 32],
            hpke: [2; 32],
            name: "Laptop".to_owned(),
            ssh,
            github,
            unknown: Unknown::default(),
        };
        for (ssh, github) in [(Some(vec![3; 8]), None), (None, Some(583_231))] {
            let encoded = request(ssh, github).to_cbor();
            assert!(LinkRequestBody::from_cbor(&encoded).is_err());
        }
        let both = request(Some(vec![3; 8]), Some(583_231));
        assert_eq!(LinkRequestBody::from_cbor(&both.to_cbor()), Ok(both));
    }

    #[test]
    fn each_message_reads_back_with_its_session() {
        let messages = [
            LinkMessage::Hello {
                session: [1; 16],
                share: [9; 32],
            },
            LinkMessage::Reply {
                session: [2; 16],
                share: [8; 32],
            },
            LinkMessage::Request {
                session: [3; 16],
                sealed: vec![7; 40],
            },
            LinkMessage::Grant {
                session: [4; 16],
                sealed: vec![6; 50],
            },
            LinkMessage::Abort { session: [5; 16] },
        ];
        for (at, message) in (1..).zip(&messages) {
            assert_eq!(message.session(), &[at; 16]);
            assert_eq!(
                LinkMessage::from_cbor(&message.to_cbor()).as_ref(),
                Ok(message)
            );
        }
    }
}
