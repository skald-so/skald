//! Invites, acceptances and invite links: `docs/protocol/membership.md`, "Invite".

use std::fmt;

use base64::Engine as _;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use hkdf::Hkdf;
use sha2::Sha256;
use skald_proto::{
    ACCEPTANCE_LABEL, Acceptance, AcceptanceBody, Cbor, DecodeError, INVITE_LABEL, Invite,
    InviteBody, InviteId,
};
use zeroize::Zeroizing;

use crate::sign::{SignatureError, SigningKey, secret_bytes, verify};

/// The path of a link up to the invite.
const PATH: &str = "/invite/";
/// A link's fragment: the secret and a revision's hash, 64 bytes, in base64url without padding.
const FRAGMENT_LEN: usize = 86;

/// An invite's secret: 32 bytes, zeroed on drop and never shown but in its link.
pub struct InviteSecret(Zeroizing<[u8; 32]>);

secret_bytes!(InviteSecret);

impl InviteSecret {
    /// The invite key pair: the Ed25519 private key
    /// `HKDF-SHA256(salt = "", IKM = secret, info = "skald invite", L = 32)`.
    pub fn invite_key(&self) -> SigningKey {
        let mut private = Zeroizing::new([0; 32]);
        Hkdf::<Sha256>::new(Some(b""), self.0.as_ref())
            .expand(INVITE_LABEL.as_bytes(), private.as_mut())
            .expect("32 bytes of HKDF-SHA256");
        SigningKey::from_bytes(&private)
    }
}

impl fmt::Debug for InviteSecret {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("InviteSecret(..)")
    }
}

/// The invite of `body`, signed by `key`, the signing key of the device in `body.signer`.
///
/// # Errors
/// Why `body` breaks the spec's CDDL.
pub fn sign_invite(body: &InviteBody, key: &SigningKey) -> Result<Invite, DecodeError> {
    let bytes = body.to_cbor();
    let signature = key.sign(INVITE_LABEL, &[&bytes]);
    Invite::from_parts(bytes, signature)
}

/// Checks that `signing` is the signing key of the invite's `signer` and that its signature over
/// `["skald invite", body]` verifies with it.
///
/// # Errors
/// [`SignatureError`] if `signer` does not derive from `signing` or the signature does not verify.
pub fn check_invite(invite: &Invite, signing: &[u8; 32]) -> Result<(), SignatureError> {
    if !invite.body().signer.is_derived_from(signing) {
        return Err(SignatureError);
    }
    verify(
        signing,
        INVITE_LABEL,
        &[invite.body_bytes()],
        invite.signature(),
    )
}

/// The acceptance of `body`, signed by `key`, the invite key.
///
/// # Errors
/// Why `body` breaks the spec's CDDL.
pub fn sign_acceptance(body: &AcceptanceBody, key: &SigningKey) -> Result<Acceptance, DecodeError> {
    let bytes = body.to_cbor();
    let signature = key.sign(ACCEPTANCE_LABEL, &[&bytes]);
    Acceptance::from_parts(bytes, signature)
}

/// Checks that the acceptance's signature over `["skald acceptance", body]` verifies with `key`,
/// the invite's `key`.
///
/// # Errors
/// [`SignatureError`] if it does not.
pub fn check_acceptance(acceptance: &Acceptance, key: &[u8; 32]) -> Result<(), SignatureError> {
    verify(
        key,
        ACCEPTANCE_LABEL,
        &[acceptance.body_bytes()],
        acceptance.signature(),
    )
}

/// An invite link: `<web>/invite/<invite>#<fragment>`, the fragment base64url without padding of
/// the secret followed by the hash of a roster revision.
#[derive(Debug)]
pub struct InviteLink {
    /// The server's web address, an origin such as `https://skald.example`.
    pub web: String,
    pub invite: InviteId,
    pub secret: InviteSecret,
    /// The hash of the roster revision the admin's device last accepted.
    pub revision: [u8; 32],
}

impl InviteLink {
    /// The link as text, zeroed on drop: it holds the secret.
    pub fn shown(&self) -> Zeroizing<String> {
        let mut fragment = Zeroizing::new([0; 64]);
        fragment[..32].copy_from_slice(self.secret.0.as_ref());
        fragment[32..].copy_from_slice(&self.revision);
        let head = format!("{}{PATH}{}#", self.web, self.invite);
        // The whole capacity up front: a reallocation would leave the secret behind unzeroed.
        let mut shown = Zeroizing::new(String::with_capacity(head.len() + FRAGMENT_LEN));
        shown.push_str(&head);
        URL_SAFE_NO_PAD.encode_string(fragment.as_ref(), &mut shown);
        shown
    }

