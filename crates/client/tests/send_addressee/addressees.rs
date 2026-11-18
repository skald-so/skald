//! Who a message is for: `--to` one agent, the content's `to`, or `--all`, every agent in the
//! channel, with no `to`; the short and the uppercase forms of an address name the full one.

use crate::clients::{ADA_AGENT, ALL, BOB_CLAUDE, BOB_CODEX, CHANNEL, Clients, TO, assert_one};
use crate::talk::messages_sent;
use crate::world::{CLIENT_A, CLIENT_B};

/// The texts of these tests; none is anywhere else.
const TO_CLAUDE: &str = "for bob's claude: rerun the flaky suite";
const TO_EVERYONE: &str = "the staging database restarts at noon";
const SHORT: &str = "for bob's claude, by the short form: tag the release";
const UPPER: &str = "for bob's claude, in uppercase: rotate the logs";
const SHORT_IN_B: &str = "for bob's codex in client b: merge the hotfix";

/// As Ada's agent in `~/work/clientA`, `skald send --to` Bob's claude in `clienta`, then `--all`,
/// in the talk's channel. Each succeeds; the front saw two message envelopes, both in that
/// channel and none in another. Bob's device reads both there: the first with `to` Bob's claude in
/// `clienta`, not his codex nor his claude in `clientb`; the second with no `to`, for both of his
/// agents in the channel. It reads nothing in another channel, so his agents in `clientb` get
/// neither.
#[test]
fn to_names_one_agent_and_all_every_agent_in_the_channel() {
    let mut clients = Clients::new(
        [4_151_001_001, 4_151_001_002],
        ["ada-names-an-addressee", "bob-two-agents"],
    );
    let channel = clients.talk.channel.to_string();
    let claude = clients.bob_agent(CLIENT_A, BOB_CLAUDE);
    let from = clients.mark();

    let to_one = clients.agent_sends_in(
        &clients.folder_a,
        ADA_AGENT,
        &[TO, &claude, CHANNEL, &channel, TO_CLAUDE],
    );
    let to_all = clients.agent_sends_in(
        &clients.folder_a,
        ADA_AGENT,
        &[ALL, CHANNEL, &channel, TO_EVERYONE],
    );

    assert!(to_one.success, "--to: {to_one:?}");
    assert!(to_all.success, "--all: {to_all:?}");
    let front = &clients.talk.front;
    assert_eq!(
        messages_sent(front, clients.talk.channel, from).len(),
        2,
        "not two messages in the channel"
    );
    for other in clients.channels() {
        if other != clients.talk.channel {
            let sent = messages_sent(front, other, from);
            assert!(sent.is_empty(), "a message in {other}: {sent:?}");
        }
    }
    let read = clients.bob_reads(clients.talk.channel);
    assert_eq!(read.len(), 2, "{read:?}");
    let one = assert_one(&read, TO_CLAUDE, Some(&claude));
    for not in [
        clients.bob_agent(CLIENT_A, BOB_CODEX),
        clients.bob_agent(CLIENT_B, BOB_CLAUDE),
    ] {
        assert_ne!(one.to.as_deref(), Some(not.as_str()), "{one:?}");
    }
    assert_one(&read, TO_EVERYONE, None);
    for other in clients.channels() {
        if other != clients.talk.channel {
            let read = clients.bob_reads(other);
            assert!(read.is_empty(), "read in {other}: {read:?}");
        }
    }
}

/// As the person, `skald send --to bob/<claude>` and `--to` the full address in uppercase, in the
/// talk's channel of `clienta`, and `--to bob/<codex>` in `clientb`'s channel. Each succeeds, and
/// Bob's device reads each in its channel with `to` the full lowercase address: the short form
/// completed with the channel's space and the server's name.
#[test]
fn short_and_uppercase_addresses_name_the_full_address_in_the_channel_s_space() {
    let mut clients = Clients::new(
        [4_151_002_001, 4_151_002_002],
        ["ada-short-addresses", "bob-short-addressed"],
    );
    let (a_channel, b_channel) = (clients.talk.channel, clients.b_channel);
    let short = format!("{}/{BOB_CLAUDE}", clients.bob());
    let claude = clients.bob_agent(CLIENT_A, BOB_CLAUDE);
    let upper = claude.to_uppercase();
    let short_codex = format!("{}/{BOB_CODEX}", clients.bob());
    let (a, b) = (a_channel.to_string(), b_channel.to_string());

    let sent = [
        clients.person_sends(&[TO, &short, CHANNEL, &a, SHORT]),
        clients.person_sends(&[TO, &upper, CHANNEL, &a, UPPER]),
        clients.person_sends(&[TO, &short_codex, CHANNEL, &b, SHORT_IN_B]),
    ];

    for one in &sent {
        assert!(one.success, "{one:?}");
    }
    let in_a = clients.bob_reads(a_channel);
    assert_one(&in_a, SHORT, Some(&claude));
    assert_one(&in_a, UPPER, Some(&claude));
    let in_b = clients.bob_reads(b_channel);
    assert_one(
        &in_b,
        SHORT_IN_B,
        Some(&clients.bob_agent(CLIENT_B, BOB_CODEX)),
    );
}
