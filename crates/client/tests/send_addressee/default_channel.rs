//! A send that names no channel: to the default channel of the working directory's space, for an
//! agent and for the person; nothing from a folder in no space.

use crate::clients::{ADA_AGENT, BOB_CLAUDE, BOB_CODEX, Clients, assert_one, assert_refused};
use crate::fixtures::{ALL, TO};
use crate::talk::messages_sent;
use crate::world::{CLIENT_A, CLIENT_B};

/// The texts of these tests; none is anywhere else.
const PERSON_IN_A: &str = "the person in client a: standup moves to ten";
const AGENT_IN_A: &str = "the agent in client a: the build is green";
const PERSON_IN_B: &str = "the person in client b: invoices are out";
const AGENT_IN_B: &str = "the agent in client b: review the contract";
const NOWHERE: &str = "from a folder in no space: never sent";

/// Fails unless the front saw, after its first `from` sends, one message in `channel` of `clients`'
/// channels and none in the others.
#[track_caller]
fn assert_sent_only_in(clients: &Clients, channel: skald_proto::ChannelId, from: usize) {
    for one in clients.channels() {
        let sent = messages_sent(&clients.talk.front, one, from);
        let expected = usize::from(one == channel);
        assert_eq!(sent.len(), expected, "messages in {one}: {sent:?}");
    }
}

/// As the person, `skald send --all` with no `--channel` in `~/work/clientA` goes to `clienta`'s
/// default channel, and in `~/work/clientB`, bound to `clientb`, to `clientb`'s: the front sees
/// each in that channel only, and Bob's device reads each there, with no `to`.
#[test]
fn person_s_send_naming_no_channel_goes_to_the_default_channel_of_the_folder_s_space() {
    let mut clients = Clients::new(
        [4_151_006_001, 4_151_006_002],
        ["ada-person-default", "bob-reads-defaults"],
    );
    let folder_b = clients.bound("work/clientB", CLIENT_B);
    let (a_default, b_default) = (clients.talk.space.default, clients.client_b.default);

    let from = clients.mark();
    let in_a = clients.person_sends_in(&clients.folder_a, &[ALL, PERSON_IN_A]);
    assert!(in_a.success, "{in_a:?}");
    assert_sent_only_in(&clients, a_default, from);

    let from = clients.mark();
    let in_b = clients.person_sends_in(&folder_b, &[ALL, PERSON_IN_B]);
    assert!(in_b.success, "{in_b:?}");
    assert_sent_only_in(&clients, b_default, from);

    assert_one(&clients.bob_reads(a_default), PERSON_IN_A, None);
    assert_one(&clients.bob_reads(b_default), PERSON_IN_B, None);
}

/// As Ada's agent, `skald --agent <name> send --to bob/<claude>` with no `--channel` in
/// `~/work/clientA` goes to `clienta`'s default channel, with `to` Bob's claude in `clienta`; in
/// `~/work/clientB`, `--to bob/<codex>` goes to `clientb`'s, with `to` Bob's codex in `clientb`.
#[test]
fn agent_s_send_naming_no_channel_goes_to_the_default_channel_of_its_folder_s_space() {
    let mut clients = Clients::new(
        [4_151_007_001, 4_151_007_002],
        ["ada-agent-default", "bob-agent-defaults"],
    );
    let folder_b = clients.bound("work/clientB", CLIENT_B);
    let (a_default, b_default) = (clients.talk.space.default, clients.client_b.default);
    let claude = format!("{}/{BOB_CLAUDE}", clients.bob());
    let codex = format!("{}/{BOB_CODEX}", clients.bob());

    let from = clients.mark();
    let in_a = clients.agent_sends_in(&clients.folder_a, ADA_AGENT, &[TO, &claude, AGENT_IN_A]);
    assert!(in_a.success, "{in_a:?}");
    assert_sent_only_in(&clients, a_default, from);

    let from = clients.mark();
    let in_b = clients.agent_sends_in(&folder_b, ADA_AGENT, &[TO, &codex, AGENT_IN_B]);
    assert!(in_b.success, "{in_b:?}");
    assert_sent_only_in(&clients, b_default, from);

    let in_a = clients.bob_reads(a_default);
    assert_one(
        &in_a,
        AGENT_IN_A,
        Some(&clients.bob_agent(CLIENT_A, BOB_CLAUDE)),
    );
    let in_b = clients.bob_reads(b_default);
    assert_one(
        &in_b,
        AGENT_IN_B,
        Some(&clients.bob_agent(CLIENT_B, BOB_CODEX)),
    );
}

/// `~/work/clientC` is bound to `clientc`, a space Ada's device is not in, so it is in no space.
/// There `skald send --all` with no `--channel` fails, as the person and as Ada's agent, says the
/// folder is in no space, and sends nothing; Bob's device reads nothing.
#[test]
fn send_naming_no_channel_in_a_folder_in_no_space_sends_nothing_and_says_so() {
    let mut clients = Clients::new(
        [4_151_008_001, 4_151_008_002],
        ["ada-folder-nowhere", "bob-gets-nothing"],
    );
    let nowhere = clients.bound("work/clientC", "clientc");

    for (what, agent) in [("the person", None), ("the agent", Some(ADA_AGENT))] {
        let from = clients.mark();
        let finished = match agent {
            Some(agent) => clients.agent_sends_in(&nowhere, agent, &[ALL, NOWHERE]),
            None => clients.person_sends_in(&nowhere, &[ALL, NOWHERE]),
        };
        assert_refused(&finished, &["no space"], what);
        clients.assert_no_message_sent(from, what);
    }
    clients.assert_bob_read_nothing();
}
