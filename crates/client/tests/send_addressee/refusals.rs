//! Sends that name no addressee, two, or one `--to` does not take: each fails, says why and sends
//! nothing.

use crate::clients::{ADA_AGENT, ALL, BOB_CLAUDE, CHANNEL, Clients, TO, assert_refused};
use crate::world::{CLIENT_A, CLIENT_B, HOST};

/// The text of every refused send; it is nowhere else.
const REFUSED: &str = "this text must never leave the machine";

/// Runs each of `cases`, `(what, args before the text, words its refusal says)`, as the person,
/// then checks that each failed, said its words and sent no message: the front saw none, so the
/// server stores none, and Bob's device reads none in any channel.
#[track_caller]
fn refused(clients: &mut Clients, cases: &[(&str, Vec<String>, &[&str])]) {
    for (what, args, words) in cases {
        let from = clients.mark();
        let mut all: Vec<&str> = args.iter().map(String::as_str).collect();
        all.push(REFUSED);

        let finished = clients.person_sends(&all);

        assert_refused(&finished, words, what);
        clients.assert_no_message_sent(from, what);
    }
    clients.assert_bob_read_nothing();
}

/// `args` as owned strings.
fn owned(args: &[&str]) -> Vec<String> {
    args.iter().map(|arg| (*arg).to_owned()).collect()
}

/// `skald send` with neither `--to` nor `--all`, and with both, in the talk's channel: each fails,
/// names both flags and sends nothing.
#[test]
fn neither_or_both_of_to_and_all_sends_nothing_and_names_both() {
    let mut clients = Clients::new(
        [4_151_003_001, 4_151_003_002],
        ["ada-no-addressee", "bob-never-addressed"],
    );
    let channel = clients.talk.channel.to_string();
    let claude = clients.bob_agent(CLIENT_A, BOB_CLAUDE);
    let both: &[&str] = &[TO, ALL];

    refused(
        &mut clients,
        &[
            ("neither", owned(&[CHANNEL, &channel]), both),
            ("both", owned(&[TO, &claude, ALL, CHANNEL, &channel]), both),
            (
                "all, then to",
                owned(&[ALL, TO, &claude, CHANNEL, &channel]),
                both,
            ),
        ],
    );
}

/// `skald send --to` a person's address, Bob's or Ada's, full or `owner/user`, in the talk's
/// channel: each fails, says that only a reply goes to a person, and sends nothing.
#[test]
fn person_s_address_sends_nothing_and_says_only_a_reply_goes_to_a_person() {
    let mut clients = Clients::new(
        [4_151_004_001, 4_151_004_002],
        ["ada-to-a-person", "bob-the-person"],
    );
    let channel = clients.talk.channel.to_string();
    let cases: Vec<(&str, Vec<String>, &[&str])> = (clients.person_addresses().into_iter())
        .map(|(what, address)| {
            let args = owned(&[TO, &address, CHANNEL, &channel]);
            (what, args, &["person", "reply"][..])
        })
        .collect();

    refused(&mut clients, &cases);
}

/// `skald send --to` an invalid address, one of another space and one of another server, in the
/// talk's channel of `clienta`: each fails, says why, and sends nothing. As Ada's agent in
/// `~/work/clientA`, an address in `clientb` fails too and sends nothing.
#[test]
fn invalid_address_or_one_of_another_space_or_server_sends_nothing_and_says_why() {
    let mut clients = Clients::new(
        [4_151_005_001, 4_151_005_002],
        ["ada-bad-addresses", "bob-badly-addressed"],
    );
    let channel = clients.talk.channel.to_string();
    let bob = clients.bob();
    let cases: Vec<(&str, Vec<String>, &[&str])> = (clients.invalid_addresses().into_iter())
        .map(|(what, address, words)| (what, owned(&[TO, &address, CHANNEL, &channel]), words))
        .collect();

    refused(&mut clients, &cases);

    let from = clients.mark();
    let elsewhere = format!("{CLIENT_B}/{bob}/{BOB_CLAUDE}@{HOST}");
    let agent = clients.agent_sends_in(&clients.folder_a, ADA_AGENT, &[TO, &elsewhere, REFUSED]);
    assert!(!agent.success, "as an agent, to clientb: {agent:?}");
    clients.assert_no_message_sent(from, "as an agent, to clientb");
    clients.assert_bob_read_nothing();
}
