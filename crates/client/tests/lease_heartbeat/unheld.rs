//! Claims with no session: while the helper keeps the claims of another agent's session, a claim
//! of an agent with no session running, and one of the person's, get no heartbeat; the first ends
//! after its lease, the second holds only while `skald extend` keeps it.

use std::thread;
use std::time::Instant;

use crate::claiming::{COMPLETE, DONE, EXTEND, QUEUED, STARTED};
use crate::helped::Helped;
use crate::scene::{
    LEASE, THIRD, claude_enqueues, extends_since, helped_claims, helped_runs, lease_ends, scene,
    stays_claimed,
};
use crate::senders::LAPTOP_AGENT;
use crate::values::{AIDER, helped_session};

/// What the tests enqueue and answer; no text is anywhere else.
const UNHELD: &str = "heartbeat test: claude asks for a report codex claims with no session";
const EXTENDED: &str = "heartbeat test: claude asks for a rollout ada keeps by hand";
const EXTENDED_DONE: &str = "heartbeat test: ada finished the rollout she kept";

/// On the second machine the helper runs aider's MCP session. Codex, with no session running,
/// claims claude's first task through the helper, and the person claims the second. The server's
/// reads show the lease of codex's task end, after no extend of it but its claim's own. For two
/// leases the person's `skald extend` of the second task each third of the lease keeps it
/// `claimed`; its complete then succeeds. The machines sent no extend of it but the claim's own
/// and the person's.
#[test]
fn claim_with_no_session_running_ends_after_its_lease_unless_the_cli_extends_it() {
    let claims = scene(
        [4_269_031_001, 4_269_031_002],
        ["ada-claims-with-no-session", "bob-watches-the-rollout"],
    );
    let helped = Helped::new(&claims.laptop().machine);
    let folder = claims.senders.laptop_folder.clone();
    let _aider = helped_session(&helped, &folder, &[], (AIDER, crate::client::CLIENT));
    let unheld = claude_enqueues(&claims, UNHELD);
    let since = claims.cloud().relay.mark();
    helped_claims(&claims, &helped, Some(LAPTOP_AGENT), unheld);
    let kept = claude_enqueues(&claims, EXTENDED);
    let kept_since = claims.cloud().relay.mark();
    helped_claims(&claims, &helped, None, kept);

    let mut extended = 1;
    let until = Instant::now() + LEASE * 2;
    while Instant::now() < until {
        thread::sleep(THIRD);
        let id = kept.to_string();
        let ran = helped_runs(&claims, &helped, None, (EXTEND, &[id.as_str()]));
        assert!(ran.success, "the person's extend: {ran:?}");
        extended += 1;
        stays_claimed(&claims, kept, STARTED, std::time::Duration::ZERO);
    }

    let read = lease_ends(&claims, unheld);
    assert_eq!(read.state, QUEUED, "{read:?}");
    assert_eq!(extends_since(&claims, since, unheld).len(), 1, "codex's");
    let id = kept.to_string();
    let ran = helped_runs(
        &claims,
        &helped,
        None,
        (COMPLETE, &[id.as_str(), EXTENDED_DONE]),
    );
    assert!(ran.success, "the person's complete: {ran:?}");
    assert_eq!(claims.read(kept).state, DONE);
    assert_eq!(
        extends_since(&claims, kept_since, kept).len(),
        extended,
        "the person's"
    );
}