    /// The link `text` reads into.
    ///
    /// # Errors
    /// [`InviteLinkError`] if it is not a web address, the path `/invite/<invite>` with nothing
    /// after it and a fragment of 86 base64url characters without padding.
    pub fn parse(text: &str) -> Result<Self, InviteLinkError> {
        let (head, fragment) = text.split_once('#').ok_or(InviteLinkError::Fragment)?;
        let after_scheme = ["https://", "http://"]
            .into_iter()
            .find_map(|scheme| head.strip_prefix(scheme))
            .ok_or(InviteLinkError::Web)?;
        let path_at = after_scheme.find('/').ok_or(InviteLinkError::Path)?;
        let (authority, path) = after_scheme.split_at(path_at);
        if !is_authority(authority) {
            return Err(InviteLinkError::Web);
        }
        let invite = path
            .strip_prefix(PATH)
            .and_then(|invite| invite.parse().ok())
            .ok_or(InviteLinkError::Path)?;
        // 86 characters of base64url without padding decode to 64 bytes.
        let mut bytes = Zeroizing::new(Vec::with_capacity(64));
        if fragment.len() != FRAGMENT_LEN
            || URL_SAFE_NO_PAD.decode_vec(fragment, &mut bytes).is_err()
        {
            return Err(InviteLinkError::Fragment);
        }
        let secret: &[u8; 32] = bytes[..32].try_into().expect("32 bytes");
        Ok(Self {
            web: head[..head.len() - path.len()].to_owned(),
            invite,
            secret: InviteSecret::from_bytes(secret),
            revision: bytes[32..].try_into().expect("32 bytes"),
        })
    }
}

/// Whether `authority` is a host of lowercase ASCII letters, digits, `.`, `-`, or an IPv6 address
/// in brackets, with an optional port.
fn is_authority(authority: &str) -> bool {
    let (host, port) = match authority.rsplit_once(':') {
        Some((host, port)) if !port.contains(']') => (host, Some(port)),
        _ => (authority, None),
    };
    let host_ok = match host.strip_prefix('[').and_then(|ip| ip.strip_suffix(']')) {
        Some(ip) => !ip.is_empty() && ip.bytes().all(|c| c.is_ascii_hexdigit() || c == b':'),
        None => {
            !host.is_empty()
                && host.bytes().all(|c| {
                    c.is_ascii_lowercase() || c.is_ascii_digit() || matches!(c, b'.' | b'-')
                })
        }
    };
    let port_ok = port.is_none_or(|port| {
        (1..=5).contains(&port.len()) && port.bytes().all(|c| c.is_ascii_digit())
    });
    host_ok && port_ok
}

/// Why text is not an invite link.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InviteLinkError {
    /// It does not start with a web address: `https://` or `http://` and a host.
    Web,
    /// Its path is not `/invite/<invite>`.
    Path,
    /// It has no fragment of the secret and a revision's hash.
    Fragment,
}

impl fmt::Display for InviteLinkError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::Web => {
                "an invite link starts with the server's web address, such as https://skald.example"
            }
            Self::Path => "an invite link's path is /invite/ and the invite's ID",
            Self::Fragment => "an invite link ends with # and 86 characters of base64url",
        })
    }
}

impl std::error::Error for InviteLinkError {}

#[cfg(test)]
mod tests {
    use super::*;

    const FRAGMENT: &str =
        "AAECAwQFBgcICQoLDA0ODxAREhMUFRYXGBkaGxwdHh-hoaGhoaGhoaGhoaGhoaGhoaGhoaGhoaGhoaGhoaGhoQ";
    const INVITE: &str = "019a6f1e-2b3c-7d4e-8f50-61a2b3c4d5f1";

    #[cfg_attr(not(target_arch = "wasm32"), test)]
    #[cfg_attr(target_arch = "wasm32", wasm_bindgen_test::wasm_bindgen_test)]
    fn the_secret_is_never_shown_and_errors_name_the_part() {
        assert_eq!(
            format!("{:?}", InviteSecret::from_bytes(&[0x5e; 32])),
            "InviteSecret(..)"
        );
        assert_eq!(
            InviteLinkError::Web.to_string(),
            "an invite link starts with the server's web address, such as https://skald.example"
        );
        assert_eq!(
            InviteLinkError::Path.to_string(),
            "an invite link's path is /invite/ and the invite's ID"
        );
        assert_eq!(
            InviteLinkError::Fragment.to_string(),
            "an invite link ends with # and 86 characters of base64url"
        );
    }

    /// The link of `web` with the fixed invite and fragment.
    fn link(web: &str) -> String {
        format!("{web}/invite/{INVITE}#{FRAGMENT}")
    }

    #[cfg_attr(not(target_arch = "wasm32"), test)]
    #[cfg_attr(target_arch = "wasm32", wasm_bindgen_test::wasm_bindgen_test)]
    fn a_local_server_over_http_or_an_ip_address_is_a_web_address() {
        for web in [
            "http://localhost:8080",
            "http://127.0.0.1:3000",
            "https://[::1]:8443",
            "http://[::1]",
        ] {
            let read =
                InviteLink::parse(&link(web)).unwrap_or_else(|error| panic!("{web}: {error}"));
            assert_eq!(read.web, web);
            assert_eq!(*read.shown(), link(web));
        }
    }

    #[cfg_attr(not(target_arch = "wasm32"), test)]
    #[cfg_attr(target_arch = "wasm32", wasm_bindgen_test::wasm_bindgen_test)]
    fn other_web_addresses_are_refused() {
        for web in [
            "ftp://skald.example",
            "https://Skald.example",
            "https://user@skald.example",
            "https://skald.example:",
            "https://skald.example:123456",
            "https://skald.example:port",
            "https://:8443",
            "https://[]",
            "https://[::1",
            "https://skald_example",
        ] {
            assert_eq!(
                InviteLink::parse(&link(web)).map(|_| ()),
                Err(InviteLinkError::Web),
                "{web}"
            );
        }
    }
}
