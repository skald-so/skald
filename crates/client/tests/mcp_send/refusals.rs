//! Calls of the send tool that name no addressee or one `to` does not take, and a message the
//! server refuses: each says why, and only the last reaches the server.

use serde_json::{Value, json};

use crate::client::Session;
use crate::clients::{BOB_CLAUDE, Clients};
use crate::front::Answers;
use crate::talk::one_message_sent;
use crate::world::{CLIENT_A, CLIENT_B, HOST};
use crate::{
    CHANNEL_FIELD, EVERYONE, SEND_TOOL, TEXT_FIELD, TO_FIELD, assert_bob_read_nothing,
    assert_says_not_sent, session_in,
};

/// The text of every refused call; it is nowhere else.
const REFUSED: &str = "send tool: this text must never leave the machine";

/// The text of the message the server refuses, and the problem it refuses it with.
const SERVER_REFUSED: &str = "send tool: the server refuses this one";
const PROBLEM: &str = "not-a-member";

/// Calls the send tool with each of `cases`, `(what, arguments, words its result says)`, from
/// `session`, then checks that each said its words, in any case, and not `sent #`, and sent no
/// message: the front saw none, so the server stores none, and Bob's device reads none in any
/// channel.
#[track_caller]
fn refused(clients: &mut Clients, session: &mut Session, cases: &[(&str, Value, &[&str])]) {
    for (what, arguments, words) in cases {
        let from = clients.mark();

        let called = session.call(SEND_TOOL, arguments.clone());

        let said = called.text.to_lowercase();
        for word in *words {
            assert!(
                said.contains(&word.to_lowercase()),
                "{what}: the result does not say {word:?}: {called:?}"
            );
        }
        assert_says_not_sent(&called.text, what);
        clients.assert_no_message_sent(from, what);
    }
    assert_bob_read_nothing(clients);
}

/// The arguments of a call `to` `to` with [`REFUSED`] in `channel`.
fn to(to: &str, channel: &str) -> Value {
    json!({ TO_FIELD: to, CHANNEL_FIELD: channel, TEXT_FIELD: REFUSED })
}

/// A session in `~/work/clientA` calls the send tool without `to`, in the talk's channel and with
/// no channel: each result names the field `to`, quoted, and nothing is sent.
#[test]
fn call_without_to_sends_nothing_and_says_why() {
    let mut clients = Clients::new(
        [4_154_003_001, 4_154_003_002],
        ["ada-mcp-no-addressee", "bob-mcp-never-addressed"],
    );
    let channel = clients.talk.channel.to_string();
    let mut session = session_in(&clients, &clients.folder_a);
    let quoted = |called: &str| {
        ["`to`", "\"to\"", "'to'"]
            .iter()
            .any(|q| called.contains(q))
    };

    for (what, arguments) in [
        (
            "in the channel",
            json!({ CHANNEL_FIELD: channel, TEXT_FIELD: REFUSED }),
        ),
        ("no channel", json!({ TEXT_FIELD: REFUSED })),
    ] {
        let from = clients.mark();

        let called = session.call(SEND_TOOL, arguments);

        assert!(
            quoted(&called.text),
            "{what}: does not name `to`: {called:?}"
        );
        assert_says_not_sent(&called.text, what);
        clients.assert_no_message_sent(from, what);
    }
    for channel in clients.channels() {
        let read = clients.bob_reads(channel);
        assert!(read.is_empty(), "read in {channel}: {read:?}");
    }
}

