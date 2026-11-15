//! A channel's view from outside its group, saved and loaded, as the server keeps it in its
//! database so that every instance and a restart go on from it; and whether a GroupInfo has the
//! view's GroupContext, so that the server keeps only GroupInfo of the group it follows
//! (`docs/protocol/membership.md`, "MLS profile" and "Commit ordering"). Natively and in WASM.
//!
//! The interface, as these tests use it, `ChannelView` of the tests of a channel seen from outside
//! its group besides:
//! - `ChannelView::save(&self) -> Result<Vec<u8>, MlsError>`: the view's bytes;
//! - `ChannelView::load(channel: ChannelId, saved: &[u8]) -> Result<ChannelView, MlsError>`: the
//!   view of `channel` those bytes hold, refused for bytes that are not a saved view of `channel`;
//! - `ChannelView::has_context(&self, group_info: &[u8]) -> bool`: whether `group_info`, a GroupInfo
//!   in an `MLSMessage`, has the GroupContext of the view's epoch; `false` for anything else.

#[path = "../common/devices.rs"]
mod devices;
#[path = "../mls/fixtures.rs"]
#[allow(dead_code)]
mod fixtures;
#[path = "../mls/steps.rs"]
#[allow(dead_code)]
mod steps;

mod views;
