//! What a party without a channel's group secrets reads of an `mls` payload: its kind and, of a
//! PublicMessage or a PrivateMessage, its group, epoch and content type. The server routes and
//! orders a channel's envelopes by it (`docs/protocol/membership.md`, "Commit ordering";
//! `docs/protocol/delivery.md`, "Mailbox").

use mls_rs::{MlsMessage, MlsMessageDescription, group::ContentType, mls_rs_codec::MlsDecode as _};

/// The framing of an `MLSMessage`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MlsFraming {
    Welcome,
    /// A PublicMessage, if `public`, or a PrivateMessage, of group `group` at `epoch`.
    Message {
        group: Vec<u8>,
        epoch: u64,
        public: bool,
        content: MlsContent,
    },
    /// A GroupInfo or a KeyPackage.
    Other,
}

/// The content type of a PublicMessage or a PrivateMessage.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MlsContent {
    Application,
    Proposal,
    Commit,
}

impl MlsFraming {
    /// The framing of `payload`, one `MLSMessage` and nothing after it; `None` if it is not one.
    pub fn read(payload: &[u8]) -> Option<Self> {
        let mut rest = payload;
        let message = MlsMessage::mls_decode(&mut rest).ok()?;
        if !rest.is_empty() {
            return None;
        }
        let (group, epoch, public, content) = match message.description() {
            MlsMessageDescription::Welcome { .. } => return Some(Self::Welcome),
            MlsMessageDescription::GroupInfo | MlsMessageDescription::KeyPackage => {
                return Some(Self::Other);
            }
            MlsMessageDescription::PublicProtocolMessage {
                group_id,
                epoch_id,
                content_type,
                ..
            } => (group_id, epoch_id, true, content_type),
            MlsMessageDescription::PrivateProtocolMessage {
                group_id,
                epoch_id,
                content_type,
            } => (group_id, epoch_id, false, content_type),
        };
        let content = match content {
            ContentType::Application => MlsContent::Application,
            ContentType::Proposal => MlsContent::Proposal,
            ContentType::Commit => MlsContent::Commit,
        };
        Some(Self::Message {
            group: group.to_vec(),
            epoch,
            public,
            content,
        })
    }

    /// Whether it is a handshake message: a Welcome, or a proposal or a Commit (RFC 9420, §6).
    pub fn is_handshake(&self) -> bool {
        match self {
            Self::Welcome => true,
            Self::Message { content, .. } => *content != MlsContent::Application,
            Self::Other => false,
        }
    }
}
