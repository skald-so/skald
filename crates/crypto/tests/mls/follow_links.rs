//! `follow_links` opens each link only with the key of the link's own epoch.

use skald_crypto::{HistoryKey, follow_links};
use skald_proto::ChannelId;

use crate::history::forged_link;

#[cfg_attr(not(target_arch = "wasm32"), test)]
#[cfg_attr(target_arch = "wasm32", wasm_bindgen_test::wasm_bindgen_test)]
fn a_link_of_another_epoch_under_the_starting_key_is_not_followed() {
    let channel = ChannelId::generate();
    let start = [0x55; 16];
    // A link of epoch 4, sealed under epoch 5's key with its own associated data.
    let link = forged_link(&start, channel, 4, 2, &[0x44; 16]);
    let key = HistoryKey::from_bytes(&start);
    assert!(key.open_link(&link).is_ok(), "epoch 5's key opens it");
    assert!(
        follow_links(channel, 5, &key, &[link]).is_empty(),
        "followed: a link of epoch 4 with epoch 5's key"
    );
}
