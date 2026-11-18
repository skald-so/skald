//! CPace: the balanced PAKE of draft-irtf-cfrg-cpace-21, with the cipher suite
//! CPACE-RISTR255-SHA512, initiator-responder. Initiator A starts with its share Ya; responder B
//! answers with its share Yb and has the ISK; A finishes with the ISK from Yb. Each side takes a
//! fresh scalar and uses it once.

use std::fmt;

use curve25519_dalek::ristretto::{CompressedRistretto, RistrettoPoint};
use curve25519_dalek::scalar::Scalar;
use curve25519_dalek::traits::IsIdentity as _;
use sha2::{Digest as _, Sha512};
use zeroize::Zeroizing;

/// The suite's domain separation identifier.
const DSI: &[u8] = b"CPaceRistretto255";
/// The input block size of SHA-512: the generator string pads to it.
const BLOCK: usize = 128;

/// The ISK of a run: 64 bytes, zeroed on drop and never shown.
pub struct Isk(Zeroizing<[u8; 64]>);

impl Isk {
    /// The ISK of these bytes, as [`Isk::to_bytes`] gives them.
    pub fn from_bytes(bytes: &[u8; 64]) -> Self {
        Self(Zeroizing::new(*bytes))
    }

    /// Its 64 bytes.
    pub fn to_bytes(&self) -> Zeroizing<[u8; 64]> {
        self.0.clone()
    }

    pub(crate) fn bytes(&self) -> &[u8; 64] {
        &self.0
    }
}

impl fmt::Debug for Isk {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("Isk(..)")
    }
}

/// A run fails: the other side's share does not decode, or K is the identity.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CpaceError;

impl fmt::Display for CpaceError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("the share does not decode or gives the identity")
    }
}

impl std::error::Error for CpaceError {}

/// Side A of a run after its start: its scalar, used once by [`CpaceInitiator::finish`].
pub struct CpaceInitiator {
    scalar: Zeroizing<Scalar>,
    sid: Vec<u8>,
    share: [u8; 32],
    ad: Vec<u8>,
}

impl CpaceInitiator {
    /// Starts a run with the PRS `prs`, the CI `ci`, the sid `sid` and A's associated data `ad`
    /// and a fresh scalar: A's state and its share Ya.
    ///
    /// # Panics
    /// If the random generator fails.
    pub fn start(prs: &[u8], ci: &[u8], sid: &[u8], ad: &[u8]) -> (Self, [u8; 32]) {
        Self::with(fresh_scalar(), generator(prs, ci, sid), sid, ad)
    }

    /// As [`CpaceInitiator::start`], with the scalar `scalar`, 32 bytes little endian, such as a
    /// test vector's.
    pub fn start_with_scalar(
        scalar: &[u8; 32],
        prs: &[u8],
        ci: &[u8],
        sid: &[u8],
        ad: &[u8],
    ) -> (Self, [u8; 32]) {
        Self::with(given_scalar(scalar), generator(prs, ci, sid), sid, ad)
    }

    fn with(
        scalar: Zeroizing<Scalar>,
        g: RistrettoPoint,
        sid: &[u8],
        ad: &[u8],
    ) -> (Self, [u8; 32]) {
        let share = (g * *scalar).compress().to_bytes();
        let initiator = Self {
            scalar,
            sid: sid.to_vec(),
            share,
            ad: ad.to_vec(),
        };
        (initiator, share)
    }

    /// The ISK, given B's share Yb and its associated data `peer_ad`; the scalar is spent.
    ///
    /// # Errors
    /// [`CpaceError`] if Yb does not decode or K is the identity.
    pub fn finish(self, share: &[u8; 32], peer_ad: &[u8]) -> Result<Isk, CpaceError> {
        let k = shared(&self.scalar, share)?;
        Ok(isk(&self.sid, &k, &self.share, &self.ad, share, peer_ad))
    }
}

/// B's share Yb and the ISK, or why A's share gives none.
type Response = Result<([u8; 32], Isk), CpaceError>;

/// Side B of a run, which answers A's share at once.
pub struct CpaceResponder;

impl CpaceResponder {
    /// Answers A's share Ya, with its associated data `peer_ad`, in a run with the PRS `prs`, the
    /// CI `ci`, the sid `sid` and B's associated data `ad`, with a fresh scalar: B's share Yb and the
    /// ISK.
    ///
    /// # Errors
    /// [`CpaceError`] if Ya does not decode or K is the identity.
    ///
    /// # Panics
    /// If the random generator fails.
    pub fn respond(
        prs: &[u8],
        ci: &[u8],
        sid: &[u8],
        ad: &[u8],
        share: &[u8; 32],
        peer_ad: &[u8],
    ) -> Response {
        let scalar = fresh_scalar();
        Self::with(&scalar, generator(prs, ci, sid), sid, ad, share, peer_ad)
    }

