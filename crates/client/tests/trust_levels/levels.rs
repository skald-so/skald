//! An agent's lines name each sender and the level its device gives, `own` for Ada's second
//! machine and `peer` for Bob's device, with one warning line before `peer` lines and none for
//! `own` lines alone; `skald recv --wait` shows them as `skald recv` does.

use crate::senders::{
    BOB_AGENT, LAPTOP, LAPTOP_AGENT, OWN, PEER, Senders, assert_labelled, assert_unwarned,
    assert_warned_before_others,
};
use crate::talk::BOB_HOST;
use crate::world::{ACME, HOST};

/// What the senders say; none is anywhere else.
const LAPTOP_AGENT_SAYS: &str = "level test: the laptop agent finished the migration";
const BOB_AGENT_SAYS: &str = "level test: bob's agent asks to rerun the pipeline";
const BOB_PERSON_SAYS: &str = "level test: bob himself approves the rollout";
const LAPTOP_PERSON_SAYS: &str = "level test: ada notes the rollout from her laptop";

/// Ada's device has run as claude in `~/work/acme`; her second machine is linked and in the
/// channel. Its agent says a line, and claude's run, with `--wait` if `wait`, shows it as
/// `[<ada>/<agent> · ada-laptop · own]`, with no warning line. Then Bob's device says a line as
/// his agent and one as himself, and the second machine's person says one; claude's next run shows
/// Bob's as `[<bob>/<agent> · bob-workstation · peer]` and `[acme/<bob>/user@localhost ·
/// bob-workstation · peer]`, Ada's as `[acme/<ada>/user@localhost · ada-laptop · own]`, with one
/// warning line, before Bob's lines.
fn shows_levels_and_one_warning_before_peer_lines(ids: [u64; 2], logins: [&str; 2], wait: bool) {
    let mut senders = Senders::new(ids, logins);
    let (ada, bob) = (senders.ada(), senders.bob());
    senders.laptop_agent_says(LAPTOP_AGENT, LAPTOP_AGENT_SAYS);

    let own_only = senders.claude_reads(wait);

    let laptop_agent = format!("{ada}/{LAPTOP_AGENT}");
    assert_labelled(
        &own_only.stdout,
        LAPTOP_AGENT_SAYS,
        (&laptop_agent, LAPTOP, OWN),
    );
    assert_unwarned(&own_only.shown());

    let bob_agent = senders.bob_agent();
    senders.bob_says_as(&bob_agent, BOB_AGENT_SAYS);
    let bob_person = senders.agents.bob();
    senders.bob_says_as(&bob_person, BOB_PERSON_SAYS);
    senders.laptop_person_says(LAPTOP_PERSON_SAYS);

    let mixed = senders.claude_reads(wait);

    let bob_agent = format!("{bob}/{BOB_AGENT}");
    assert_labelled(&mixed.stdout, BOB_AGENT_SAYS, (&bob_agent, BOB_HOST, PEER));
    assert_labelled(
        &mixed.stdout,
        BOB_PERSON_SAYS,
        (&bob_person, BOB_HOST, PEER),
    );
    let ada_person = format!("{ACME}/{ada}/user@{HOST}");
    assert_labelled(
        &mixed.stdout,
        LAPTOP_PERSON_SAYS,
        (&ada_person, LAPTOP, OWN),
    );
    assert_warned_before_others(&mixed);
}

#[test]
fn agent_sees_own_lines_unwarned_and_peer_lines_after_one_warning() {
    shows_levels_and_one_warning_before_peer_lines(
        [4_152_001_001, 4_152_001_002],
        ["ada-reads-levels", "bob-sends-levels"],
        false,
    );
}

#[test]
fn waiting_agent_sees_the_same_levels_and_warning() {
    shows_levels_and_one_warning_before_peer_lines(
        [4_152_002_001, 4_152_002_002],
        ["ada-waits-for-levels", "bob-sends-to-a-waiter"],
        true,
    );
}
