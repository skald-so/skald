//! Signing anchors objects and checking their signature: `docs/protocol/membership.md`, "Anchors".

use skald_proto::{ANCHORS_LABEL, Anchors, AnchorsBody, Cbor, DecodeError};

use crate::sign::{SignatureError, SigningKey, verify};

/// The anchors object of `body`, signed by `key`, the signing key of the device in `body.device`.
///
/// # Errors
/// Why `body` breaks the spec's CDDL.
pub fn sign_anchors(body: &AnchorsBody, key: &SigningKey) -> Result<Anchors, DecodeError> {
    let bytes = body.to_cbor();
    let signature = key.sign(ANCHORS_LABEL, &[&bytes]);
    Anchors::from_parts(bytes, signature)
}

/// Checks that `signing` is the signing key of the object's `device` and that its signature over
/// `["skald anchors", body]` verifies with it.
///
/// # Errors
/// [`SignatureError`] if `device` does not derive from `signing` or the signature does not verify.
pub fn check_anchors(anchors: &Anchors, signing: &[u8; 32]) -> Result<(), SignatureError> {
    if !anchors.body().device.is_derived_from(signing) {
        return Err(SignatureError);
    }
    verify(
        signing,
        ANCHORS_LABEL,
        &[anchors.body_bytes()],
        anchors.signature(),
    )
}
