//! A WASM module with only the browser's GroupInfo check on OpenMLS, for its size.

use wasm_bindgen::prelude::wasm_bindgen;

/// The IDs of the members a browser takes from `group_info`, as
/// `mls_scenario::browser::member_ids` gives them; none if the check fails.
#[wasm_bindgen]
pub fn member_ids(
    group_info: &[u8],
    channel: &[u8],
    epoch: u64,
    signer: &[u8],
    admitted: &[u8],
) -> Option<Vec<u8>> {
    mls_scenario::browser::member_ids::<mls_proto_openmls::OpenMlsObserver>(
        group_info, channel, epoch, signer, admitted,
    )
    .ok()
}
