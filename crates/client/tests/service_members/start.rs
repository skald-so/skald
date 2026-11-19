//! Where a run of the agent starts its space's roster: from the latest revision its account's
//! `client` devices anchored, past its key secret's anchor.

use skald_proto::Role;

use crate::kit::{latest, received};
use crate::member::Bot;
use crate::revocations::revoked_by;
use crate::world::{next_by, set_role, store, warnings};

/// Ada's device makes Bob an admin, and Bob's device signs the next revision, both past the key
/// secret's anchor. Ada's machine accepts them, so her device's anchors object names the second;
/// then Bob's device revokes itself. The agent's first run accepts both revisions, the one its
/// revoked signer signed included, without a warning that one fails.
#[test]
fn a_run_starts_from_the_revision_its_accounts_devices_anchored() {
    let bot = Bot::created(
        [3_135_750_001, 3_135_750_002],
        ["ada-member-start-standin", "bob-member-start-standin"],
    );
    let (cloud, space) = (bot.talk.cloud(), bot.talk.space.id);
    let (ada, bob) = (&bot.talk.ada, &bot.talk.played[0].person);
    let promoted = next_by(&latest(cloud, &ada.token, space), ada, |members| {
        set_role(members, bob.account, Role::Admin);
    });
    store(cloud, &ada.token, &promoted);
    let by_bob = next_by(&promoted, bob, |_| {});
    store(cloud, &bob.token, &by_bob);
    received(&ada.machine);
    revoked_by(cloud, bob, bob.device);

    let ran = bot.runner.received();
    let failing: Vec<String> = (warnings(&ran).into_iter())
        .filter(|line| line.contains("fails"))
        .collect();
    assert_eq!(failing, Vec::<String>::new(), "{ran:?}");
}
