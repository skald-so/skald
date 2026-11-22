//! As an agent, the device replies only to a message handed over to it in its folder's space: one
//! handed to the agent of the same name in another space is unknown, and the refusal names nothing
//! of that space.

use crate::agents::{CLAUDE, run_as_in};
use crate::helped::with_client_b;
use crate::kit::bind_client_a;
use crate::lines::agent_line_of;
use crate::replying::{
    BOB_CLAUDE, REPLY, Writes, agent_of, assert_bob_never_reads, assert_refused, assert_reply,
    bob_reads, bob_writes, handles, run_in, the_one,
};
use crate::talk::Talk;
use crate::world::{ADD_RULE, CLIENT_A, CLIENT_B, dir_of, path, run};

/// What Bob's device says in clientb, and what Ada's claude answers from each folder; none is
/// anywhere else.
const IN_B: &str = "space test: bob's claude asks clientb's claude for the contract";
const FROM_A: &str = "space test: claude in clienta answers clientb's message";
const FROM_B: &str = "space test: claude in clientb answers its own message";

/// Ada's machine follows `clienta` and `clientb`, each with a channel that holds Bob's device;
/// `~/work/clientA` is bound to `clienta`, `~/work/clientB` to `clientb`, and claude has run in
/// both. Bob's device says something in clientb's channel as Bob's claude there, to Ada's claude
/// there; claude's `skald recv` in `~/work/clientB` shows it. As claude in `~/work/clientA`,
/// `skald reply` to it by that reference fails, says why, names nothing of clientb and sends
/// nothing; as claude in `~/work/clientB`, it reaches Bob's device in clientb's channel as a reply.
#[test]
fn agent_replies_only_to_messages_handed_over_to_it_in_its_folder_s_space() {
    let mut talk = Talk::in_space(
        CLIENT_A,
        &[4_656_031_001, 4_656_031_002],
        &["ada-replies-within-a-client", "bob-writes-to-clientb"],
        false,
    );
    let (other, elsewhere) = with_client_b(&mut talk);
    let machine = &talk.ada.machine;
    let in_a = bind_client_a(machine);
    let in_b = dir_of(machine, "work/clientB");
    let bound = run(machine, &[ADD_RULE, path(&in_b), CLIENT_B]);
    assert!(bound.success, "not bound: {bound:?}");
    for folder in [&in_a, &in_b] {
        run_as_in(machine, folder, CLAUDE);
    }
    let (bob, ada) = handles(&talk);
    let (bob_claude, ada_claude) = (
        agent_of(&bob, CLIENT_B, BOB_CLAUDE),
        agent_of(&ada, CLIENT_B, CLAUDE),
    );
    let asked = Writes::new(&bob_claude, Some(&ada_claude), 1, IN_B).hops(1);
    let asked = bob_writes(&mut talk, (other.id, elsewhere), &asked);
    let shown = run_as_in(&talk.ada.machine, &in_b, CLAUDE).shown();
    let short = format!("#{}", agent_line_of(&shown, IN_B).reference);
    let machine = &talk.ada.machine;

    let refused = run_in(machine, (&in_a, Some(CLAUDE)), REPLY, &[&short, FROM_A]);

    let names: [&str; 5] = [
        CLIENT_B,
        IN_B,
        &other.id.to_string(),
        &elsewhere.to_string(),
        &asked.to_string(),
    ];
    assert_refused(&refused, &names, "clientb's message from clienta");
    let channels = [talk.space.default, talk.channel, other.default, elsewhere];
    assert_bob_never_reads(&mut talk, &channels, &[FROM_A]);

    let machine = &talk.ada.machine;
    let replied = run_in(machine, (&in_b, Some(CLAUDE)), REPLY, &[&short, FROM_B]);

    assert!(replied.success, "{replied:?}");
    let read = bob_reads(&mut talk, elsewhere);
    let (_, content) = the_one(&read, FROM_B);
    assert_reply(&content, (&ada_claude, &bob_claude), asked, 2);
}