    /// As [`CpaceResponder::respond`], with the scalar `scalar`, 32 bytes little endian, such as a
    /// test vector's.
    ///
    /// # Errors
    /// [`CpaceError`] if Ya does not decode or K is the identity.
    pub fn respond_with_scalar(
        scalar: &[u8; 32],
        prs: &[u8],
        ci: &[u8],
        sid: &[u8],
        ad: &[u8],
        share: &[u8; 32],
        peer_ad: &[u8],
    ) -> Response {
        Self::with(
            &given_scalar(scalar),
            generator(prs, ci, sid),
            sid,
            ad,
            share,
            peer_ad,
        )
    }

    fn with(
        scalar: &Scalar,
        g: RistrettoPoint,
        sid: &[u8],
        ad: &[u8],
        share: &[u8; 32],
        peer_ad: &[u8],
    ) -> Response {
        let k = shared(scalar, share)?;
        let own = (g * scalar).compress().to_bytes();
        Ok((own, isk(sid, &k, share, peer_ad, &own, ad)))
    }
}

/// A scalar from 64 bytes of the cryptographically secure generator, reduced.
fn fresh_scalar() -> Zeroizing<Scalar> {
    let mut wide = Zeroizing::new([0; 64]);
    getrandom::fill(wide.as_mut()).expect("the random generator failed");
    Zeroizing::new(Scalar::from_bytes_mod_order_wide(&wide))
}

/// The scalar of 32 bytes little endian, reduced.
fn given_scalar(bytes: &[u8; 32]) -> Zeroizing<Scalar> {
    Zeroizing::new(Scalar::from_bytes_mod_order(*bytes))
}

/// `calculate_generator(PRS, CI, sid)`: the element of the hash of the generator string.
fn generator(prs: &[u8], ci: &[u8], sid: &[u8]) -> RistrettoPoint {
    let dsi = prepend_len(DSI);
    let pad = BLOCK
        .saturating_sub(1)
        .saturating_sub(prepend_len(prs).len())
        .saturating_sub(dsi.len());
    let string = lv_cat(&[DSI, prs, &vec![0; pad], ci, sid]);
    RistrettoPoint::from_uniform_bytes(&Sha512::digest(&string).into())
}

/// `scalar_mult_vfy(scalar, share)`: K, encoded; an error if `share` does not decode or K is the
/// identity.
fn shared(scalar: &Scalar, share: &[u8; 32]) -> Result<Zeroizing<[u8; 32]>, CpaceError> {
    let point = CompressedRistretto(*share).decompress().ok_or(CpaceError)?;
    let k = point * scalar;
    if k.is_identity() {
        return Err(CpaceError);
    }
    Ok(Zeroizing::new(k.compress().to_bytes()))
}

/// `ISK = H.hash(lv_cat(DSI || "_ISK", sid, K) || transcript_ir(Ya, ADa, Yb, ADb))`.
fn isk(sid: &[u8], k: &[u8; 32], ya: &[u8], ad_a: &[u8], yb: &[u8], ad_b: &[u8]) -> Isk {
    let dsi = [DSI, b"_ISK"].concat();
    let mut hash = Sha512::new();
    hash.update(lv_cat(&[&dsi, sid, k]));
    hash.update(lv_cat(&[ya, ad_a]));
    hash.update(lv_cat(&[yb, ad_b]));
    Isk(Zeroizing::new(hash.finalize().into()))
}

/// `prepend_len(data)`: the length of `data` in LEB128, then `data`.
fn prepend_len(data: &[u8]) -> Vec<u8> {
    let len = data.len();
    // 7 bits a byte, at least one byte.
    let bytes = (usize::BITS - len.leading_zeros()).div_ceil(7).max(1);
    let mut out = Vec::new();
    for at in 0..bytes {
        let byte = (len >> (7 * at)) as u8 & 0x7f;
        out.push(if at + 1 < bytes { byte | 0x80 } else { byte });
    }
    out.extend_from_slice(data);
    out
}

/// `lv_cat(parts...)`: each part with its length prepended.
fn lv_cat(parts: &[&[u8]]) -> Vec<u8> {
    parts.iter().flat_map(|part| prepend_len(part)).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[cfg_attr(not(target_arch = "wasm32"), test)]
    #[cfg_attr(target_arch = "wasm32", wasm_bindgen_test::wasm_bindgen_test)]
    fn prepend_len_writes_the_length_in_leb128() {
        assert_eq!(prepend_len(b""), [0]);
        assert_eq!(prepend_len(&[7; 127])[..1], [127]);
        assert_eq!(prepend_len(&[7; 128])[..2], [0x80, 0x01]);
        assert_eq!(prepend_len(&[7; 300])[..2], [0xac, 0x02]);
        assert_eq!(prepend_len(&[7; 16_384])[..3], [0x80, 0x80, 0x01]);
        assert_eq!(prepend_len(&[7; 300]).len(), 302);
    }

    #[cfg_attr(not(target_arch = "wasm32"), test)]
    #[cfg_attr(target_arch = "wasm32", wasm_bindgen_test::wasm_bindgen_test)]
    fn an_isk_is_never_shown_and_its_error_says_why() {
        assert_eq!(format!("{:?}", Isk::from_bytes(&[7; 64])), "Isk(..)");
        assert_eq!(
            CpaceError.to_string(),
            "the share does not decode or gives the identity"
        );
    }
}
