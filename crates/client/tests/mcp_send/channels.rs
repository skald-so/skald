//! The channel of the send tool: by default the default channel of the session's folder's space;
//! one of another space is refused and named nowhere; in a folder in no space nothing is sent.

use serde_json::json;

use crate::clients::{BOB_CLAUDE, BOB_CODEX, Clients, assert_one};
use crate::talk::messages_sent;
use crate::world::{CLIENT_A, CLIENT_B};
use crate::{
    CHANNEL_FIELD, EVERYONE, SEND_TOOL, TEXT_FIELD, TO_FIELD, assert_bob_read_nothing,
    assert_says_not_sent, sent_by, session_in,
};

/// The texts of these tests; none is anywhere else.
const IN_A: &str = "send tool in client a: the build is green";
const IN_B: &str = "send tool in client b: review the contract";
const ACROSS: &str = "send tool: never sent to client b's channel";
const OWN: &str = "send tool: sent to client a's channel after the refusals";
const NOWHERE: &str = "send tool from a folder in no space: never sent";

/// What a session in a folder in no space says.
const NO_SPACE: &str = "no space";

/// Fails unless the front saw no message in any of `clients`' channels after its first `from`
/// sends, but `but` of them.
#[track_caller]
fn none_sent_but(clients: &Clients, from: usize, but: usize) {
    let all: usize = (clients.channels().into_iter())
        .map(|channel| messages_sent(&clients.talk.front, channel, from).len())
        .sum();
    assert_eq!(all, but, "messages in the channels of both spaces");
}

/// `~/work/clientB` is bound to `clientb`. Sessions start in `~/work/clientA` and there. The one
/// in `~/work/clientA` calls the send tool `to` `bob/<claude>` with no channel: the message goes to
/// `clienta`'s default channel and no other, with `to` Bob's claude in `clienta`, and the result is
/// its `sent #<ref>`. The one in `~/work/clientB` does the same `to` `bob/<codex>`: to `clientb`'s
/// default channel and no other, with `to` Bob's codex in `clientb`.
#[test]
fn send_naming_no_channel_goes_to_the_default_channel_of_the_folder_s_space() {
    let mut clients = Clients::new(
        [4_154_007_001, 4_154_007_002],
        ["ada-mcp-default-channel", "bob-mcp-reads-defaults"],
    );
    let in_b = clients.bound("work/clientB", CLIENT_B);
    let bob = clients.bob();
    let mut sessions = [
        session_in(&clients, &clients.folder_a),
        session_in(&clients, &in_b),
    ];
    let cases = [
        (CLIENT_A, clients.talk.space.default, BOB_CLAUDE, IN_A),
        (CLIENT_B, clients.client_b.default, BOB_CODEX, IN_B),
    ];

    for (session, (_, default, agent, text)) in sessions.iter_mut().zip(cases) {
        let from = clients.mark();
        let to = format!("{bob}/{agent}");
        sent_by(
            session,
            &clients,
            json!({ TO_FIELD: to, TEXT_FIELD: text }),
            default,
        );
        none_sent_but(&clients, from, 1);
    }

    for (space, default, agent, text) in cases {
        let read = clients.bob_reads(default);
        assert_one(&read, text, Some(&clients.bob_agent(space, agent)));
    }
}

/// A session in `~/work/clientA` calls the send tool `*` in `clientb`'s channel, then in its
/// default one: each is refused, says not `sent #`, sends nothing, and its result names nothing of
/// `clientb`: not its name, its ID nor its channels' IDs. Then the same call in the talk's channel
/// of `clienta` gives `sent #<ref>`. Bob's device reads only that message, in the talk's channel.
#[test]
fn channel_of_another_space_is_refused_and_named_nowhere() {
    let mut clients = Clients::new(
        [4_154_008_001, 4_154_008_002],
        ["ada-mcp-keeps-clients-apart", "bob-mcp-in-both-clients"],
    );
    let b_channels = [clients.b_channel, clients.client_b.default];
    let of_b = [
        CLIENT_B.to_owned(),
        clients.client_b.id.to_string().to_lowercase(),
        b_channels[0].to_string().to_lowercase(),
        b_channels[1].to_string().to_lowercase(),
    ];
    let mut session = session_in(&clients, &clients.folder_a);

    for channel in b_channels {
        let what = format!("in {channel}");
        let from = clients.mark();

        let called = session.call(
            SEND_TOOL,
            json!({ TO_FIELD: EVERYONE, CHANNEL_FIELD: channel.to_string(), TEXT_FIELD: ACROSS }),
        );

        assert_says_not_sent(&called.text, &what);
        clients.assert_no_message_sent(from, &what);
        let said = called.text.to_lowercase();
        for part in &of_b {
            assert!(!said.contains(part), "{what}: names {part}: {called:?}");
        }
    }
    let own = clients.talk.channel;
    let arguments = json!({ TO_FIELD: EVERYONE, CHANNEL_FIELD: own.to_string(), TEXT_FIELD: OWN });
    sent_by(&mut session, &clients, arguments, own);

    let read = clients.bob_reads(own);
    assert_eq!(read.len(), 1, "{read:?}");
    assert_one(&read, OWN, None);
    for channel in b_channels {
        let read = clients.bob_reads(channel);
        assert!(read.is_empty(), "read in {channel}: {read:?}");
    }
}

/// `~/work/clientC` is bound to `clientc`, a space Ada's device is not in, so it is in no space. A
/// session there calls the send tool `*` with no channel, then `to` Bob's claude in `clienta`: each
/// result says the agent sees no space, not `sent #`, and nothing is sent.
#[test]
fn send_in_a_folder_in_no_space_sends_nothing_and_says_so() {
    let mut clients = Clients::new(
        [4_154_009_001, 4_154_009_002],
        ["ada-mcp-folder-nowhere", "bob-mcp-gets-nothing"],
    );
    let nowhere = clients.bound("work/clientC", "clientc");
    let claude = clients.bob_agent(CLIENT_A, BOB_CLAUDE);
    let mut session = session_in(&clients, &nowhere);

    for to in [EVERYONE, claude.as_str()] {
        let what = format!("to {to}");
        let from = clients.mark();

        let called = session.call(SEND_TOOL, json!({ TO_FIELD: to, TEXT_FIELD: NOWHERE }));

        assert!(
            called.text.to_lowercase().contains(NO_SPACE),
            "{what}: does not say it sees no space: {called:?}"
        );
        assert_says_not_sent(&called.text, &what);
        clients.assert_no_message_sent(from, &what);
    }
    assert_bob_read_nothing(&mut clients);
}
