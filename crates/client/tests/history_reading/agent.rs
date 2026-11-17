//! As an agent, the history command reads only channels of its folder's space.

use skald_proto::Role;

use crate::fixtures::person_address;
use crate::kit::{assert_names_none, bind_client_a, create_space, received, with_channel};
use crate::reading::{HISTORY, assert_history, says, shown_history};
use crate::talk::{Line, Talk};
use crate::world::{AGENT, CLIENT_A, CLIENT_B};

/// The agent the tests run as.
const CLAUDE: &str = "claude-h5t3mq";

/// Ada's machine follows `clientb`, with a channel with Bob, and `clienta`, with a channel with
/// Bob too, and `~/work/clientA` is bound to `clienta`; Bob says a line in each channel. As an
/// agent in `~/work/clientA`, `skald history` shows clienta's line; for clientb's channel it fails,
/// names nothing of `clientb`, its default channel or its line, and requests nothing of that
/// channel's history. The person's `skald history` of clientb's channel shows its line.
#[test]
fn agent_reads_the_history_of_its_folder_s_space_alone() {
    let mut talk = Talk::in_space(
        CLIENT_B,
        &[4_166_013_001, 4_166_013_002],
        &["ada-desk-history", "bob-client-history"],
        false,
    );
    let cloud = &talk.world.cloud;
    let client_a = create_space(cloud, &talk.ada, CLIENT_A);
    let bob_s = [(&talk.played[0].person, Role::Member)];
    let in_a = (
        client_a.id,
        with_channel(cloud, client_a.id, &talk.ada, &bob_s),
    );
    received(&talk.ada.machine);
    let folder = bind_client_a(&talk.ada.machine);
    let from_bob_in_a = person_address(CLIENT_A, &talk.played[0].person);
    let from_bob_in_b = talk.address(&talk.played[0].person);
    let in_b = talk.at();
    let bob = &mut talk.played[0];
    let (said_in_a, ..) = says(
        cloud,
        bob,
        in_a,
        &Line::new(&from_bob_in_a, 1, "client a's plan"),
    );
    let (said_in_b, ..) = says(
        cloud,
        bob,
        in_b,
        &Line::new(&from_bob_in_b, 1, "client b's plan"),
    );
    let as_agent = |channel: String| {
        let machine = &talk.ada.machine;
        machine
            .start_in(&folder, &[AGENT, CLAUDE, HISTORY, &channel])
            .finish()
    };

    let own = as_agent(in_a.1.to_string());

    assert!(own.success, "{own:?}");
    assert_history(&own, in_a.1, &[said_in_a], &[]);
    let mark = cloud.relay.mark();

    let refused = as_agent(in_b.1.to_string());

    assert!(!refused.success, "{refused:?}");
    let space = &talk.space;
    let names = [
        CLIENT_B.to_owned(),
        space.id.to_string(),
        space.default.to_string(),
        said_in_b.text.clone(),
    ];
    assert_names_none(&refused, &names);
    let history_of_b = format!("/channels/{}/history", in_b.1);
    let asked: Vec<_> = (cloud.relay.sent_since(mark).into_iter())
        .filter(|sent| sent.head.contains(&history_of_b))
        .collect();
    assert!(asked.is_empty(), "clientb's history requested: {asked:?}");

    let person = shown_history(&talk.ada.machine, in_b.1);

    assert_history(&person, in_b.1, &[said_in_b], &[]);
}
