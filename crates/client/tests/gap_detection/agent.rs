//! As an agent, the CLI's output names nothing of a missing message in another space's channel;
//! the person's next run reports it.

use crate::gaps::{assert_reported_once, bob_in, reports, second_dropped};
use crate::kit::{RECV, create_space, received};
use crate::talk::{BOB_HOST, Talk, assert_shows_once};
use crate::world::{ADD_RULE, AGENT, CLIENT_A, CLIENT_B, dir_of, path, run};

/// The agent the tests run as.
const CLAUDE: &str = "claude-q4m8zt";

/// The texts Bob's device sends; none is anywhere else.
const SAYS: [&str; 3] = [
    "client b invoices are queued",
    "client b invoices half sent",
    "client b invoices all sent",
];

/// Ada's machine follows `clientb`, with a channel whose group holds Bob's device, and `clienta`,
/// to which `~/work/clientA` is bound. Bob's device sends three messages to the channel; the second
/// never reaches the server. As an agent in `~/work/clientA`, `skald recv` reports nothing missing
/// and names nothing of `clientb`, its channels, Bob's device or the texts; the person's next run
/// shows the first and the third and reports messages missing once, naming the channel and Bob's
/// device.
#[test]
fn agent_names_no_missing_message_of_another_space_and_the_person_s_next_run_reports_it() {
    let mut talk = Talk::in_space(
        CLIENT_B,
        &[4_137_076_001, 4_137_076_002],
        &["ada-desk-gaps", "bob-client-gaps"],
        false,
    );
    let folder = dir_of(&talk.ada.machine, "work/clientA");
    create_space(talk.cloud(), &talk.ada, CLIENT_A);
    let bound = run(&talk.ada.machine, &[ADD_RULE, path(&folder), CLIENT_A]);
    assert!(bound.success, "not bound: {bound:?}");
    let (from_bob, channel, bob) = bob_in(&talk);
    second_dropped(&mut talk, SAYS);
    let space = &talk.space;
    let mut names = vec![
        CLIENT_B.to_owned(),
        space.id.to_string(),
        space.default.to_string(),
        channel.to_string(),
        bob.to_string(),
    ];
    names.extend(SAYS.map(str::to_owned));

    let agent = (talk.ada.machine)
        .start_in(&folder, &[AGENT, CLAUDE, RECV])
        .finish();

    assert!(agent.success, "{agent:?}");
    let shown = agent.shown();
    let lower = shown.to_lowercase();
    let named: Vec<&String> = names
        .iter()
        .filter(|name| lower.contains(&name.to_lowercase()))
        .collect();
    assert!(
        named.is_empty(),
        "the agent's output names {named:?}: {agent:?}"
    );
    let reported = reports(&shown);
    assert!(
        reported.is_empty(),
        "the agent's output reports {reported:?}"
    );

    let person = received(&talk.ada.machine).shown();

    for text in [SAYS[0], SAYS[2]] {
        assert_shows_once(&person, text, (channel, &from_bob, BOB_HOST));
    }
    assert_reported_once(&person, channel, bob);
}
