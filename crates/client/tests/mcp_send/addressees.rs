//! Who a message of the send tool is for: `to` one agent, the content's `to`, or `*`, every agent
//! in the channel, with no `to`; the short and the uppercase forms of an address name the full one.
//! Each stored message gives `sent #<ref>`.

use serde_json::json;

use crate::clients::{BOB_CLAUDE, Clients, assert_one};
use crate::talk::messages_sent;
use crate::world::CLIENT_A;
use crate::{
    CHANNEL_FIELD, EVERYONE, TEXT_FIELD, TO_FIELD, ada_agent, senders_read, sent_by, session_in,
};

/// The texts of these tests; none is anywhere else.
const TO_CLAUDE: &str = "send tool: for bob's claude, rerun the flaky suite";
const TO_EVERYONE: &str = "send tool: the staging database restarts at noon";
const SHORT: &str = "send tool: for bob's claude by the short form, tag the release";
const UPPER: &str = "send tool: for bob's claude in uppercase, rotate the logs";

/// A session in `~/work/clientA` calls the send tool `to` Bob's claude in `clienta`, then `*`, in
/// the talk's channel. Each result is `sent #<ref>` of the one message the front saw in that
/// channel; the front saw no message in another channel. Bob's device reads both there, from the
/// session's agent: the first with `to` Bob's claude in `clienta` only, so neither his codex nor
/// his claude in `clientb`; the second with no `to`, for every agent in the channel. It reads
/// nothing in another channel, so his agents in `clientb` get neither.
#[test]
fn send_to_one_agent_reaches_it_only_and_to_everyone_reaches_every_agent() {
    let mut clients = Clients::new(
        [4_154_001_001, 4_154_001_002],
        ["ada-mcp-names-one", "bob-mcp-two-agents"],
    );
    let channel = clients.talk.channel;
    let claude = clients.bob_agent(CLIENT_A, BOB_CLAUDE);
    let mut session = session_in(&clients, &clients.folder_a);
    let start = clients.mark();

    let ids = [(claude.as_str(), TO_CLAUDE), (EVERYONE, TO_EVERYONE)].map(|(to, text)| {
        let arguments =
            json!({ TO_FIELD: to, CHANNEL_FIELD: channel.to_string(), TEXT_FIELD: text });
        sent_by(&mut session, &clients, arguments, channel)
    });

    let elsewhere: Vec<_> = (clients.channels().into_iter())
        .filter(|other| *other != channel)
        .flat_map(|other| messages_sent(&clients.talk.front, other, start))
        .collect();
    assert!(elsewhere.is_empty(), "messages elsewhere: {elsewhere:?}");
    let read = clients.bob_reads(channel);
    assert_eq!(read.len(), 2, "{read:?}");
    assert_eq!(assert_one(&read, TO_CLAUDE, Some(&claude)).msg_id, ids[0]);
    assert_eq!(assert_one(&read, TO_EVERYONE, None).msg_id, ids[1]);
    let agent = ada_agent(&clients);
    for sender in senders_read(&mut clients, channel) {
        assert_eq!(sender, agent, "not from the session's agent");
    }
    let others: Vec<_> = (clients.channels().into_iter())
        .filter(|other| *other != channel)
        .flat_map(|other| clients.bob_reads(other))
        .collect();
    assert!(others.is_empty(), "read elsewhere: {others:?}");
}

/// A session in `~/work/clientA` calls the send tool `to` `bob/<claude>` and `to` the full address
/// of Bob's claude in uppercase, in the talk's channel. Each gives `sent #<ref>`, and Bob's device
/// reads each there with `to` the full lowercase address: the short form completed with the
/// channel's space and the server's name.
#[test]
fn short_and_uppercase_addresses_name_the_full_address_in_the_channel_s_space() {
    let mut clients = Clients::new(
        [4_154_002_001, 4_154_002_002],
        ["ada-mcp-short-addresses", "bob-mcp-short-addressed"],
    );
    let channel = clients.talk.channel;
    let claude = clients.bob_agent(CLIENT_A, BOB_CLAUDE);
    let short = format!("{}/{BOB_CLAUDE}", clients.bob());
    let upper = claude.to_uppercase();
    let mut session = session_in(&clients, &clients.folder_a);

    for (to, text) in [(short.as_str(), SHORT), (upper.as_str(), UPPER)] {
        let arguments =
            json!({ TO_FIELD: to, CHANNEL_FIELD: channel.to_string(), TEXT_FIELD: text });
        sent_by(&mut session, &clients, arguments, channel);
    }

    let read = clients.bob_reads(channel);
    assert_one(&read, SHORT, Some(&claude));
    assert_one(&read, UPPER, Some(&claude));
}
