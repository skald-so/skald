//! Key changes the CLI refuses: of another account's agent.

use crate::changes::{STORE_REVOCATION, agent_of, replace_key};
use crate::fixtures::fronted;
use crate::service_kit::{Acme, Snapshot, assert_refused_saying, writes_since};

/// Bob, a member of `acme`, has the service agent `ci/deploy-bot` on his account, in a revision
/// Ada's device signed. Ada's machine cannot replace its key: the command is refused, saying
/// `account`, with no write to the server; both accounts' devices and the space's revisions and
/// tokens stay as they were.
#[test]
fn the_command_refuses_to_replace_the_key_of_another_account_s_agent() {
    let (world, _front) = fronted();
    let cloud = &world.cloud;
    let ada = world.sign_up(3_135_930_001, "ada-otherkey-standin", "ada-workstation");
    let bob = world.sign_up(3_135_930_002, "bob-otherkey-standin", "bob-laptop");
    let acme = Acme::with_member(cloud, &ada, &bob);
    agent_of(cloud, (&bob, &ada), acme.id, acme.default);
    let before = [&ada, &bob].map(|person| Snapshot::take(cloud, person, &ada, acme.id));
    let since = cloud.relay.mark();

    let refused = replace_key(&ada, &[acme.default]);

    assert_refused_saying(&refused, "account");
    let written = writes_since(cloud, since, acme.id) + cloud.count_sent(since, STORE_REVOCATION);
    assert_eq!(
        written,
        0,
        "the refused run wrote to the server: {:?}",
        cloud.relay.sent_since(since)
    );
    let after = [&ada, &bob].map(|person| Snapshot::take(cloud, person, &ada, acme.id));
    assert_eq!(after, before, "{refused:?}");
}
