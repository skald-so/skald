//! Invites and acceptances on the server (`docs/protocol/membership.md`, "Invite") beyond what the
//! tests of invites on the server check: an acceptance's `space-notice` on several instances, and
//! an invite past its expiry. Against `skald-server` instances as in the tests of server instances,
//! with the scene of the tests of invites on the server.

#[path = "../common/mod.rs"]
mod common;

// The spaces tests' accounts and revisions, the fixture and frames of the tests of storing sent
// envelopes, the entries of the tests of delivering a mailbox, the kit of the tests of revocations,
// the scene of the tests of invites and the instances of the tests of server instances; this binary
// uses some of them.
#[allow(dead_code)]
#[path = "../spaces/api.rs"]
mod api;
#[allow(dead_code)]
#[path = "../mailbox/fixture.rs"]
mod fixture;
#[allow(dead_code)]
#[path = "../server_instances/instances.rs"]
mod instances;
#[allow(dead_code)]
#[path = "../revocations/kit.rs"]
mod kit;
#[allow(dead_code)]
#[path = "../mailbox_delivery/link.rs"]
mod link;
#[allow(dead_code)]
#[path = "../spaces/rosters.rs"]
mod rosters;
#[allow(dead_code)]
#[path = "../invites/scene.rs"]
mod scene;
#[allow(dead_code)]
#[path = "../server_instances/watch.rs"]
mod watch;
#[allow(dead_code)]
#[path = "../mailbox/wire.rs"]
mod wire;

mod expiry;
mod instances_notices;
