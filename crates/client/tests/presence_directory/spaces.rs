//! As an agent, the list names nothing of other spaces; the person lists a space by naming it.

use skald_proto::AgentState;

use crate::agents::CLAUDE;
use crate::helped::with_client_b;
use crate::kit::bind_client_a;
use crate::talk::Talk;
use crate::values::{agent_and_tool_list, listed_in, listed_of, person_line_of, status, uploads};
use crate::world::{CLIENT_A, CLIENT_B, HOST, now};

/// Bob's agent in each space; neither name is anywhere else.
const IN_A: &str = "claude-a7c3ne";
const IN_B: &str = "claude-b5z8kt";

/// Ada's machine follows `clienta` and `clientb`, each with a channel that holds Bob's device;
/// `~/work/clientA` is bound to `clienta`. Bob's device lists `claude-a7c3ne` in clienta's channel
/// and `claude-b5z8kt` in clientb's. As claude in `~/work/clientA`, `skald agents` and the tool
/// `agents` list `claude-a7c3ne` and name nothing of clientb: not `claude-b5z8kt`, the space's
/// name or ID, nor its channel's ID. The person's `skald agents clientb` there lists
/// `claude-b5z8kt`.
#[test]
fn agent_lists_nothing_of_another_space_and_the_person_names_one() {
    let mut talk = Talk::in_space(
        CLIENT_A,
        &[4_214_012_001, 4_214_012_002],
        &["ada-lists-within-a-client", "bob-works-for-both"],
        false,
    );
    let (other, elsewhere) = with_client_b(&mut talk);
    let folder = bind_client_a(&talk.ada.machine);
    let bob = talk.played[0].person.handle.to_string();
    let in_a = format!("{CLIENT_A}/{bob}/{IN_A}@{HOST}");
    let in_b = format!("{CLIENT_B}/{bob}/{IN_B}@{HOST}");
    let since = now() - 60;
    let at_a = talk.at();
    for (at, address) in [(at_a, &in_a), ((other.id, elsewhere), &in_b)] {
        let listed = status(address, AgentState::Available, since, (None, None));
        uploads(
            &talk.world.cloud,
            &mut talk.played[0],
            (at, 1),
            vec![listed],
        );
    }
    let machine = &talk.ada.machine;

    let (as_claude, tool) = agent_and_tool_list(machine, &folder, CLAUDE);
    let named = listed_in(machine, &folder, &[], &[CLIENT_B]);

    let short = format!("{bob}/{IN_A}");
    listed_of(&as_claude.stdout, &short);
    listed_of(&tool.text, &short);
    let names = [
        IN_B.to_owned(),
        CLIENT_B.to_owned(),
        other.id.to_string(),
        elsewhere.to_string(),
    ];
    for name in &names {
        for (what, shown) in [
            ("the command", as_claude.shown()),
            ("the tool", tool.text.clone()),
        ] {
            assert!(
                !shown.to_lowercase().contains(&name.to_lowercase()),
                "{what} names {name:?}: {shown}"
            );
        }
    }
    let line = person_line_of(&named, IN_B);
    assert!(line.contains(&in_b), "{line}");
}