/// A session in `~/work/clientA` calls the send tool `to` a person's address, Bob's or Ada's, full
/// or `owner/user`, in the talk's channel: each result says that only a reply goes to a person, and
/// nothing is sent.
#[test]
fn person_s_address_sends_nothing_and_says_only_a_reply_goes_to_a_person() {
    let mut clients = Clients::new(
        [4_154_004_001, 4_154_004_002],
        ["ada-mcp-to-a-person", "bob-mcp-the-person"],
    );
    let channel = clients.talk.channel.to_string();
    let bob = clients.bob();
    let ada = clients.talk.ada.handle.to_string();
    let mut session = session_in(&clients, &clients.folder_a);
    let addresses = [
        ("bob, full", format!("{CLIENT_A}/{bob}/user@{HOST}")),
        ("bob, short", format!("{bob}/user")),
        ("bob, uppercase", format!("{bob}/USER")),
        ("ada, full", format!("{CLIENT_A}/{ada}/user@{HOST}")),
    ];
    let cases: Vec<(&str, Value, &[&str])> = (addresses.iter())
        .map(|(what, address)| (*what, to(address, &channel), &["person", "reply"][..]))
        .collect();

    refused(&mut clients, &mut session, &cases);
}

/// A session in `~/work/clientA` calls the send tool `to` an invalid address, an empty one, one of
/// another space and one of another server, in the talk's channel: each result says why, and
/// nothing is sent.
#[test]
fn invalid_address_or_one_of_another_space_or_server_sends_nothing_and_says_why() {
    let mut clients = Clients::new(
        [4_154_005_001, 4_154_005_002],
        ["ada-mcp-bad-addresses", "bob-mcp-badly-addressed"],
    );
    let channel = clients.talk.channel.to_string();
    let bob = clients.bob();
    let invalid: &[&str] = &["address"];
    let mut cases = Vec::new();
    for address in [
        String::new(),
        BOB_CLAUDE.to_owned(),
        format!("{bob}/"),
        format!("/{BOB_CLAUDE}"),
        format!("{bob}/claude_r4n8vz"),
        format!("{EVERYONE}/{BOB_CLAUDE}"),
        format!("{CLIENT_A}//{BOB_CLAUDE}@{HOST}"),
        format!("{CLIENT_A}/{bob}/{BOB_CLAUDE}@{HOST}:8443"),
        format!("{CLIENT_A}/{bob}/{BOB_CLAUDE}@"),
        format!("{CLIENT_A}/{bob}/{BOB_CLAUDE} x@{HOST}"),
    ] {
        cases.push(("invalid", to(&address, &channel), invalid));
    }
    cases.push((
        "another space",
        to(&format!("{CLIENT_B}/{bob}/{BOB_CLAUDE}@{HOST}"), &channel),
        &["space"],
    ));
    cases.push((
        "another server",
        to(
            &format!("{CLIENT_A}/{bob}/{BOB_CLAUDE}@elsewhere.example"),
            &channel,
        ),
        &["server"],
    ));
    let mut session = session_in(&clients, &clients.folder_a);

    refused(&mut clients, &mut session, &cases);
}

/// The front refuses each message in place of the server. A session in `~/work/clientA` calls the
/// send tool `*` in the talk's channel: its result is the refusal as the agent's `skald send`
/// prints it, with the message's ID, the channel's ID and the problem, and no `sent #`.
#[test]
fn message_the_server_refuses_gives_the_refusal_as_skald_send_gives_it_to_an_agent() {
    let clients = Clients::new(
        [4_154_006_001, 4_154_006_002],
        ["ada-mcp-refused", "bob-mcp-reads-no-refused"],
    );
    let channel = clients.talk.channel;
    let mut session = session_in(&clients, &clients.folder_a);
    clients
        .talk
        .front
        .answer(Answers::Refuse(PROBLEM.to_owned()));
    let from = clients.mark();

    let called = session.call(
        SEND_TOOL,
        json!({ TO_FIELD: EVERYONE, CHANNEL_FIELD: channel.to_string(), TEXT_FIELD: SERVER_REFUSED }),
    );

    let msg_id = one_message_sent(&clients.talk.front, channel, from);
    for part in [msg_id.to_string(), channel.to_string(), PROBLEM.to_owned()] {
        assert!(called.text.contains(&part), "no {part}: {called:?}");
    }
    assert_says_not_sent(&called.text, "refused by the server");
}
