//! The check a linked browser makes of a channel's latest GroupInfo. A browser is outside the
//! channel's MLS group: it takes the group's member devices from that GroupInfo with its ratchet
//! tree, signed by a member leaf (`docs/protocol/README.md`, "Envelope"), and writes history
//! entries under a wrap's key only once the GroupInfo passes the checks of
//! `docs/protocol/membership.md`, "Wraps".

use crate::{Admitted, DeviceId, Leaf, Observer, Result, wire};

/// The length of an admitted device in [`member_ids`]: its ID and its signing key.
const ADMITTED_LEN: usize = 16 + 32;

/// The member devices of `channel` in `epoch` that a browser takes from `group_info`, an encoded
/// `MLSMessage` with a GroupInfo and the `ratchet_tree` extension, for a wrap of `epoch` by device
/// `signer`. Fails unless:
/// - the GroupInfo is of `channel` and `epoch`;
/// - the library of `O` verifies its signature with the leaf of its signer in its tree, and the
///   tree is valid;
/// - it lists `signer` as a member;
/// - every member it lists is admitted.
pub fn members<O: Observer>(
    group_info: &[u8],
    channel: &[u8],
    epoch: u64,
    signer: &DeviceId,
    admitted: &Admitted,
) -> Result<Vec<Leaf>> {
    let fields = wire::group_info(group_info)?;
    if fields.group_id != channel {
        return Err("GroupInfo: of another channel".into());
    }
    if fields.epoch != epoch {
        return Err("GroupInfo: not of the wrap's epoch".into());
    }
    let members = O::observe(group_info)?.members()?;
    if !members.iter().any(|leaf| leaf.identity == signer) {
        return Err("GroupInfo: does not list the wrap's signer".into());
    }
    members.iter().try_for_each(|leaf| admitted.check(leaf))?;
    Ok(members)
}

/// [`members`] with its arguments and result as bytes, as a WASM module exports it: `admitted`
/// holds the ID and the signing key of each admitted device, one after another; the result, the
/// ID of each member.
pub fn member_ids<O: Observer>(
    group_info: &[u8],
    channel: &[u8],
    epoch: u64,
    signer: &[u8],
    admitted: &[u8],
) -> Result<Vec<u8>> {
    let (devices, rest) = admitted.as_chunks::<ADMITTED_LEN>();
    if !rest.is_empty() {
        return Err("admitted devices: not a whole number of devices".into());
    }
    let admitted = devices
        .iter()
        .fold(Admitted::default(), |admitted, device| {
            let (identity, signature_key) = device.split_at(16);
            admitted.with(&Leaf {
                identity: identity.to_vec(),
                signature_key: signature_key.to_vec(),
            })
        });
    let members = members::<O>(group_info, channel, epoch, signer.try_into()?, &admitted)?;
    Ok(members.into_iter().flat_map(|leaf| leaf.identity).collect())
}
