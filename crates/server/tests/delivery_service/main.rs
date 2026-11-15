//! The server as a channel's MLS Delivery Service (`docs/protocol/membership.md`, "MLS profile",
//! "Creating a channel" and "Commit ordering"; `docs/protocol/delivery.md`, "Send", "Mailbox" and
//! "Order"; `docs/protocol/README.md`, "Envelope"). It follows each channel's group from outside
//! it, from the GroupInfo and the Commits it takes:
//! - it keeps a channel's first Commit of epoch 0 and refuses later ones with `epoch-mismatch`; it
//!   takes the GroupInfo of epoch 1 only from that Commit's sender and starts following from it;
//!   until then it refuses the channel's other `mls` envelopes with `not-a-member`;
//! - then it accepts a Commit only of the current epoch, otherwise `epoch-mismatch`, and only from
//!   the device of its sender leaf or, for an external Commit, its joiner, otherwise
//!   `not-a-member`, as it is for a Commit it cannot process and a PublicMessage proposal; of
//!   Commits sent at once, across instances too, the first stored wins; the refusals of storing
//!   sent envelopes come first;
//! - it keeps a GroupInfo of the current epoch from a session of an account with a member device
//!   that it lists, with `ratchet_tree` and `external_pub`, that verifies and has the GroupContext
//!   of the group it follows; it gives it only while it is of the current epoch, and only to
//!   sessions of accounts the latest revision lists as the channel's members;
//! - it adds an application message to the mailbox of each member device of the current epoch but
//!   its sender, which must be one; a Commit to each member device of the epoch it commits from but
//!   its sender; a Welcome to its `recipients`, each a member device of the current epoch; none to
//!   a revoked device; otherwise `not-a-member` and `unknown-recipient`;
//! - it stores a channel's envelopes one at a time, in one order across instances, and goes on
//!   from the group it follows after a restart; entries of handshake messages do not expire by
//!   time.
//!
//! Against a `skald-server` as in the tests of storing sent envelopes, whose fixture and frames
//! these tests share; the devices run MLS with `skald-crypto`, and with an MLS client on mls-rs of
//! the tests' own for what `skald-crypto` does not make, such as an external Commit.
//!
//! The interfaces, as these tests use them, besides those of storing and delivering a mailbox:
//! - `mls` envelopes: `scheme` `mls`, the payload an `MLSMessage`, the send-key `[msg_id]`; under
//!   `mls` the problem `epoch-mismatch` too.
//! - `PUT /spaces/{space}/channels/{channel}/group-info` of `{"group_info": <bytes>}`, a GroupInfo in
//!   an `MLSMessage`: taken, `2xx`; refused, `4xx` with a problem details object.
//! - `GET /spaces/{space}/channels/{channel}/group-info`: `200` with
//!   `{"group": <bool>, "group_info": <bytes> | null}`, whether the channel has a group, its first
//!   Commit kept, and its GroupInfo of the current epoch if the server has one; refused, `4xx` with
//!   a problem details object.
//! - Revoking a device: `POST /account/revocations` of `{"revocation": <bytes>}`, a `revocation`
//!   of `docs/protocol/membership.md`, "Revocation", from a session of its account: taken, `200` or
//!   `201`.
//!
//! Bytes are base64url without padding. Every response is checked against the OpenAPI spec.

#[path = "../common/mod.rs"]
mod common;

// The spaces tests' accounts and revisions; this binary uses some of them.
#[allow(dead_code)]
#[path = "../spaces/api.rs"]
mod api;
#[allow(dead_code)]
#[path = "../spaces/rosters.rs"]
mod rosters;

// The fixture and frames of the tests of storing sent envelopes, and the connections of the tests
// of delivering a mailbox; this binary uses some of them.
#[allow(dead_code)]
#[path = "../mailbox/fixture.rs"]
mod fixture;
#[allow(dead_code)]
#[path = "../mailbox_delivery/link.rs"]
mod link;
#[allow(dead_code)]
#[path = "../mailbox/wire.rs"]
mod wire;

mod foreign;
mod scene;

mod commits;
mod creating;
mod expiry;
mod group_info;
mod instances;
mod routing;
