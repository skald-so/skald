//! The modules of other test targets that the tests of the channel history share, by name: the
//! server's tests' readers of the KeyPackages API's answers and the person's browser; the links of
//! the tests of the key chain; the server, relay and machines of the tests of signing in; the
//! helpers of every client test; the admin commands' kit of the tests of removing members; the
//! fixtures and front of the tests of sending; the joins of the tests of joining channels; finding
//! keys in bytes; the kit and played devices of the tests of channel groups; the device the tests
//! of receiving play; the revocations of the tests of revoking devices; the talk of the tests of
//! channel messages; people in spaces, of the tests of roster revisions.
//!
//! A test target declares this module and imports its modules into its root, so they reach each
//! other as `crate::<module>`.

#[path = "../../../server/tests/key_packages/answers.rs"]
pub mod answers;
#[path = "../../../server/tests/common/browser.rs"]
pub mod browser;
#[path = "../key_chain/chain.rs"]
pub mod chain;
#[path = "../sign_in/cloud.rs"]
pub mod cloud;
#[path = "../common/mod.rs"]
pub mod common;
#[path = "../member_removal/edits.rs"]
pub mod edits;
#[path = "../sending/fixtures.rs"]
pub mod fixtures;
#[path = "../sending/front.rs"]
pub mod front;
#[path = "../channel_joins/joins.rs"]
pub mod joins;
#[path = "../device_keys/keys.rs"]
pub mod keys;
#[path = "../channel_groups/kit.rs"]
pub mod kit;
#[path = "../sign_in/machine.rs"]
pub mod machine;
#[path = "../receiving/peer.rs"]
pub mod peer;
#[path = "../channel_groups/played.rs"]
pub mod played;
#[path = "../sign_in/relay.rs"]
pub mod relay;
#[path = "../device_revocation/revocations.rs"]
pub mod revocations;
#[path = "../channel_messages/talk.rs"]
pub mod talk;
#[path = "../roster_revisions/world.rs"]
pub mod world;
