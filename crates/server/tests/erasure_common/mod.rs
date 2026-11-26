//! The modules the tests of erasing an account's data (`account_erasure/main.rs`) and of the rows
//! it erases (`erased_account_rows/main.rs`) share, by name: the helpers of every server test; the
//! spaces tests' accounts and revisions; the fixture and frames of the tests of storing sent
//! envelopes; the revocations of the tests of revocations on the server; the second devices of the
//! tests of delivering a mailbox; the anchors objects of the tests of anchors; the requests and
//! checks of the tests of account deletion; the API calls of the tests of sessions on the web; the
//! channel's group and history uploads of the tests of a channel's history; and the checks and
//! shared space of the tests of erasing an account's data.
//!
//! A test target declares this module and imports its modules into its root, so they reach each
//! other as `crate::<module>`.

#[path = "../common/mod.rs"]
pub mod common;

#[path = "../spaces/api.rs"]
pub mod api;
#[path = "../account_deletion/deletion.rs"]
pub mod deletion;
#[path = "../account_erasure/erasure.rs"]
pub mod erasure;
#[path = "../mailbox/fixture.rs"]
pub mod fixture;
#[path = "../channel_history/kit.rs"]
pub mod history;
#[path = "../revocations/kit.rs"]
pub mod kit;
#[path = "../mailbox_delivery/link.rs"]
pub mod link;
#[path = "../mailbox_delivery/listener.rs"]
pub mod listener;
#[path = "../anchors/objects.rs"]
pub mod objects;
#[path = "../spaces/rosters.rs"]
pub mod rosters;
#[path = "../delivery_service/scene.rs"]
pub mod scene;
#[path = "../web_sessions/sessions.rs"]
pub mod sessions;
#[path = "../account_erasure/shared.rs"]
pub mod shared;
#[path = "../mailbox/wire.rs"]
pub mod wire;
