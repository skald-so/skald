//! A send counts only the messages handed to the agent in its own space: what the agent of the same
//! name took in another space does not count.

use std::path::PathBuf;

use crate::agents::{CLAUDE, run_as_in};
use crate::fixtures::{CHANNEL, SEND, TO};
use crate::helped::with_client_b;
use crate::kit::bind_client_a;
use crate::lines::says_sent;
use crate::loops::assert_stopped;
use crate::machine::Machine;
use crate::replying::{
    BOB_CLAUDE, Writes, agent_of, assert_bob_never_reads, bob_reads, bob_writes, handles, run_in,
    the_one,
};
use crate::talk::Talk;
use crate::world::{ADD_RULE, CLIENT_A, CLIENT_B, dir_of, path, run};

/// What Bob's claude says in clientb, and what Ada's claude sends from each folder; none is
/// anywhere else.
const IN_B: &str = "hop limit space test: bob's claude asks clientb's claude with 10 hops";
const FROM_A: &str = "hop limit space test: claude in clienta sends bob's claude a note";
const FROM_B: &str = "hop limit space test: claude in clientb would answer at hop 11";

/// Ada's machine follows `clienta` and `clientb`, each with a channel that holds Bob's device;
/// `~/work/clientA` is bound to `clienta`, `~/work/clientB` to `clientb`, and claude has run in
/// both. Bob's claude says something to Ada's claude in clientb's channel with `hops` 10, and
/// claude's `skald recv` in `~/work/clientB` takes it. As claude in `~/work/clientA`,
/// `skald send --to` Bob's claude in clienta's channel is sent with `hops` 0; as claude in
/// `~/work/clientB`, `skald send --to` Bob's claude in clientb's channel fails, says why, and is
/// not sent.
#[test]
fn messages_handed_over_in_another_space_do_not_count() {
    let mut talk = Talk::in_space(
        CLIENT_A,
        &[4_811_008_001, 4_811_008_002],
        &["ada-counts-per-space", "bob-loops-in-clientb"],
        false,
    );
    let (other, elsewhere) = with_client_b(&mut talk);
    let in_a = bind_client_a(&talk.ada.machine);
    let in_b = bound_to_client_b(&talk.ada.machine);
    let (bob, _) = handles(&talk);
    let ada = talk.ada.handle.to_string();
    let to_claude = Writes::new(
        &agent_of(&bob, CLIENT_B, BOB_CLAUDE),
        Some(&agent_of(&ada, CLIENT_B, CLAUDE)),
        1,
        IN_B,
    );
    run_as_in(&talk.ada.machine, &in_a, CLAUDE);
    run_as_in(&talk.ada.machine, &in_b, CLAUDE);
    bob_writes(&mut talk, (other.id, elsewhere), &to_claude.hops(10));
    let taken = run_as_in(&talk.ada.machine, &in_b, CLAUDE).stdout;
    assert!(taken.contains(IN_B), "{IN_B:?} not handed over: {taken}");
    let to = format!("{bob}/{BOB_CLAUDE}");
    let machine = &talk.ada.machine;

    let in_clienta = talk.channel.to_string();
    let sent = run_in(
        machine,
        (&in_a, Some(CLAUDE)),
        SEND,
        &[TO, &to, CHANNEL, &in_clienta, FROM_A],
    );

    assert!(sent.success && says_sent(&sent.stdout), "{sent:?}");
    let channel = talk.channel;
    let (_, content) = the_one(&bob_reads(&mut talk, channel), FROM_A);
    assert_eq!(content.hops, 0, "{content:?}");

    let machine = &talk.ada.machine;
    let in_clientb = elsewhere.to_string();
    let refused = run_in(
        machine,
        (&in_b, Some(CLAUDE)),
        SEND,
        &[TO, &to, CHANNEL, &in_clientb, FROM_B],
    );

    assert_stopped(&refused.into(), "the send at hop 11 in clientb");
    let channels = [elsewhere, other.default, talk.channel, talk.space.default];
    assert_bob_never_reads(&mut talk, &channels, &[FROM_B]);
}

/// Binds `~/work/clientB` on `machine` to `clientb`; fails unless the binding succeeds. Gives the
/// folder.
#[track_caller]
fn bound_to_client_b(machine: &Machine) -> PathBuf {
    let folder = dir_of(machine, "work/clientB");
    let bound = run(machine, &[ADD_RULE, path(&folder), CLIENT_B]);
    assert!(bound.success, "not bound: {bound:?}");
    folder
}
